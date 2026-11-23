//! `octoscode olp board append` and `octoscode olp id alloc` — the two
//! writes that hand out numbers, each under an exclusive lock on
//! `<file>.lock`.
//!
//! The lock is the one `flock(1)` takes, so these and the scripts they
//! replace (`olp-board-append.sh`, `olp-id-alloc.sh`, still installed as
//! single files by `olp init`) exclude each other; this side needs no
//! `flock` program.
//!
//! `board append [--key K] <board>` (entry on stdin): a first line starting
//! `### {N}` gets the next number, every `{N}` in the entry is replaced and
//! the number is printed; `--key` makes a resend a no-op that prints the
//! original number; anything else is appended as it is.
//! `id alloc <registry.tsv> <prefix> <width> <key>`: the same key gets the
//! same id again; each prefix counts on its own.
//!
//! Exit codes: 0 written (or already there); 2 usage, nothing touched;
//! 6 the outer-duty lease is no longer valid (`board append`).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

fn die(prog: &str, why: &str) -> i32 {
    eprintln!("{prog}: {why}");
    2
}

/// `<path>.lock`, held exclusively until the file is dropped.
fn lock(path: &Path) -> std::io::Result<std::fs::File> {
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(PathBuf::from(name))?;
    fs2::FileExt::lock_exclusive(&file)?;
    Ok(file)
}

fn append_to(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn replace(haystack: &[u8], needle: &[u8], with: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(haystack.len());
    let mut rest = haystack;
    while let Some(at) = rest.windows(needle.len()).position(|w| w == needle) {
        out.extend_from_slice(&rest[..at]);
        out.extend_from_slice(with);
        rest = &rest[at + needle.len()..];
    }
    out.extend_from_slice(rest);
    out
}

/// The number a board already records for `key`: `Some(None)` when the key
/// is there without a number.
fn key_hit(board: &[u8], key: &str) -> Option<Option<u64>> {
    let bare = format!("<!-- olp-key: {key} -->");
    let numbered = format!("<!-- olp-key: {key} n=");
    for line in board.split(|b| *b == b'\n') {
        if line == bare.as_bytes() {
            return Some(None);
        }
        if let Some(rest) = line.strip_prefix(numbered.as_bytes()) {
            let digits = rest.strip_suffix(b" -->").unwrap_or(rest);
            if let Some(n) = std::str::from_utf8(digits)
                .ok()
                .filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|d| d.parse().ok())
            {
                return Some(Some(n));
            }
        }
    }
    None
}

/// The largest `### <number>` heading on the board.
fn max_heading(board: &[u8]) -> u64 {
    board
        .split(|b| *b == b'\n')
        .filter_map(|line| line.strip_prefix(b"### "))
        .filter_map(|rest| {
            let digits: Vec<u8> = rest
                .iter()
                .copied()
                .take_while(u8::is_ascii_digit)
                .collect();
            String::from_utf8(digits).ok()?.parse::<u64>().ok()
        })
        .max()
        .unwrap_or(0)
}

/// `argv` is what follows `olp board append`.
pub fn board_append(argv: &[String]) -> i32 {
    let prog = "olp-board-append";
    let usage = "用法: octoscode olp board append [--key <自然键>] <board.md> (正文从 stdin 喂)";
    let mut key: Option<&str> = None;
    let mut rest: &[String] = argv;
    loop {
        match rest {
            [flag, ..] if flag == "-h" || flag == "--help" => {
                println!(
                    "{usage}\n\n首行以 `### {{N}}` 起头:锁内分配编号,替换正文里所有 {{N}},stdout 输出编号。\n--key:同一个键已在板上时不再追加,输出原编号。其余原样追加。\n退出码:0 成功(含幂等命中);2 用法,未触碰板;6 主审权已失效"
                );
                return 0;
            }
            [flag, value, tail @ ..] if flag == "--key" => {
                key = Some(value);
                rest = tail;
            }
            [flag] if flag == "--key" => return die(prog, "--key 需要一个值"),
            [flag, tail @ ..] if flag == "--" => {
                rest = tail;
                break;
            }
            [flag, ..] if flag.starts_with('-') => return die(prog, &format!("未知参数: {flag}")),
            _ => break,
        }
    }
    let board = match rest {
        [board] => Path::new(board),
        [] => return die(prog, usage),
        [_, extra, ..] => return die(prog, &format!("多余的参数: {extra}")),
    };
    if let Some(key) = key {
        if key.is_empty() {
            return die(prog, "--key 不能为空");
        }
        if key.len() > 200 {
            return die(prog, "--key 超过 200 字节");
        }
        if !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:/@+=-".contains(&b))
        {
            return die(prog, "--key 只允许 [A-Za-z0-9._:/@+=-]");
        }
    }
    let mut body = Vec::new();
    if std::io::stdin().read_to_end(&mut body).is_err() {
        return die(prog, "读不了 stdin");
    }
    write_entry(board, key, body)
}

/// Append `body` to `board` under its lock: number it when its first line
/// starts `### {N}`, skip it when `key` is already on the board. Prints the
/// number (when there is one) to stdout. Returns the exit code.
pub fn write_entry(board: &Path, key: Option<&str>, body: Vec<u8>) -> i32 {
    let prog = "olp-board-append";
    if body.is_empty() {
        return if key.is_some() {
            die(prog, "正文为空")
        } else {
            0
        };
    }
    // Only an entry whose first line starts `### {N}` is numbered: `{N}`
    // elsewhere (quoted text, say) is not a placeholder.
    let numbered = body.starts_with(b"### {N}");

    let _lock = match lock(board) {
        Ok(lock) => lock,
        Err(e) => return die(prog, &format!("拿不到板的锁: {e}")),
    };
    // A command left behind by an outer session that ended or was replaced
    // must not write: checked inside the lock, before the write.
    if let Err(code) = super::duty::gate(prog) {
        return code;
    }
    let existing = std::fs::read(board).unwrap_or_default();
    if let Some(hit) = key.and_then(|k| key_hit(&existing, k)) {
        let key = key.unwrap_or_default();
        match hit {
            None => eprintln!("{prog}: 键 {key} 已在板上,未重复追加"),
            Some(n) => {
                eprintln!("{prog}: 键 {key} 已在板上(#{n}),未重复追加");
                println!("{n}");
            }
        }
        return 0;
    }
    let number = numbered.then(|| max_heading(&existing) + 1);
    let mut entry = match number {
        Some(n) => replace(&body, b"{N}", n.to_string().as_bytes()),
        None => body,
    };
    // The entry ends with a newline, so a key mark has its own line and the
    // next heading does not run on.
    if entry.last() != Some(&b'\n') {
        entry.push(b'\n');
    }
    if let Some(key) = key {
        let mark = match number {
            Some(n) => format!("<!-- olp-key: {key} n={n} -->\n"),
            None => format!("<!-- olp-key: {key} -->\n"),
        };
        entry.extend_from_slice(mark.as_bytes());
    }
    let mut lead = Vec::new();
    if !existing.is_empty() {
        // Start at the beginning of a line; a numbered entry opens with a
        // heading, which also wants a blank line above it.
        let ends_line = existing.last() == Some(&b'\n');
        if !ends_line {
            lead.push(b'\n');
        }
        let body_end = existing.len() - usize::from(ends_line);
        let last_line_blank = body_end == 0 || existing[body_end - 1] == b'\n';
        if number.is_some() && !last_line_blank {
            lead.push(b'\n');
        }
    }
    lead.extend_from_slice(&entry);
    if let Err(e) = append_to(board, &lead) {
        return die(prog, &format!("写不了板 {}: {e}", board.display()));
    }
    if let Some(n) = number {
        println!("{n}");
    }
    0
}

/// `argv` is what follows `olp id alloc`.
pub fn id_alloc(argv: &[String]) -> i32 {
    let prog = "olp-id-alloc";
    let usage = "用法: octoscode olp id alloc <登记文件.tsv> <前缀> <位数> <自然键>";
    if argv.iter().any(|w| w == "-h" || w == "--help") {
        println!(
            "{usage}\n\n在锁内按自然键分配顺序编号:同一个键再次请求返回原编号;各前缀独立计数。\nstdout 只输出编号。退出码:0 成功(含已存在);2 用法,未触碰登记文件"
        );
        return 0;
    }
    let [registry, prefix, width, key] = argv else {
        return die(prog, usage);
    };
    let registry = Path::new(registry);
    let prefix_ok = prefix
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic())
        && prefix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if !prefix_ok {
        return die(prog, "前缀须以字母开头,只含 [A-Za-z0-9-]");
    }
    if prefix.len() > 32 {
        return die(prog, "前缀超过 32 字节");
    }
    let width = match width.parse::<usize>() {
        Ok(w)
            if (1..=12).contains(&w)
                && width.len() <= 2
                && width.bytes().all(|b| b.is_ascii_digit()) =>
        {
            w
        }
        _ => return die(prog, "位数须为 1-12 的整数"),
    };
    if key.is_empty() {
        return die(prog, "自然键不能为空");
    }
    if key.len() > 500 {
        return die(prog, "自然键超过 500 字节");
    }
    if key.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return die(prog, "自然键不能含制表符、换行等控制字符");
    }
    let _lock = match lock(registry) {
        Ok(lock) => lock,
        Err(e) => return die(prog, &format!("拿不到登记文件的锁: {e}")),
    };
    let existing = std::fs::read(registry).unwrap_or_default();
    let mut max = 0u64;
    for line in existing.split(|b| *b == b'\n') {
        let fields: Vec<&[u8]> = line.split(|b| *b == b'\t').collect();
        if fields.len() < 3 || fields[1] != prefix.as_bytes() {
            continue;
        }
        if fields[2] == key.as_bytes() {
            println!("{}", String::from_utf8_lossy(fields[0]));
            return 0;
        }
        if let Some(n) = fields[0]
            .strip_prefix(prefix.as_bytes())
            .filter(|s| !s.is_empty() && s.iter().all(u8::is_ascii_digit))
            .and_then(|s| std::str::from_utf8(s).ok()?.parse::<u64>().ok())
        {
            max = max.max(n);
        }
    }
    let id = format!("{prefix}{:0width$}", max + 1);
    let mut line = Vec::new();
    if !existing.is_empty() && existing.last() != Some(&b'\n') {
        line.push(b'\n');
    }
    line.extend_from_slice(
        format!(
            "{id}\t{prefix}\t{key}\t{}\n",
            chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ")
        )
        .as_bytes(),
    );
    if let Err(e) = append_to(registry, &line) {
        return die(prog, &format!("写不了登记文件 {}: {e}", registry.display()));
    }
    println!("{id}");
    0
}
