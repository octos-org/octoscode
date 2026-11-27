//! `octoscode olp evo retro <repo> [--dry-run]` — the evolution loop's
//! retro: read the cards harvested since the last retro, group them into
//! candidates and write a brief with a record draft for each. Judging the
//! candidates stays with the outer reviewer.
//!
//! State lives outside the repository, under
//! `${OLP_EVO_STATE:-~/.octos/outer/evo}/<project key>/`: `retro.json`
//! (the last card seen and every run) and `retro/<time>-<run>.md` (the
//! briefs). A dry run prints the brief and creates nothing. On a
//! multi-machine deployment only the hub runs it (exit 3 elsewhere).
//!
//! Exit codes: 0 (also: no new cards); 1 error; 3 not the hub; 70 an
//! injected fault (tests).

use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::evolib::{Card, Json, group, group_text, hub_guard, parse_cards};

struct Brief {
    text: String,
    new_cards: usize,
    candidates: usize,
    max_seen: u64,
    malformed: Vec<String>,
}

fn take(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

fn brief(board: &Path, last_id: u64, flaw_start: u64) -> Brief {
    let text = std::fs::read_to_string(board).unwrap_or_default();
    let cards = parse_cards(&text);
    let whole = |c: &Card| {
        [&c.trigger, &c.identity, &c.symptom]
            .iter()
            .all(|field| field.as_deref().is_some_and(|v| !v.is_empty()))
    };
    let malformed: Vec<String> = cards
        .iter()
        .filter(|c| !whole(c) && c.num > last_id)
        .map(|c| c.id.clone())
        .collect();
    let fresh: Vec<Card> = cards
        .iter()
        .filter(|c| whole(c) && c.num > last_id)
        .cloned()
        .collect();
    let max_seen = cards.iter().map(|c| c.num).max().unwrap_or(0).max(last_id);
    let candidates = group(&fresh);
    let mut out = format!(
        "cards: {}\ncandidates: {}\nnote: recurrence_hint 是去重锚点数,不是跨 goal 计数;草稿 TODO 未消除前不得保存为 FLAW-NNN.md\n",
        fresh.len(),
        candidates.len()
    );
    for (i, candidate) in candidates.iter().enumerate() {
        let ids: Vec<&str> = candidate.cards.iter().map(|c| c.id.as_str()).collect();
        out.push_str(&format!(
            "\n## C{} {} · recurrence_hint={} · layer={}\nkey: {}\nanchors: {}\ncards: {}\n",
            i + 1,
            candidate.trigger,
            candidate.recurrence_hint(),
            candidate.layer,
            candidate.key,
            candidate.anchors.join(", "),
            ids.join(", ")
        ));
        for card in &candidate.cards {
            out.push_str(&format!(
                "- {} | source={} | envelope={} | symptom={}\n",
                card.id,
                card.source,
                card.envelope,
                take(card.symptom.as_deref().unwrap_or(""), 120)
            ));
        }
        out.push_str(&format!(
            "\n```yaml\ndraft: true\nkind: context\nid: FLAW-{:03}\ntitle: \"{}\"\nrepo: TODO\nlayers: [{}]\nstatus: open\nseverity: TODO\nrecurrence: {}\nfingerprint: TODO\ncards: [{}]\n```\n",
            flaw_start + i as u64,
            take(&group_text(&candidate.cards[0]), 60),
            candidate.layer,
            candidate.recurrence_hint(),
            ids.join(", ")
        ));
    }
    Brief {
        text: out,
        new_cards: fresh.len(),
        candidates: candidates.len(),
        max_seen,
        malformed,
    }
}

fn state(path: &Path) -> serde_json::Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// The next free record number in the repository's FLAW directory.
fn next_flaw(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|name| {
            let digits = name.strip_prefix("FLAW-")?.strip_suffix(".md")?;
            (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
                .then(|| digits.parse::<u64>().ok())?
        })
        .max()
        .unwrap_or(0)
        + 1
}

fn replace(path: &Path, text: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut tmp = tempfile::Builder::new()
        .prefix(".retro-")
        .tempfile_in(dir)?;
    tmp.write_all(text.as_bytes())?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// `argv` is what follows `olp evo retro`.
pub fn run(argv: &[String]) -> i32 {
    let Some(repo) = argv.first().filter(|a| !a.starts_with("--")) else {
        eprintln!("usage: octoscode olp evo retro <repo-root> [--dry-run]");
        return 1;
    };
    if argv.iter().any(|a| a == "-h" || a == "--help") {
        println!(
            "用法: octoscode olp evo retro <repo-root> [--dry-run]\n\n读上次 retro 之后采集到的卡,归成候选,写一份带记录草稿的简报。--dry-run 只打印,不建任何文件。\n多机部署时只在 hub 上运行。退出码:0(含没有新卡);1 出错;3 不是 hub"
        );
        return 0;
    }
    let dry_run = argv.get(1).is_some_and(|a| a == "--dry-run");
    // Before any write: off the hub the counters would collide with its.
    if !dry_run {
        if let Err(why) = hub_guard("olp-evo-retro") {
            eprintln!("{why}");
            return 3;
        }
    }
    let repo = Path::new(repo);
    if !repo.is_dir() {
        eprintln!("error: repo-root not found: {}", repo.display());
        return 1;
    }
    let real = std::fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
    let env_path = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let board = env_path("OLP_EVO_BOARD").unwrap_or_else(|| repo.join(".octos/EVOLUTION.md"));
    let digest = Sha256::digest(real.to_string_lossy().as_bytes());
    let key: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    let state_dir = env_path("OLP_EVO_STATE")
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".octos/outer/evo")
        })
        .join(&key);
    let retro_json = state_dir.join("retro.json");
    let stamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let last_id = |path: &Path| state(path)["last_id"].as_u64().unwrap_or(0);
    let flaw_start = next_flaw(&repo.join("knowledge/context/evolution"));

    if dry_run {
        // The brief to stdout, notes to stderr, and nothing created.
        let made = brief(&board, last_id(&retro_json), flaw_start);
        print!("{}", made.text);
        for id in &made.malformed {
            eprintln!("malformed-card: {id}");
        }
        return 0;
    }
    if std::fs::create_dir_all(&state_dir).is_err() {
        eprintln!("error: cannot create {}", state_dir.display());
        return 1;
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(state_dir.join("retro.lock"))
        .and_then(|file| fs2::FileExt::lock_exclusive(&file).map(|()| file));
    let Ok(_lock) = lock else {
        eprintln!("error: cannot lock {}", state_dir.display());
        return 1;
    };
    // Read again, now that nobody else can be writing.
    let made = brief(&board, last_id(&retro_json), flaw_start);
    for id in &made.malformed {
        eprintln!("malformed-card: {id}");
    }
    if made.new_cards == 0 {
        println!("retro: 0 new card(s)");
        return 0;
    }
    let retro_dir = state_dir.join("retro");
    if std::fs::create_dir_all(&retro_dir).is_err() {
        eprintln!("error: cannot create {}", retro_dir.display());
        return 1;
    }
    let before = state(&retro_json);
    let runs = before["runs"].as_array().cloned().unwrap_or_default();
    let run = runs.len() + 1;
    let brief_path = retro_dir.join(format!("{stamp}-{run}.md"));
    let page = format!("# retro {stamp} · {key} · run {run}\n\n{}", made.text);
    if let Err(e) = replace(&brief_path, &page) {
        eprintln!("error: {}: {e}", brief_path.display());
        return 1;
    }
    // Test hook: stop after the brief, before the state is updated.
    if std::env::var("OLP_EVO_TEST").as_deref() == Ok("1")
        && std::env::var("OLP_EVO_FAULT").as_deref() == Ok("after-brief")
    {
        eprintln!("fault-injected: after-brief");
        return 70;
    }
    let mut all: Vec<Json> = runs.iter().map(Json::raw).collect();
    all.push(Json::Obj(vec![
        ("ts".to_string(), Json::Str(stamp.clone())),
        ("run".to_string(), Json::Int(run as i64)),
        ("cards".to_string(), Json::Int(made.new_cards as i64)),
        ("candidates".to_string(), Json::Int(made.candidates as i64)),
        (
            "brief".to_string(),
            Json::Str(
                std::path::absolute(&brief_path)
                    .unwrap_or_else(|_| brief_path.clone())
                    .to_string_lossy()
                    .into_owned(),
            ),
        ),
    ]));
    let next = Json::Obj(vec![
        ("last_id".to_string(), Json::Int(made.max_seen as i64)),
        ("runs".to_string(), Json::Arr(all)),
    ]);
    if let Err(e) = replace(&retro_json, &next.line()) {
        eprintln!("error: {}: {e}", retro_json.display());
        return 1;
    }
    println!(
        "retro: {} new card(s), {} candidate(s), brief: {}",
        made.new_cards,
        made.candidates,
        brief_path.display()
    );
    0
}
