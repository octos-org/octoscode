//! The evolution loop's read side: FLAW records, their index, and the
//! contract skeleton made from one.
//!
//! `octoscode olp evo index <repo>` writes
//! `knowledge/context/evolution/INDEX.md` from the `FLAW-*.md` records
//! there (a rerun with nothing changed leaves the file's mtime alone).
//! `octoscode olp evo skeleton <FLAW-NNN.md> [--out <file>]` prints an
//! agent-spec task contract skeleton for a person to complete; `--out` may
//! only name a file outside any repository or under a repository's
//! `specs/drafts/`.
//!
//! Exit codes: 0; 2 usage, or a refused `--out`.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// A FLAW record: its frontmatter, its `## ` sections, and the code paths
/// its 责任步 and 锚点 sections name.
pub struct Flaw {
    pub frontmatter: BTreeMap<String, String>,
    pub sections: BTreeMap<String, String>,
    pub paths: Vec<String>,
}

fn digits(s: &str) -> usize {
    s.len() - s.trim_start_matches(|c: char| c.is_ascii_digit()).len()
}

/// Does `s`, whole, read as a line-number note: ` L123`, `L123–L140`,
/// `L123-140`, `:123`, `:123-140`?
fn is_line_note(s: &str) -> bool {
    let dash = |rest: &str| -> Option<usize> {
        ['–', '-']
            .iter()
            .find_map(|d| rest.starts_with(*d).then(|| d.len_utf8()))
    };
    let lettered = {
        let rest = s.trim_start();
        match rest.strip_prefix('L') {
            Some(rest) if digits(rest) > 0 => {
                let rest = &rest[digits(rest)..];
                match dash(rest) {
                    None => rest.is_empty(),
                    Some(skip) => {
                        let rest = &rest[skip..];
                        let rest = rest.strip_prefix('L').unwrap_or(rest);
                        digits(rest) > 0 && digits(rest) == rest.len()
                    }
                }
            }
            _ => false,
        }
    };
    let colon = match s.strip_prefix(':') {
        Some(rest) if digits(rest) > 0 => {
            let rest = &rest[digits(rest)..];
            match dash(rest) {
                None => rest.is_empty(),
                Some(skip) => {
                    let rest = &rest[skip..];
                    digits(rest) > 0 && digits(rest) == rest.len()
                }
            }
        }
        _ => false,
    };
    lettered || colon
}

/// `text` without a trailing line-number note.
fn strip_line_note(text: &str) -> &str {
    text.char_indices()
        .map(|(at, _)| at)
        .find(|at| is_line_note(&text[*at..]))
        .map(|at| &text[..at])
        .unwrap_or(text)
}

pub fn parse_flaw(text: &str) -> Flaw {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut frontmatter = BTreeMap::new();
    let mut at = 0;
    if lines.first().is_some_and(|l| l.trim() == "---") {
        at = 1;
        while at < lines.len() && lines[at].trim() != "---" {
            if let Some((key, value)) = lines[at].split_once(':') {
                frontmatter.insert(key.trim().to_string(), value.trim().to_string());
            }
            at += 1;
        }
        at += 1; // past the closing fence
    }
    let mut sections = BTreeMap::new();
    let (mut name, mut body) = (String::new(), Vec::new());
    for line in lines.iter().skip(at) {
        if let Some(title) = line.strip_prefix("## ") {
            if !name.is_empty() {
                sections.insert(name.clone(), body.join("\n").trim().to_string());
            }
            name = title.trim().to_string();
            body.clear();
        } else if !name.is_empty() {
            body.push(*line);
        }
    }
    if !name.is_empty() {
        sections.insert(name, body.join("\n").trim().to_string());
    }
    // Backtick spans that contain a `/`, without their line-number notes.
    let mut paths: Vec<String> = Vec::new();
    for section in ["责任步", "锚点"] {
        let body = sections.get(section).map(String::as_str).unwrap_or("");
        let ticks: Vec<usize> = body.match_indices('`').map(|(i, _)| i).collect();
        let mut i = 0;
        while i + 1 < ticks.len() {
            let span = &body[ticks[i] + 1..ticks[i + 1]];
            if span.contains('/') {
                let path = strip_line_note(span).trim().to_string();
                if !path.is_empty() && !paths.contains(&path) {
                    paths.push(path);
                }
                i += 2;
            } else {
                // Not a path: its closing tick may open the next span.
                i += 1;
            }
        }
    }
    Flaw {
        frontmatter,
        sections,
        paths,
    }
}

/// `argv` is what follows `olp evo index`: the repository root.
pub fn index(argv: &[String]) -> i32 {
    let [repo] = argv else {
        eprintln!("usage: octoscode olp evo index <repo-root>");
        return 2;
    };
    if repo == "-h" || repo == "--help" {
        println!(
            "用法: octoscode olp evo index <repo-root>\n\n从 knowledge/context/evolution/FLAW-*.md 生成 INDEX.md;内容没变时不改写文件。"
        );
        return 0;
    }
    let repo = Path::new(repo);
    if !repo.is_dir() {
        eprintln!("error: repo-root not found: {}", repo.display());
        return 2;
    }
    let dir = repo.join("knowledge/context/evolution");
    let mut names: Vec<String> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.starts_with("FLAW-") && n.ends_with(".md") && n != "FLAW-template.md")
            .collect(),
        Err(e) => {
            eprintln!("error: {}: {e}", dir.display());
            return 1;
        }
    };
    names.sort();
    let mut rows: Vec<[String; 4]> = Vec::new();
    for name in &names {
        let text = std::fs::read_to_string(dir.join(name)).unwrap_or_default();
        let flaw = parse_flaw(&text);
        let field = |key: &str| flaw.frontmatter.get(key).map(String::as_str).unwrap_or("");
        let (issue, pr) = (field("issue").trim(), field("pr").trim());
        rows.push([
            flaw.frontmatter
                .get("id")
                .cloned()
                .unwrap_or_else(|| name.trim_end_matches(".md").to_string()),
            flaw.frontmatter
                .get("status")
                .cloned()
                .unwrap_or_else(|| "unknown".to_string()),
            field("layers").trim_matches(['[', ']']).to_string(),
            match (issue.is_empty(), pr.is_empty()) {
                (false, false) => format!("{issue} / {pr}"),
                (false, true) => issue.to_string(),
                (true, false) => pr.to_string(),
                (true, true) => "—".to_string(),
            },
        ]);
    }
    rows.sort_by(|a, b| a[0].cmp(&b[0]));
    // Prose the protocol retired in favour of a record: `> 已记录:FLAW-NNN`,
    // titled by the nearest heading above it.
    let mut retired: BTreeMap<String, String> = BTreeMap::new();
    if let Ok(protocol) = std::fs::read_to_string(repo.join("docs/OUTER_LOOP_PROTOCOL.md")) {
        let mut heading = String::new();
        for line in protocol.lines() {
            if line.starts_with('#') {
                heading = line.trim_start_matches('#').trim().to_string();
            }
            let number = line
                .strip_prefix('>')
                .map(str::trim_start)
                .and_then(|rest| rest.strip_prefix("已记录:FLAW-"))
                .map(|rest| &rest[..digits(rest)])
                .filter(|n| !n.is_empty());
            if let Some(number) = number {
                retired.entry(format!("FLAW-{number}")).or_insert_with(|| {
                    if heading.is_empty() {
                        "PROTOCOL".to_string()
                    } else {
                        heading.clone()
                    }
                });
            }
        }
    }
    let mut out = String::from(
        "# FLAW 索引(生成,勿手改)\n\n| FLAW | 状态 | 层 | issue / PR | 取代散文 |\n|---|---|---|---|---|\n",
    );
    for [id, status, layers, issue] in &rows {
        let prose = retired.get(id).map(String::as_str).unwrap_or("—");
        out.push_str(&format!(
            "| {id} | {status} | {layers} | {issue} | {prose} |\n"
        ));
    }
    out.push_str(&format!("\nretired_prose: {}\n", retired.len()));
    let target = dir.join("INDEX.md");
    if std::fs::read_to_string(&target).is_ok_and(|old| old == out) {
        return 0; // identical: the mtime stays
    }
    if let Err(e) = std::fs::write(&target, out) {
        eprintln!("error: {}: {e}", target.display());
        return 1;
    }
    println!("wrote {}", target.display());
    0
}

/// The path with `.` and `..` resolved and the symlinks of whatever part of
/// it exists followed; the rest need not exist.
fn resolve(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => {
                out.push(other);
                if let Ok(real) = std::fs::canonicalize(&out) {
                    out = real;
                }
            }
        }
    }
    out
}

/// `- ` items of a section, or the whole body as one item.
fn items(body: &str) -> Vec<String> {
    let listed: Vec<String> = body
        .split('\n')
        .filter_map(|l| l.strip_prefix("- "))
        .map(|l| l.trim().to_string())
        .collect();
    if listed.is_empty() && !body.trim().is_empty() {
        vec![body.trim().to_string()]
    } else {
        listed
    }
}

/// A test-name slug: the ASCII words of `text`, 48 characters at most, cut
/// at a word boundary.
fn slug(text: &str, n: usize) -> String {
    let words: Vec<String> = text
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let whole = words.join("_");
    if whole.is_empty() {
        return format!("item_{n}");
    }
    if whole.len() <= 48 {
        return whole;
    }
    let mut out = String::new();
    for segment in whole.split('_') {
        let candidate = if out.is_empty() {
            segment.to_string()
        } else {
            format!("{out}_{segment}")
        };
        if candidate.len() > 48 {
            break;
        }
        out = candidate;
    }
    if out.is_empty() {
        out = whole[..48].to_string();
    }
    out
}

/// `argv` is what follows `olp evo skeleton`.
pub fn skeleton(argv: &[String]) -> i32 {
    let Some((src, rest)) = argv.split_first() else {
        eprintln!("usage: octoscode olp evo skeleton <FLAW-NNN.md> [--out <file>]");
        return 2;
    };
    if src == "-h" || src == "--help" {
        println!(
            "用法: octoscode olp evo skeleton <FLAW-NNN.md> [--out <文件>]\n\n把一份 FLAW 记录变成任务合约的骨架(待人工补全),默认打印到 stdout。\n--out 只能指向仓库之外,或仓库的 specs/drafts/ 之下。退出码:0;2 用法或被拒绝的 --out"
        );
        return 0;
    }
    let mut out_file: Option<PathBuf> = None;
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        match (word.as_str(), words.next()) {
            ("--out", Some(path)) if !path.is_empty() => out_file = Some(PathBuf::from(path)),
            ("--out", _) => {
                eprintln!("--out needs a path");
                return 2;
            }
            (other, _) => {
                eprintln!("error: unknown flag {other}");
                return 2;
            }
        }
    }
    if !Path::new(src).is_file() {
        eprintln!("error: not a file: {src}");
        return 2;
    }
    // Where the target may go depends on the repository that owns the
    // target, not on the caller's directory.
    let out_file = match out_file {
        None => None,
        Some(path) => {
            let real = resolve(&path);
            let parent = real.parent().unwrap_or(Path::new("/")).to_path_buf();
            let mut probe = parent.clone();
            while !probe.is_dir() && probe.pop() {}
            let owner = std::process::Command::new("git")
                .arg("-C")
                .arg(&probe)
                .args(["rev-parse", "--show-toplevel"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|top| !top.is_empty());
            if let Some(top) = owner {
                if !real.starts_with(Path::new(&top).join("specs/drafts"))
                    || real == Path::new(&top).join("specs/drafts")
                {
                    eprintln!("refusing to write outside specs/drafts/");
                    return 2;
                }
            }
            // Only after the guard: a refused path leaves nothing behind.
            if let Err(e) = std::fs::create_dir_all(&parent) {
                eprintln!("error: {}: {e}", parent.display());
                return 2;
            }
            Some(path)
        }
    };
    let flaw = parse_flaw(&std::fs::read_to_string(src).unwrap_or_default());
    let field = |key: &str| {
        flaw.frontmatter
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    };
    let section = |names: [&str; 2]| {
        names
            .iter()
            .find_map(|n| flaw.sections.get(*n).filter(|b| !b.is_empty()))
            .map(String::as_str)
            .unwrap_or("")
    };
    let req = field("req")
        .or(field("satisfies"))
        .or(field("requirement"))
        .unwrap_or("");
    let id = flaw.frontmatter.get("id").cloned().unwrap_or_else(|| {
        Path::new(src)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let decisions = items(section(["修复", "结案"]));
    let forbidden = items(section(["预防", "保护门"]));
    let or_todo = |name: &str| {
        flaw.sections
            .get(name)
            .map(|b| b.trim())
            .filter(|b| !b.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("<!-- TODO: {name} -->"))
    };
    let mut lines: Vec<String> = vec![
        "spec: task".to_string(),
        format!("name: \"骨架(来自 {id},待人工补全)\""),
        if req.is_empty() {
            "satisfies: []".to_string()
        } else {
            format!("satisfies: [{req}]")
        },
        "tags: [evo-skeleton]".to_string(),
        "estimate: TODO".to_string(),
        "---".to_string(),
        String::new(),
        "## 意图".to_string(),
        String::new(),
        format!("{}\n\n根因:{}", or_todo("症状"), or_todo("根因")),
        String::new(),
        "## 已定决策".to_string(),
        String::new(),
    ];
    if decisions.is_empty() {
        lines.push("<!-- TODO: 已定决策(来源段:修复/结案) -->".to_string());
    }
    lines.extend(decisions.iter().map(|d| format!("- {d}")));
    lines.extend(["", "## 边界", "", "### Allowed Changes"].map(str::to_string));
    if flaw.paths.is_empty() {
        lines.push("<!-- TODO: Allowed(来源段:责任步/锚点 paths) -->".to_string());
    }
    lines.extend(flaw.paths.iter().map(|p| format!("- {p}")));
    lines.extend(["", "### Forbidden"].map(str::to_string));
    if forbidden.is_empty() {
        lines.push("<!-- TODO: Forbidden(来源段:预防/保护门) -->".to_string());
    }
    lines.extend(forbidden.iter().map(|f| format!("- {f}")));
    lines.extend(
        [
            "",
            "## 排除范围",
            "",
            "<!-- TODO: 排除范围 -->",
            "",
            "## 完成条件",
            "",
        ]
        .map(str::to_string),
    );
    if decisions.is_empty() {
        lines.push("<!-- TODO: 完成条件 -->".to_string());
    }
    for (n, decision) in decisions.iter().enumerate() {
        let head: String = decision.chars().take(40).collect();
        lines.push(format!("场景: {head}"));
        lines.push(format!("  测试: pending_{}", slug(decision, n + 1)));
        lines.extend(["  假设 TODO", "  当 TODO", "  那么 TODO", ""].map(str::to_string));
    }
    lines.extend(["## 问题", ""].map(str::to_string));
    if req.is_empty() {
        lines.push("- 未绑定需求".to_string());
    }
    let text = format!("{}\n", lines.join("\n").trim_end());
    match out_file {
        None => print!("{text}"),
        Some(path) => {
            if let Err(e) = std::fs::write(&path, text) {
                eprintln!("error: {}: {e}", path.display());
                return 2;
            }
            println!("wrote {}", path.display());
        }
    }
    0
}
