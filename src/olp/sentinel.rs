//! `octoscode olp board watch <板> <token>` — the board sentinel: fire when
//! a line appended after arming contains the token.
//!
//! Only lines added since the baseline are looked at (old text — a card
//! quoting the token, an earlier ACK of the same number — can never fire),
//! and the match is a plain substring (any prefix shape counts). Lines that
//! contain a `--skip-signature` are left out, so the outer loop's own notes
//! do not trip its own sentinel. A board that shrank is re-baselined.
//!
//! Without `--harvest` it exits 0 on the first hit. With `--harvest <repo>`
//! it runs the evolution harvest on each hit, reports, moves the baseline
//! and keeps watching.
//!
//! Exit codes: 0 hit (prints `BOARD-SIGNAL: <token>` and up to 3 lines);
//! 2 usage. A missing board is waited for.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

const USAGE: &str = "用法: octoscode olp board watch <板路径> <子串token> [--interval 秒(默认 20)] [--skip-signature <署名>]... [--harvest <repo-root>]";

fn newlines(path: &Path) -> Option<(usize, Vec<u8>)> {
    let bytes = std::fs::read(path).ok()?;
    Some((bytes.iter().filter(|b| **b == b'\n').count(), bytes))
}

fn harvest(repo: &str) -> i32 {
    // This binary's own harvest: no checkout is needed beside the board.
    let status = std::env::current_exe().and_then(|exe| {
        Command::new(exe)
            .args(["olp", "evo", "harvest", repo])
            .status()
    });
    match status {
        Ok(status) => status.code().unwrap_or(1),
        Err(_) => 127,
    }
}

/// `argv` is what follows `olp board watch`.
pub fn run(argv: &[String]) -> i32 {
    let (mut board, mut token, mut interval) = (None, None, "20".to_string());
    let mut skips: Vec<String> = Vec::new();
    let mut harvest_repo: Option<String> = None;
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        let mut value = |what: &str| {
            words.next().cloned().ok_or_else(|| {
                eprintln!("{word} 需要{what}");
            })
        };
        match word.as_str() {
            "-h" | "--help" => {
                println!(
                    "{USAGE}\n\n只看挂哨后新增的行,域内做子串匹配;命中时打印 BOARD-SIGNAL 与最多 3 行命中。\n--harvest:命中时先跑进化环采集,再报信号,然后推进基线继续监视。\n退出码:0 命中;2 参数错误。板不存在时等它出现。"
                );
                return 0;
            }
            "--interval" => match value("秒数") {
                Ok(v) => interval = v,
                Err(()) => return 2,
            },
            "--skip-signature" => match value("署名") {
                Ok(v) => skips.push(v),
                Err(()) => return 2,
            },
            "--harvest" => match value(" repo-root") {
                Ok(v) => harvest_repo = Some(v),
                Err(()) => return 2,
            },
            other if other.starts_with("--") => {
                eprintln!("unknown option: {other}");
                return 2;
            }
            other if board.is_none() => board = Some(other.to_string()),
            other if token.is_none() => token = Some(other.to_string()),
            other => interval = other.to_string(),
        }
    }
    let (Some(board), Some(token)) = (board, token) else {
        eprintln!("{USAGE}");
        return 2;
    };
    let seconds = match interval.parse::<u64>() {
        Ok(n) if interval.bytes().all(|b| b.is_ascii_digit()) => n,
        _ => {
            eprintln!("interval 必须是整数秒: {interval}");
            return 2;
        }
    };
    let pause = || std::thread::sleep(Duration::from_secs(seconds));
    let board = Path::new(&board);
    let mut base = loop {
        match newlines(board) {
            Some((count, _)) => break count,
            None => pause(),
        }
    };
    loop {
        if let Some((current, bytes)) = newlines(board) {
            if current > base {
                // Everything after the first `base` lines, a last line
                // without its newline included.
                let text = String::from_utf8_lossy(&bytes);
                let hits: Vec<&str> = text
                    .split('\n')
                    .skip(base)
                    .filter(|line| !skips.iter().any(|s| line.contains(s.as_str())))
                    .filter(|line| line.contains(token.as_str()))
                    .take(3)
                    .collect();
                if !hits.is_empty() {
                    if let Some(repo) = &harvest_repo {
                        let code = harvest(repo);
                        if code != 0 {
                            eprintln!("harvest: failed (exit {code})");
                        }
                    }
                    println!("BOARD-SIGNAL: {token}");
                    println!("{}", hits.join("\n"));
                    if harvest_repo.is_none() {
                        return 0;
                    }
                    base = current;
                }
            } else if current < base {
                // Truncated or rewritten: the baseline is void; old text is
                // not looked at again.
                base = current;
            }
        }
        pause();
    }
}
