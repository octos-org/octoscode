//! `octoscode olp board archive <board.md>` — move the text of closed
//! entries to an archive file and leave a numbered stub on the board.
//!
//! An entry is a `### <number>` heading up to the next `###`/`##` heading
//! outside code fences. Only cards the inbox calls settled are archived,
//! and never the most recent `--keep` (default 10). The stub keeps the
//! heading and every line tools read by machine — `ACK(` lines, `unit:` /
//! `impl:` / `review:` / `verify-of:`, key marks, signed verdicts, push
//! records, signed inner ACKs, unit alerts and forwarded MCP audit lines —
//! so numbering, the inbox, the local-PR status and the hub's push hook
//! read the board as before.
//!
//! The archive is written first: after a failure an entry is in both
//! places, never in neither. The board is replaced atomically, under the
//! lock `board append` takes; an in-place edit by an inner loop (which
//! takes no lock) is detected and the replace is given up.
//!
//! Exit codes: 0 (also: nothing to archive); 2 usage or no such board;
//! 4 the board changed while archiving; 6 the outer-duty lease is invalid.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::board::{self, ARCHIVED_MARK};
use super::inbox::{Partition, classify};

const PROG: &str = "olp-board-archive";
const USAGE: &str =
    "用法: octoscode olp board archive <board.md> [--archive <file>] [--keep N] [--dry-run]

把已闭环条目的正文移到归档文件,板上留编号桩。
  --archive <file>  归档文件(默认 <板名>.archive.md,与板同目录)
  --keep N          原样保留最近 N 个编号条目(默认 10)
  --dry-run         只列出会归档的条目,不写任何文件
退出码:0 成功(含无可归档);2 用法错误或板不存在;4 归档期间板被改动;6 主审权已失效";

fn stat_key(path: &Path) -> Option<(u64, u64, std::time::SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    #[cfg(unix)]
    let inode = std::os::unix::fs::MetadataExt::ino(&meta);
    #[cfg(not(unix))]
    let inode = 0;
    Some((inode, meta.len(), meta.modified().ok()?))
}

fn with_newline(line: &str) -> String {
    if line.ends_with('\n') {
        line.to_string()
    } else {
        format!("{line}\n")
    }
}

fn bare(line: &str) -> &str {
    line.trim_end_matches(['\r', '\n'])
}

/// `<!-- olp-dispatch: n=<N> sha256=<64 hex> pane=<-|pane> -->`: its number.
fn dispatch_mark(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("<!-- olp-dispatch: n=")?;
    let (n, rest) = rest.split_once(" sha256=")?;
    let (sha, rest) = rest.split_once(" pane=")?;
    let pane = rest.strip_suffix(" -->")?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let hex = sha.len() == 64 && sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    (digits(n) && hex && (pane == "-" || super::card::is_pane(pane))).then_some(n)
}

/// `<!-- olp-key: <key> n=<N> -->`: its number.
fn numbered_key(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("<!-- olp-key: ")?.strip_suffix(" -->")?;
    let (key, n) = rest.split_once(" n=")?;
    let key_ok = !key.is_empty()
        && key.len() <= 200
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:/@+=-".contains(&b));
    (key_ok && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())).then_some(n)
}

fn role_line(line: &str) -> bool {
    ["unit:", "impl:", "review:", "verify-of:"]
        .iter()
        .any(|key| {
            line.strip_prefix(key)
                .is_some_and(|value| !value.trim().is_empty())
        })
}

/// A unit's alert, or an MCP blocked/timeout line a unit forwarded: the
/// signals the evolution harvest reads on the live board.
fn unit_signal(signed: &str) -> bool {
    let Some(rest) = signed.strip_prefix("单元(") else {
        return false;
    };
    let Some((name, rest)) = rest.split_once(")·") else {
        return false;
    };
    if name.is_empty() || name.contains(')') {
        return false;
    }
    if rest.starts_with("负哨(") {
        return true;
    }
    let words: Vec<&str> = rest.split_whitespace().collect();
    rest.starts_with('-')
        && words.len() >= 5
        && words[0] == "-"
        && words[3] == "MCP(ask_outer)"
        && (words[4].starts_with("blocked:") || words[4].starts_with("timeout:"))
}

/// The lines of an entry its stub keeps, outside code fences, in order.
fn kept_lines<'a>(entry: &[&'a str]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let numbered = entry.iter().enumerate().map(|(i, l)| (i + 1, bare(l)));
    for (no, line) in board::markdown_lines(numbered) {
        // A dispatch binding is kept only with the key line right under it.
        if let Some(n) = dispatch_mark(line) {
            if entry.get(no).map(|next| numbered_key(bare(next))) == Some(Some(n)) {
                out.push(entry[no - 1]);
            }
        }
        let signed = board::strip_quote(line);
        if line.starts_with("ACK(")
            || line.starts_with("<!-- olp-key: ")
            || role_line(line)
            || board::is_signed_form(signed)
            || unit_signal(signed)
        {
            out.push(entry[no - 1]);
        }
    }
    out
}

struct Args {
    board: PathBuf,
    archive: Option<PathBuf>,
    keep: usize,
    dry_run: bool,
}

fn parse(argv: &[String]) -> Result<Option<Args>, String> {
    let (mut board, mut archive, mut keep, mut dry_run) = (None, None, 10usize, false);
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        let (name, inline) = match word.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (word.as_str(), None),
        };
        let mut value = || {
            inline
                .clone()
                .or_else(|| words.next().cloned())
                .ok_or_else(|| format!("{name} 需要一个值"))
        };
        match name {
            "-h" | "--help" => return Ok(None),
            "--dry-run" => dry_run = true,
            "--archive" => archive = Some(PathBuf::from(value()?)),
            "--keep" => {
                let text = value()?;
                keep = match text.parse::<i64>() {
                    Ok(n) if n >= 0 => n as usize,
                    Ok(_) => return Err("--keep 不能为负数".to_string()),
                    Err(_) => return Err(format!("--keep 不是整数: {text}")),
                };
            }
            other if other.starts_with("--") => return Err(format!("不认识的参数: {other}")),
            other if board.is_none() => board = Some(PathBuf::from(other)),
            other => return Err(format!("多余的参数: {other}")),
        }
    }
    let board = board.ok_or("缺少黑板文件")?;
    Ok(Some(Args {
        board,
        archive,
        keep,
        dry_run,
    }))
}

/// `argv` is what follows `olp board archive`.
pub fn run(argv: &[String]) -> i32 {
    let args = match parse(argv) {
        Ok(Some(args)) => args,
        Ok(None) => {
            println!("{USAGE}");
            return 0;
        }
        Err(why) => {
            eprintln!("{PROG}: {why}\n{USAGE}");
            return 2;
        }
    };
    let board_path = &args.board;
    if !board_path.is_file() {
        eprintln!("{PROG}: 板不存在: {}", board_path.display());
        return 2;
    }
    let absolute = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let board_dir = absolute(board_path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let board_name = board_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let archive = args.archive.clone().unwrap_or_else(|| {
        let stem = board_name.strip_suffix(".md").unwrap_or(&board_name);
        board_dir.join(format!("{stem}.archive.md"))
    });
    if absolute(&archive) == absolute(board_path) {
        eprintln!("{PROG}: 归档文件不能是黑板本身");
        return 2;
    }
    let archive_name = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut _lock = None;
    if !args.dry_run {
        if let Err(code) = super::duty::gate(PROG) {
            return code;
        }
        let mut name = board_path.as_os_str().to_owned();
        name.push(".lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(PathBuf::from(name))
            .and_then(|file| fs2::FileExt::lock_exclusive(&file).map(|()| file));
        match file {
            Ok(file) => _lock = Some(file),
            Err(e) => {
                eprintln!("{PROG}: 拿不到板的锁: {e}");
                return 2;
            }
        }
    }
    let before = stat_key(board_path);
    let text = match std::fs::read(board_path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(e) => {
            eprintln!("{PROG}: 读不了板: {e}");
            return 2;
        }
    };
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    // Blocks start at unfenced headings; each keeps its first line's number.
    let mut starts = vec![0usize];
    starts.extend(
        board::markdown_lines(board::lines(&text))
            .into_iter()
            .filter(|(no, line)| (line.starts_with("### ") || line.starts_with("## ")) && *no > 1)
            .map(|(no, _)| no - 1),
    );
    starts.push(lines.len());
    let blocks: Vec<(usize, &[&str])> = starts
        .windows(2)
        .filter(|w| w[0] < w[1])
        .map(|w| (w[0] + 1, &lines[w[0]..w[1]]))
        .collect();

    let mut scan = board::scan(&text);
    let cards: Vec<(usize, u64)> = blocks
        .iter()
        .enumerate()
        .filter_map(|(i, (line, _))| {
            let (number, _) = scan.entries.iter().find(|(_, e)| e.line == *line)?;
            Some((i, *number))
        })
        .collect();
    let eligible = &cards[..cards.len().saturating_sub(args.keep)];
    let today = chrono::Local::now().date_naive();
    let mut targets = Vec::new();
    for (i, number) in eligible {
        let entry = &scan.entries[number];
        let events = scan.events.get(number).map(Vec::as_slice).unwrap_or(&[]);
        let state = classify(*number, entry, events, today, &mut scan.warnings);
        if state.partition == Partition::Settled && entry.archive_line.is_none() {
            targets.push((*i, *number));
        }
    }
    if targets.is_empty() {
        println!("{PROG}: 没有可归档的条目");
        return 0;
    }
    let title = |i: usize| blocks[i].1[0].trim().chars().skip(4).collect::<String>();
    if args.dry_run {
        for (i, number) in &targets {
            println!("would archive #{number} {}", title(*i));
        }
        println!(
            "{PROG}: dry-run,{} 个条目可归档,未写入任何文件",
            targets.len()
        );
        return 0;
    }

    let already: BTreeSet<u64> = std::fs::read_to_string(&archive)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.strip_prefix("<!-- olp-archived-entry: "))
        .filter_map(|rest| rest.split_once(' ')?.0.parse().ok())
        .collect();
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let mut to_append = String::new();
    let mut stubs: Vec<Option<String>> = vec![None; blocks.len()];
    for (i, number) in &targets {
        let entry = blocks[*i].1;
        if !already.contains(number) {
            to_append.push_str(&format!("<!-- olp-archived-entry: {number} at {now} -->\n"));
            for line in entry {
                to_append.push_str(&with_newline(line));
            }
            if entry.last().is_some_and(|l| !l.trim().is_empty()) {
                to_append.push('\n');
            }
        }
        let mut stub = with_newline(entry[0]);
        for line in kept_lines(entry) {
            stub.push_str(&with_newline(line));
        }
        stub.push_str(&format!("{ARCHIVED_MARK}{archive_name} -->\n\n"));
        stubs[*i] = Some(stub);
    }

    // Archive first: if anything fails after this, the entries exist in
    // both places, never in neither.
    if !to_append.is_empty() {
        let old = std::fs::read(&archive).unwrap_or_default();
        let mut head = String::new();
        if old.is_empty() {
            head = format!(
                "# 黑板归档({board_name})\n\n> 由 olp-board-archive.py 追加;已闭环条目的完整原文,只供审计,不可执行。\n\n"
            );
        } else if old.last() != Some(&b'\n') {
            head.push('\n');
        }
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&archive)
            .and_then(|mut file| {
                file.write_all(head.as_bytes())?;
                file.write_all(to_append.as_bytes())?;
                file.sync_all()
            });
        if let Err(e) = written {
            eprintln!("{PROG}: 写不了归档文件 {}: {e}", archive.display());
            return 2;
        }
    }

    let mut new_board = String::new();
    for (i, (_, block)) in blocks.iter().enumerate() {
        match &stubs[i] {
            Some(stub) => new_board.push_str(stub),
            None => block.iter().for_each(|line| new_board.push_str(line)),
        }
    }
    let staged = tempfile::Builder::new()
        .prefix(".olp-archive-")
        .tempfile_in(&board_dir)
        .and_then(|mut tmp| {
            tmp.write_all(new_board.as_bytes())?;
            tmp.as_file().sync_all()?;
            if let Ok(meta) = std::fs::metadata(board_path) {
                std::fs::set_permissions(tmp.path(), meta.permissions())?;
            }
            Ok(tmp)
        });
    let tmp = match staged {
        Ok(tmp) => tmp,
        Err(e) => {
            eprintln!("{PROG}: 写不了临时文件: {e}");
            return 2;
        }
    };
    // Test-only: an unlocked writer (an inner loop's in-place ACK) landing
    // between the read and the replace.
    if let Some(injected) =
        std::env::var_os("OLP_BOARD_ARCHIVE_TEST_CONCURRENT_EDIT").filter(|v| !v.is_empty())
    {
        if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(board_path) {
            let _ = file.write_all(injected.to_string_lossy().as_bytes());
        }
    }
    if stat_key(board_path) != before {
        eprintln!(
            "{PROG}: 归档期间板被改动,未替换黑板。归档文件里已写入的条目下次重跑时会跳过;请在内环空闲时重跑。"
        );
        return 4;
    }
    if let Err(e) = tmp.persist(board_path) {
        eprintln!("{PROG}: 换不了黑板: {}", e.error);
        return 2;
    }
    for (i, number) in &targets {
        println!("archived #{number} {}", title(*i));
    }
    println!(
        "{PROG}: {} 个条目已归档到 {}",
        targets.len(),
        archive.display()
    );
    eprintln!("{PROG}: 板的行数已变少;一击观测哨请重新挂。");
    0
}
