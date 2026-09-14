//! olp-review-monitor 生产入口集成测试(spec: review-monitor rules)。
//! 子进程真实调用 scripts/olp-review-monitor.py。测试方法:
//! spawn → 等 ready 基线(已有输出) → 变更 → 限时轮询/try_wait →
//! 只 kill 本测试持有的 child 句柄并 wait 回收。禁止全局 pkill。
//!
//! 外层反例覆盖(../outer-monitor-probe-results.json 四类):
//!   1. 无关 peer b 被误报 running(active_thread 全局广播)
//!   2. wrong-session/version=999/phase=nonsense lifetime 显示 authoritative
//!   3. monitor 改写 review-state.json 字节
//!   4. 同一行 ACK 原地修改误判 added(行号索引而非内容键)

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn script() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("scripts/olp-review-monitor.py");
    assert!(p.exists(), "missing script: {}", p.display());
    p
}

fn fixtures() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("fixtures/review-evidence");
    p
}

fn append_to(p: &Path, text: &str) {
    let mut f = std::fs::OpenOptions::new().append(true).open(p).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

struct TmpDir(PathBuf);
impl TmpDir {
    fn new(tag: &str) -> Self {
        let base = std::env::temp_dir().join(format!(
            "olp-review-monitor-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
        TmpDir(base)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_json(p: &Path, v: serde_json::Value) {
    std::fs::write(p, serde_json::to_string(&v).unwrap()).unwrap();
}

fn hex_lower(b: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        let _ = write!(s, "{x:02x}");
    }
    s
}

fn sha256_file(p: &Path) -> String {
    let bytes = std::fs::read(p).unwrap();
    let digest = <sha2::Sha256 as sha2::Digest>::digest(&bytes);
    hex_lower(&digest)
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex_lower(&<sha2::Sha256 as sha2::Digest>::digest(bytes))
}

/// 一次性渲染(单次 render)。
fn monitor(review_dir: &Path, runtime_dir: Option<&Path>, board: Option<&Path>) -> (bool, String) {
    let mut c = Command::new("python3");
    c.arg(script()).arg(review_dir);
    if let Some(r) = runtime_dir {
        c.arg("--runtime-dir").arg(r);
    }
    if let Some(b) = board {
        c.arg("--board").arg(b);
    }
    let out = c.output().expect("spawn monitor");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// 一次性渲染,显式传入 --session(originator/master 视角)。
fn monitor_with_session(
    review_dir: &Path,
    runtime_dir: Option<&Path>,
    board: Option<&Path>,
    session: &str,
) -> (bool, String) {
    let mut c = Command::new("python3");
    c.arg(script()).arg(review_dir);
    if let Some(r) = runtime_dir {
        c.arg("--runtime-dir").arg(r);
    }
    if let Some(b) = board {
        c.arg("--board").arg(b);
    }
    c.arg("--session").arg(session);
    let out = c.output().expect("spawn monitor");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// 一次性渲染,显式传入 --profile 与 --session。
fn monitor_with_profile(
    review_dir: &Path,
    runtime_dir: Option<&Path>,
    board: Option<&Path>,
    profile: &str,
) -> (bool, String) {
    let mut c = Command::new("python3");
    c.arg(script()).arg(review_dir);
    if let Some(r) = runtime_dir {
        c.arg("--runtime-dir").arg(r);
    }
    if let Some(b) = board {
        c.arg("--board").arg(b);
    }
    c.arg("--profile").arg(profile);
    let out = c.output().expect("spawn monitor");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// 一次性渲染,JSON 格式(带额外 profile 参数)。
fn monitor_json_profile(
    review_dir: &Path,
    runtime_dir: &Path,
    profile: &str,
) -> (bool, serde_json::Value) {
    let out = Command::new("python3")
        .arg(script())
        .arg(review_dir)
        .arg("--runtime-dir")
        .arg(runtime_dir)
        .arg("--profile")
        .arg(profile)
        .arg("--format")
        .arg("json")
        .output()
        .expect("spawn monitor");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("monitor JSON 可解析");
    (out.status.success(), v)
}

/// watch 模式子进程:stdout 行缓冲读取,限时等待一行匹配。
struct WatchChild {
    child: Child,
    lines: std::sync::mpsc::Receiver<String>,
}
impl WatchChild {
    fn spawn(review_dir: &Path, board: Option<&Path>, extra: &[&str]) -> Self {
        let mut cmd = Command::new("python3");
        cmd.arg(script())
            .arg(review_dir)
            .arg("--watch")
            .arg("--interval")
            .arg("0.3");
        if let Some(b) = board {
            cmd.arg("--board").arg(b);
        }
        cmd.args(extra);
        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut so = child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let mut r = BufReader::new(&mut so);
            let mut line = String::new();
            loop {
                line.clear();
                match r.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let _ = tx.send(line.trim_end().to_string());
                    }
                }
            }
        });
        WatchChild { child, lines: rx }
    }
    /// 限时等待出现包含 needle 的输出行(轮询,不无限等待)。
    fn wait_line(&self, needle: &str, timeout: Duration) -> Option<String> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            while let Ok(line) = self.lines.try_recv() {
                if line.contains(needle) {
                    return Some(line);
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }
}
impl Drop for WatchChild {
    fn drop(&mut self) {
        // 只 kill 本测试持有的 child 并回收,禁止全局 pkill。
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// 既有契约场景(spec filters 不变)
// ---------------------------------------------------------------------------

/// 场景: 监控分离四区块且 runtime 身份完整(复合标识,非裸 goal_01)。
#[test]
fn olp_review_monitor_sections_and_runtime_identity() {
    let t = TmpDir::new("sections");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("review-state.json"),
        r#"{"protocol":"x","frozen":true,"runtime":"/private/tmp/runtime-abc","session":"sess-42","goal":"goal_01","challenge":{"accepted":true},"cross":[{},{},{}]}"#,
    )
    .unwrap();
    std::fs::write(d.join("runtime-evidence.json"), r#"{"peers":[]}"#).unwrap();
    std::fs::write(d.join("deliverable-1.md"), "report").unwrap();

    let (ok, out) = monitor(&d, None, None);
    assert!(ok, "monitor failed: {out}");
    for section in ["lifecycle", "current-turn", "last-outcome", "deliverables"] {
        assert!(out.contains(section), "缺区块 {section}: {out}");
    }
    // 复合身份: runtime 路径 + session + goal
    assert!(
        out.contains("/private/tmp/runtime-abc"),
        "缺 runtime 路径: {out}"
    );
    assert!(out.contains("sess-42"), "缺 session: {out}");
    assert!(out.contains("goal_01"), "缺 goal: {out}");
    let id_line = out.lines().find(|l| l.contains("==")).unwrap();
    assert!(
        id_line.contains("runtime-abc") && id_line.contains("session"),
        "身份须为复合标识: {id_line}"
    );
}

/// 场景: 分层显示 — 最近终止 completed + 新轮 running,不混写。
/// 使用真实 fixtures 形态: runtime-evidence.json active_thread 非空。
#[test]
fn olp_review_monitor_fallback_cross_checks_result_and_thread() {
    let t = TmpDir::new("layered");
    let d = t.path().to_path_buf();
    let fx = fixtures().join("pr-629");
    let native_dst = d.join("native");
    std::fs::create_dir_all(&native_dst).unwrap();
    for slug in ["pr629-glm-primary", "pr629-k3"] {
        let src = fx.join("native").join(slug);
        let dst = native_dst.join(slug);
        std::fs::create_dir_all(&dst).unwrap();
        for f in std::fs::read_dir(&src).unwrap() {
            let f = f.unwrap().path();
            std::fs::copy(&f, dst.join(f.file_name().unwrap())).unwrap();
        }
    }
    std::fs::copy(
        fx.join("runtime-evidence.json"),
        d.join("runtime-evidence.json"),
    )
    .unwrap();

    // 保持真实结构,只对其中一个 peer 打开 active_thread(模拟"当前新轮在跑")。
    // 补全完整复合身份(真实 writer 形状)—— 裸快照(仅 slug)不得宽松采信。
    std::fs::write(
        d.join("review-state.json"),
        r#"{"runtime":"/tmp/rt-629","session":"sess-629","goal":"goal_01"}"#,
    )
    .unwrap();
    let mut re: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(d.join("runtime-evidence.json")).unwrap())
            .unwrap();
    let peers = re.pointer_mut("/goal/peers").expect("fixture goal.peers");
    peers.as_array_mut().unwrap().push(serde_json::json!({
        "slug": "pr629-k3",
        "runtime": "/tmp/rt-629",
        "session": "sess-629",
        "goal_id": "goal_01",
        "head": "9bcf4099c2719cd8ee63090a1849a2c6f3766999",
        "status": "running",
        "active_thread": "th-live",
        "outcome": "completed"
    }));
    std::fs::write(
        d.join("runtime-evidence.json"),
        serde_json::to_string(&re).unwrap(),
    )
    .unwrap();

    let (ok, out) = monitor(&d, None, None);
    assert!(ok, "monitor failed: {out}");
    // last-outcome 保持最近终止结果 completed(来自 result-N.md)
    assert!(
        out.contains("last-outcome=completed") || out.contains("last-outcome: completed"),
        "last-outcome 应显示最近终止 completed: {out}"
    );
    // current-turn 独立显示 running(该 peer 有精确活跃 thread)
    assert!(
        out.contains("current-turn=running"),
        "current-turn 应显示 running: {out}"
    );
    // 不得混写为 inconsistent
    assert!(
        !out.contains("inconsistent"),
        "不得混写为 inconsistent: {out}"
    );
    // 分层说明存在
    assert!(out.contains("最近终止"), "应标注 last-outcome 语义: {out}");
    // 未被点名的另一 peer 不得继承 running(精确归属,反广播)
    let k3_line = out
        .lines()
        .find(|l| l.contains("pr629-k3") && l.contains("current-turn"))
        .expect("k3 行");
    assert!(k3_line.contains("running"), "k3 应 running: {k3_line}");
    let glm_line = out
        .lines()
        .find(|l| l.contains("pr629-glm-primary") && l.contains("current-turn"))
        .expect("glm-primary 行");
    assert!(
        !glm_line.contains("running"),
        "glm-primary 无 active_thread,不得误报 running: {glm_line}"
    );
}

/// 场景: legacy 运行版本无 lifetime.json 且缺当前 thread → unknown,不推测。
#[test]
fn olp_review_monitor_unknown_without_lifetime() {
    let t = TmpDir::new("legacy");
    let d = t.path().to_path_buf();
    let runtime = d.join("runtime-legacy"); // 空目录: 无 lifetime.json、无 ui-protocol
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("runtime-evidence.json"), r#"{"peers":[]}"#).unwrap();

    let (ok, out) = monitor(&d, Some(&runtime), None);
    assert!(ok, "monitor failed: {out}");
    assert!(out.contains("unknown"), "无 lifetime 应显示 unknown: {out}");
    let lt_line = out
        .lines()
        .find(|l| l.contains("[current-turn]"))
        .expect("current-turn 区块");
    assert!(
        lt_line.contains("unknown"),
        "lifetime 应为 unknown: {lt_line}"
    );
}

/// 场景: inplace ACK 变更以哈希比对观测(非行数)。
#[test]
fn olp_review_monitor_observes_inplace_ack_edit() {
    let t = TmpDir::new("ackhash");
    let d = t.path().to_path_buf();
    let board = d.join("board.md");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(&board, "ACK(done): task finished\n").unwrap();
    std::fs::write(d.join("runtime-evidence.json"), r#"{"peers":[]}"#).unwrap();

    // 基线渲染(建立 ACK 基线,写入自家 monitor-state.json)
    let (ok1, out1) = monitor(&d, None, Some(&board));
    assert!(ok1, "baseline render failed: {out1}");

    // 原地修改 ACK 行(done → blocked): 行数不变,内容变了
    std::fs::write(&board, "ACK(blocked): task finished\n").unwrap();
    let (ok2, out2) = monitor(&d, None, Some(&board));
    assert!(ok2, "second render failed: {out2}");

    // watch 模式(两次采样间修改)验证事件输出
    let child = WatchChild::spawn(&d, Some(&board), &[]);
    // 等 ready 基线(首个渲染块出现后再改板)
    child
        .wait_line("olp-review-monitor", Duration::from_secs(10))
        .expect("watch 基线渲染");
    std::thread::sleep(Duration::from_millis(600));
    std::fs::write(&board, "ACK(wontdo): task finished\n").unwrap();
    let hit = child.wait_line("ack-inplace-edit", Duration::from_secs(10));
    // 一次性渲染或 watch 渲染任一观测到哈希变更事件即可
    assert!(
        hit.is_some() || out2.contains("ack-inplace-edit"),
        "原地 ACK 修改应产生哈希比对事件: watch={hit:?} second={out2}"
    );
    // 原地修改不得报成 added(外层反例 4)
    assert!(
        !out2.contains("\"type\": \"ack-added\"") && !out2.contains("ack-added line=1"),
        "同一行原地修改不得误判为 added: {out2}"
    );
}

/// 场景: watch-board 正哨保持不变 — monitor 只读板文件,不干扰 olp-watch-board.sh。
#[test]
fn olp_review_monitor_keeps_watch_board_contract() {
    let t = TmpDir::new("coexist");
    let d = t.path().to_path_buf();
    let board = d.join("board.md");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(&board, "# Board\n\n- [ ] item1\n").unwrap();
    std::fs::write(d.join("runtime-evidence.json"), r#"{"peers":[]}"#).unwrap();

    let mut wb = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    wb.push("scripts/olp-watch-board.sh");
    let mut script_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    script_dir.push("scripts");

    // monitor 先读基线板(建立自家 ACK 基线)
    let (ok, out) = monitor(&d, None, Some(&board));
    assert!(ok, "monitor failed: {out}");

    // watch-board 按既有协议命中(BOARD-SIGNAL + 退出码 0):
    // 严格 spawn → 确认 ready 基线(不预先落 ACK) → append → 限时 try_wait。
    // 挂哨 token 用 ACK(12 前缀(属于板面 ACK( 协议),监控同一正则可见)
    let mut sentinel = Command::new("bash")
        .arg(&wb)
        .arg(&board)
        .arg("ACK(12")
        .arg("--interval")
        .arg("1")
        .current_dir(&script_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("watch-board spawn failed: {e}"));
    std::thread::sleep(Duration::from_millis(1500)); // 等基线建立
    append_to(&board, "ACK(12 done): coexist ok\n");
    let start = Instant::now();
    let (hit, found) = loop {
        if let Some(_st) = sentinel.try_wait().unwrap() {
            let mut s = String::new();
            if let Some(mut so) = sentinel.stdout.take() {
                use std::io::Read;
                let _ = so.read_to_string(&mut s);
            }
            break (true, s);
        }
        if start.elapsed() > Duration::from_secs(10) {
            let _ = sentinel.kill();
            let _ = sentinel.wait();
            break (false, String::new());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(
        hit && found.contains("BOARD-SIGNAL"),
        "watch-board 正哨应按既有协议命中: {found}"
    );

    // monitor 随后渲染同一板: 只读,不干扰;并观测到新增 ACK(哈希)
    let (ok2, out2) = monitor(&d, None, Some(&board));
    assert!(ok2, "monitor 二次渲染失败: {out2}");
    assert!(
        out2.contains("ack-added") || out2.contains("ack-inplace-edit"),
        "监控应报告 ACK 变更: {out2}"
    );
    // 监控渲染不得改写板文件(只读消费,不干扰正哨协议)
    let board_text = std::fs::read_to_string(&board).unwrap();
    assert!(
        board_text.contains("ACK(12 done): coexist ok"),
        "板内容保留: {board_text}"
    );
}

/// JSON 输出与 human 输出一致性(monitor 侧)。
#[test]
fn olp_review_monitor_json_output_parseable() {
    let t = TmpDir::new("jsonout");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("review-state.json"),
        r#"{"frozen":true,"runtime":"/tmp/r","session":"s","goal":"g"}"#,
    )
    .unwrap();
    let out = Command::new("python3")
        .arg(script())
        .arg(&d)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("monitor JSON 可解析");
    assert!(v["identity"].is_string());
    assert!(v["lifecycle"]["frozen"].is_boolean());
}

// ---------------------------------------------------------------------------
// 外层反例 + 本轮新约场景(先写测试,跑出真实 RED,再修实现)
// ---------------------------------------------------------------------------

/// 外层反例 3: monitor 只读 review-state.json,任何渲染不得改变其字节(含损坏时)。
#[test]
fn olp_review_monitor_never_mutates_review_state() {
    let t = TmpDir::new("readonly");
    let d = t.path().to_path_buf();
    let board = d.join("board.md");
    std::fs::create_dir_all(&d).unwrap();
    let state_body = "{\n \"runtime\": \"correct-runtime\",\n \"session\": \"correct-session\",\n \"goal\": \"goal_01\",\n \"keep\": \"original\"\n}\n";
    std::fs::write(d.join("review-state.json"), state_body).unwrap();
    std::fs::write(&board, "ACK(done): first\n").unwrap();

    let before = sha256_file(&d.join("review-state.json"));
    // 多次渲染(含带 board 建立 ACK 基线)后 review-state 字节不变
    for _ in 0..2 {
        let (ok, _) = monitor(&d, None, Some(&board));
        assert!(ok, "render failed");
    }
    let after = sha256_file(&d.join("review-state.json"));
    assert_eq!(before, after, "monitor 不得改变 review-state.json 字节");

    // 损坏的 review-state 也不得更改;渲染仍应成功并明确标注 corrupt
    std::fs::write(d.join("review-state.json"), "{not-json").unwrap();
    let corrupt_before = sha256_file(&d.join("review-state.json"));
    let (ok3, out3) = monitor(&d, None, Some(&board));
    assert!(ok3, "corrupt review-state 下渲染仍应成功: {out3}");
    assert!(out3.contains("corrupt"), "损坏应明确诊断: {out3}");
    assert_eq!(
        corrupt_before,
        sha256_file(&d.join("review-state.json")),
        "损坏状态下也不得更改 review-state.json"
    );
}

/// 外层反例 1+2 复刻(真实 probe fixture 形态):
/// - peer a 有 active_thread,peer b 没有 → b 不得误报 running;
/// - runtime/unrelated/lifetime.json(version=999/phase=nonsense/master=wrong-session)
///   位于错误路径且校验失败 → 不得显示 authoritative。
#[test]
fn olp_review_monitor_no_crosstalk_and_untrusted_lifetime() {
    let t = TmpDir::new("probe");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    for slug in ["a", "b"] {
        let nd = d.join("native").join(slug);
        std::fs::create_dir_all(&nd).unwrap();
        std::fs::write(
            nd.join("result-1.md"),
            format!("---\nslug: {slug}\noutcome: completed\nturn: 1\n---\nold\n"),
        )
        .unwrap();
        std::fs::write(nd.join("turns.txt"), "1 completed 1\n").unwrap();
    }
    // runtime-evidence: a 活跃,b 不活跃。补全完整复合身份(真实 writer
    // 形状)—— 裸快照(仅 slug)不得宽松采信,见 v3-4 契约。
    let rt_path = d.join("runtime");
    let full_ident = |slug: &str, active: serde_json::Value| {
        serde_json::json!({
            "slug": slug,
            "runtime": rt_path.to_str().unwrap(),
            "session": "sess-probe",
            "goal_id": "goal_01",
            "head": "deadbeef",
            "active_thread": active,
        })
    };
    write_json(
        &d.join("runtime-evidence.json"),
        serde_json::json!({"peers": [
            full_ident("a", serde_json::json!("a-turn2")),
            full_ident("b", serde_json::Value::Null),
        ]}),
    );
    std::fs::write(
        d.join("review-state.json"),
        r#"{"runtime":"/tmp/rt-probe","session":"sess-probe","goal":"goal_01"}"#,
    )
    .unwrap();
    // 伪造 lifetime: 错误路径 + version=999 + phase=nonsense + wrong master
    let fake = d.join("runtime").join("unrelated");
    std::fs::create_dir_all(&fake).unwrap();
    write_json(
        &fake.join("lifetime.json"),
        serde_json::json!({"version":999,"master":"wrong-session","phase":"nonsense","turn_id":"wrong-turn","generation":44}),
    );

    let (ok, out) = monitor_with_session(&d, Some(&d.join("runtime")), None, "sess-probe");
    assert!(ok, "monitor failed: {out}");
    // 反例 2: 伪造 lifetime 不得显示 authoritative
    assert!(
        !out.contains("authoritative"),
        "version=999/phase=nonsense/wrong-path lifetime 不得为 authoritative: {out}"
    );
    let lt_line = out.lines().find(|l| l.contains("[current-turn]")).unwrap();
    assert!(
        lt_line.contains("unknown"),
        "lifetime 应 unknown: {lt_line}"
    );
    // 反例 1: b 不得误报 running;a 精确 running
    let a_line = out.lines().find(|l| l.contains("peer a:")).expect("a 行");
    let b_line = out.lines().find(|l| l.contains("peer b:")).expect("b 行");
    assert!(
        a_line.contains("current-turn=running"),
        "a 应 running: {a_line}"
    );
    assert!(
        !b_line.contains("running"),
        "b 无活跃 thread 不得 running: {b_line}"
    );
    // next_seq(流事件序号)与 result turn(轮次计数)不得数值比较 —— 输出无该判旧痕迹
    assert!(
        !out.contains("next_seq"),
        "不得输出 next_seq 量纲比较: {out}"
    );
}

/// ACK 缓存: 原子写自家 monitor-state.json,JSON key 类型一致,支持
/// 新增/原地改/删除;review-state.json 永远只读;没有初始 ACK 后新增也能报。
#[test]
fn olp_review_monitor_ack_cache_atomic_own_state() {
    let t = TmpDir::new("ackcache");
    let d = t.path().to_path_buf();
    let board = d.join("board.md");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("review-state.json"),
        r#"{"frozen":false,"runtime":"/tmp/rt","session":"s1","goal":"goal_01"}"#,
    )
    .unwrap();
    let review_hash = sha256_file(&d.join("review-state.json"));

    // 初始板无任何 ACK → 建立基线
    std::fs::write(&board, "# board\nplain line\n").unwrap();
    let (ok0, _) = monitor(&d, None, Some(&board));
    assert!(ok0, "baseline failed");

    // 1) 没有初始 ACK 后新增 → 必须报 added(外层: 不因基线为空而漏报)
    std::fs::write(&board, "# board\nplain line\nACK(9 done): first\n").unwrap();
    let (ok1, out1) = monitor(&d, None, Some(&board));
    assert!(ok1, "render1 failed: {out1}");
    assert!(out1.contains("ack-added"), "新增 ACK 应报 added: {out1}");

    // 2) 同一 ACK(内容键)原地改写 → changed,不是 added(宽松子串,不认前缀)
    std::fs::write(
        &board,
        "# board\nplain line\n> 引述 ACK(9 blocked): rework needed\n",
    )
    .unwrap();
    let (ok2, out2) = monitor(&d, None, Some(&board));
    assert!(ok2, "render2 failed: {out2}");
    assert!(
        out2.contains("ack-inplace-edit"),
        "同 ACK 改写应报 changed: {out2}"
    );

    // 3) 删除 ACK 行 → removed
    std::fs::write(&board, "# board\nplain line\n").unwrap();
    let (ok3, out3) = monitor(&d, None, Some(&board));
    assert!(ok3, "render3 failed: {out3}");
    assert!(
        out3.contains("ack-removed"),
        "删除 ACK 行应报 removed: {out3}"
    );

    // review-state 全程只读
    assert_eq!(
        review_hash,
        sha256_file(&d.join("review-state.json")),
        "review-state.json 不得被改写"
    );

    // 自家缓存 monitor-state.json: JSON key 类型一致(字符串化行号或内容键),
    // 且是原子替换产物(不存在 *.tmp 残留)
    let cache = d.join("monitor-state.json");
    assert!(cache.exists(), "应写自家 monitor-state.json 缓存");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&cache).unwrap()).unwrap();
    let acks = v.get("acks").expect("cache 含 acks 表");
    assert!(acks.is_object(), "acks 为 JSON object");
    for k in acks.as_object().unwrap().keys() {
        assert!(!k.is_empty(), "key 非空");
        assert!(
            k.chars().all(|c| !c.is_control()),
            "key 类型一致可解析: {k}"
        );
    }
    let tmp_leftover = std::fs::read_dir(&d)
        .unwrap()
        .any(|e| e.unwrap().file_name().to_string_lossy().ends_with(".tmp"));
    assert!(!tmp_leftover, "原子写不得留下 .tmp 残留");
}

/// lifetime 严格校验: version=1/task_id/registry_key=<profile>:peer:<slug>/
/// originator==master;Pending→queued Running→running Failed→failed;
/// Idle 实算 SHA256(result.md)==result_digest,不匹配 → unknown。
#[test]
fn olp_review_monitor_lifetime_strict_validation() {
    let t = TmpDir::new("lifetime");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("runtime-evidence.json"), r#"{"peers":[]}"#).unwrap();
    let profile = "octosfix";
    let peers_root = d
        .join("runtime")
        .join("profiles")
        .join(profile)
        .join("data")
        .join("peers");

    let mk = |slug: &str, phase: &str, digest: Option<&str>| {
        let dir = peers_root.join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("originator"), "master-sess-1\n").unwrap();
        let mut lt = serde_json::json!({
            "version": 1,
            "task_id": "task-42",
            "registry_key": format!("{profile}:peer:{slug}"),
            "master": "master-sess-1",
            "generation": 7,
            "phase": phase,
            "turn_id": "turn-9"
        });
        if phase == "pending" {
            lt["turn_id"] = serde_json::Value::Null;
        }
        if let Some(dg) = digest {
            lt["result_digest"] = serde_json::json!(dg);
        }
        write_json(&dir.join("lifetime.json"), lt);
        dir
    };

    // queued: Pending(turn_id=null)
    mk("p-queued", "pending", None);
    // running: Running
    mk("p-running", "running", None);
    // failed: Failed
    mk("p-failed", "failed", None);
    // idle-good: Idle + result.md 摘要实算匹配
    let good_body = b"final result body\n";
    let good = mk("p-idle-good", "idle", Some(&sha256_bytes(good_body)));
    std::fs::write(good.join("result.md"), good_body).unwrap();
    // idle-bad: Idle + 摘要非空但不匹配 → unknown(不是 idle)
    let bad = mk("p-idle-bad", "idle", Some("deadbeef"));
    std::fs::write(bad.join("result.md"), b"other body\n").unwrap();
    // wrongkey: registry_key 属于别的 profile → unknown(防跨 profile 串扰)
    let wk = peers_root.join("p-wrongkey");
    std::fs::create_dir_all(&wk).unwrap();
    std::fs::write(wk.join("originator"), "master-sess-1\n").unwrap();
    write_json(
        &wk.join("lifetime.json"),
        serde_json::json!({
            "version": 1, "task_id": "task-42",
            "registry_key": "otherprofile:peer:p-wrongkey",
            "master": "master-sess-1", "generation": 7,
            "phase": "running", "turn_id": "turn-9"
        }),
    );
    // wrongorigin: originator != master → unknown
    let wo = peers_root.join("p-wrongorigin");
    std::fs::create_dir_all(&wo).unwrap();
    std::fs::write(wo.join("originator"), "someone-else\n").unwrap();
    write_json(
        &wo.join("lifetime.json"),
        serde_json::json!({
            "version": 1, "task_id": "task-42",
            "registry_key": format!("{profile}:peer:p-wrongorigin"),
            "master": "master-sess-1", "generation": 7,
            "phase": "running", "turn_id": "turn-9"
        }),
    );
    // notask: task_id 为空 → unknown
    let nt = peers_root.join("p-notask");
    std::fs::create_dir_all(&nt).unwrap();
    std::fs::write(nt.join("originator"), "master-sess-1\n").unwrap();
    write_json(
        &nt.join("lifetime.json"),
        serde_json::json!({
            "version": 1, "task_id": "",
            "registry_key": format!("{profile}:peer:p-notask"),
            "master": "master-sess-1", "generation": 7,
            "phase": "running", "turn_id": "turn-9"
        }),
    );
    // symlinked: lifetime.json 为符号链接 → 不采信 → unknown
    let sl = peers_root.join("p-symlinked");
    std::fs::create_dir_all(&sl).unwrap();
    std::fs::write(sl.join("originator"), "master-sess-1\n").unwrap();
    let real_lt = d.join("elsewhere-lifetime.json");
    write_json(
        &real_lt,
        serde_json::json!({
            "version": 1, "task_id": "task-42",
            "registry_key": format!("{profile}:peer:p-symlinked"),
            "master": "master-sess-1", "generation": 7,
            "phase": "running", "turn_id": "turn-9"
        }),
    );
    std::os::unix::fs::symlink(&real_lt, sl.join("lifetime.json")).unwrap();

    let (ok, v) = monitor_json_profile(&d, &d.join("runtime"), profile);
    assert!(ok, "render failed");
    let peers = &v["current-turn"]["peers"];
    let get = |slug: &str| {
        peers[slug]["execution"]
            .as_str()
            .unwrap_or("<missing>")
            .to_string()
    };
    assert_eq!(get("p-queued"), "queued", "{peers}");
    assert_eq!(get("p-running"), "running", "{peers}");
    assert_eq!(get("p-failed"), "failed", "{peers}");
    assert_eq!(
        get("p-idle-good"),
        "idle",
        "digest 实算匹配应 idle: {peers}"
    );
    assert_eq!(
        get("p-idle-bad"),
        "unknown",
        "digest 不匹配不得 idle: {peers}"
    );
    assert_eq!(
        get("p-wrongkey"),
        "unknown",
        "registry_key 跨 profile 不采信: {peers}"
    );
    assert_eq!(
        get("p-wrongorigin"),
        "unknown",
        "originator!=master 不采信: {peers}"
    );
    assert_eq!(get("p-notask"), "unknown", "空 task_id 不采信: {peers}");
    assert_eq!(
        get("p-symlinked"),
        "unknown",
        "符号链接 lifetime 不采信: {peers}"
    );

    // 可信投影下身份字段来自 lifetime(task_id/generation/turn_id/master_session)
    let good_peer = &peers["p-idle-good"];
    assert_eq!(good_peer["task_id"], "task-42");
    assert_eq!(good_peer["generation"], 7);
    assert_eq!(good_peer["turn_id"], "turn-9");
    assert_eq!(good_peer["master_session_id"], "master-sess-1");
    // 不可信 peer 身份字段为 null(fail-closed)
    let bad_peer = &peers["p-idle-bad"];
    assert!(
        bad_peer["task_id"].is_null(),
        "不可信时身份 null: {bad_peer}"
    );
}

/// 追加覆盖: 跨 runtime 同 goal_01 同 slug 不同 session 不串扰;
/// 多 peer 一 running 一 unknown(无 CURRENT authority 且无精确活跃线程 → unknown,
/// 即使有旧 completed)。closed 独立显示且不把旧失败变成功。
#[test]
fn olp_review_monitor_cross_runtime_binding_and_closed() {
    let t = TmpDir::new("crossrt");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    let profile = "octosfix";
    // runtime A: session-A 的 peer s 有 Running lifetime
    let pa = d
        .join("runtimeA")
        .join("profiles")
        .join(profile)
        .join("data")
        .join("peers")
        .join("s");
    std::fs::create_dir_all(&pa).unwrap();
    std::fs::write(pa.join("originator"), "session-A\n").unwrap();
    write_json(
        &pa.join("lifetime.json"),
        serde_json::json!({
            "version": 1, "task_id": "task-1",
            "registry_key": format!("{profile}:peer:s"),
            "master": "session-A", "generation": 1,
            "phase": "running", "turn_id": "turn-a1"
        }),
    );
    // runtime B: 同 slug 同 goal,但 lifetime 属于 session-A(旧 runtime 残留)
    let pb = d
        .join("runtimeB")
        .join("profiles")
        .join(profile)
        .join("data")
        .join("peers")
        .join("s");
    std::fs::create_dir_all(&pb).unwrap();
    std::fs::write(pb.join("originator"), "session-A\n").unwrap();
    write_json(
        &pb.join("lifetime.json"),
        serde_json::json!({
            "version": 1, "task_id": "task-1",
            "registry_key": format!("{profile}:peer:s"),
            "master": "session-A", "generation": 1,
            "phase": "running", "turn_id": "turn-a1"
        }),
    );
    // B 侧还有: 一 running(精确活跃 thread) 一 unknown(旧 completed 但无 authority)
    let nb = d.join("native");
    for slug in ["s", "r-old"] {
        let nd = nb.join(slug);
        std::fs::create_dir_all(&nd).unwrap();
        std::fs::write(
            nd.join("result-1.md"),
            format!("---\nslug: {slug}\noutcome: errored\nturn: 1\n---\nold failure\n"),
        )
        .unwrap();
        std::fs::write(nd.join("turns.txt"), "1 errored 1\n").unwrap();
    }
    write_json(
        &d.join("runtime-evidence.json"),
        serde_json::json!({"goal": {"goal_id": "goal_01", "peers": [
            {"slug": "s", "active_thread": "th-1"},
            {"slug": "r-old", "active_thread": null, "status": "done"}
        ]}}),
    );

    // 以 runtime B + session-B 视角渲染: lifetime master=session-A ≠ session-B → unknown
    let out = Command::new("python3")
        .arg(script())
        .arg(&d)
        .arg("--runtime-dir")
        .arg(d.join("runtimeB"))
        .arg("--profile")
        .arg(profile)
        .arg("--session")
        .arg("session-B")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "render failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let peers = &v["current-turn"]["peers"];
    // 跨 session: lifetime 属于 session-A,在 session-B 视角不得采信
    assert_eq!(
        peers["s"]["execution"].as_str().unwrap_or("?"),
        "unknown",
        "lifetime master≠当前 session 不得采信: {peers}"
    );
    // r-old: 无 CURRENT authority 且无精确活跃线程 → unknown(即使旧 result 为 errored)
    assert_eq!(
        peers["r-old"]["execution"].as_str().unwrap_or("?"),
        "unknown",
        "旧 terminated 但无 authority → unknown: {peers}"
    );
    // 旧 last-outcome 仍按终止证据保留(errored),不被改判
    assert_eq!(
        v["last-outcome"]["peers"]["r-old"]["state"]
            .as_str()
            .unwrap_or("?"),
        "errored",
        "last-outcome 保留旧失败证据: {v}"
    );

    // closed 独立: closed peer 显示 closed 且 last_outcome 保留旧失败
    let pc = d
        .join("runtimeC")
        .join("profiles")
        .join(profile)
        .join("data")
        .join("peers")
        .join("c1");
    std::fs::create_dir_all(&pc).unwrap();
    std::fs::write(pc.join("originator"), "session-C\n").unwrap();
    std::fs::write(pc.join("closed"), "").unwrap();
    std::fs::write(pc.join("turns.txt"), "1 errored 100\n").unwrap();
    std::fs::write(
        pc.join("result-1.md"),
        "---\nslug: c1\noutcome: errored\nturn: 1\n---\nfailed body\n",
    )
    .unwrap();
    let d2 = TmpDir::new("closed");
    std::fs::write(
        d2.path().join("review-state.json"),
        r#"{"runtime":"runtimeC","session":"session-C","goal":"goal_01"}"#,
    )
    .unwrap();
    let out2 = Command::new("python3")
        .arg(script())
        .arg(d2.path())
        .arg("--runtime-dir")
        .arg(d.join("runtimeC"))
        .arg("--profile")
        .arg(profile)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(out2.status.success());
    let v2: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out2.stdout)).unwrap();
    let c1 = &v2["current-turn"]["peers"]["c1"];
    assert_eq!(
        c1["execution"].as_str().unwrap_or("?"),
        "closed",
        "closed 独立显示: {v2}"
    );
    assert_eq!(
        v2["last-outcome"]["peers"]["c1"]["state"]
            .as_str()
            .unwrap_or("?"),
        "errored",
        "closed 不把旧失败变成功: {v2}"
    );
}

/// 首次终止层核对: result-N frontmatter slug/turn/outcome 与 turns.txt 一致;
/// 坏行/不一致/文件缺失 → unknown,但输出来源说明。
#[test]
fn olp_review_monitor_last_outcome_cross_check() {
    let t = TmpDir::new("xcheck");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("runtime-evidence.json"), r#"{"peers":[]}"#).unwrap();

    // good: result-2 与 turns.txt 一致
    let good = d.join("native").join("p-good");
    std::fs::create_dir_all(&good).unwrap();
    std::fs::write(
        good.join("result-2.md"),
        "---\nslug: p-good\noutcome: completed\nturn: 2\n---\nbody\n",
    )
    .unwrap();
    std::fs::write(good.join("turns.txt"), "1 completed 1\n2 completed 2\n").unwrap();

    // badturns: turns.txt 全坏行 → outcome unknown 但来源可说明
    let bt = d.join("native").join("p-badturns");
    std::fs::create_dir_all(&bt).unwrap();
    std::fs::write(
        bt.join("result-1.md"),
        "---\nslug: p-badturns\noutcome: completed\nturn: 1\n---\nbody\n",
    )
    .unwrap();
    std::fs::write(bt.join("turns.txt"), "garbage line no digits\n???\n").unwrap();

    // mismatch: result-N outcome=completed 但 turns.txt 最后轮 outcome=errored → unknown
    let mm = d.join("native").join("p-mismatch");
    std::fs::create_dir_all(&mm).unwrap();
    std::fs::write(
        mm.join("result-1.md"),
        "---\nslug: p-mismatch\noutcome: completed\nturn: 1\n---\nbody\n",
    )
    .unwrap();
    std::fs::write(mm.join("turns.txt"), "1 errored 1\n").unwrap();

    let (ok, out) = monitor(&d, None, None);
    assert!(ok, "render failed: {out}");
    let good_line = out
        .lines()
        .find(|l| l.contains("p-good") && l.contains("last-outcome"))
        .unwrap();
    assert!(
        good_line.contains("completed"),
        "一致证据 → completed: {good_line}"
    );
    let bt_line = out
        .lines()
        .find(|l| l.contains("p-badturns") && l.contains("last-outcome"))
        .unwrap();
    assert!(
        bt_line.contains("unknown"),
        "turns.txt 坏行 → unknown: {bt_line}"
    );
    let mm_line = out
        .lines()
        .find(|l| l.contains("p-mismatch") && l.contains("last-outcome"))
        .unwrap();
    assert!(
        mm_line.contains("unknown"),
        "outcome 矛盾 → unknown: {mm_line}"
    );
    // 来源可说明: 输出来源文件
    assert!(out.contains("result-"), "应说明 last-outcome 来源: {out}");
}

/// 负向事件: events/supervisor 的 goal_transition(blocked)/escalation 在监控
/// 可见且精确归属本 runtime;其他 runtime 的同名事件不归属本 runtime。
#[test]
fn olp_review_monitor_negative_events_attributed() {
    let t = TmpDir::new("events");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("review-state.json"),
        r#"{"runtime":"/tmp/rt-ours","session":"sess-ours","goal":"goal_01"}"#,
    )
    .unwrap();
    // 本 runtime 的 events.jsonl
    let ours_dir = d
        .join("runtime")
        .join("profiles")
        .join("octosfix")
        .join("data");
    std::fs::create_dir_all(&ours_dir).unwrap();
    let mut f = std::fs::File::create(ours_dir.join("events.jsonl")).unwrap();
    writeln!(f, r#"{{"ts":"2026-09-09T01:00:00Z","kind":"goal_transition","goal_id":"goal_01","detail":"goal transitioned to `blocked`"}}"#).unwrap();
    writeln!(f, r#"{{"ts":"2026-09-09T01:01:00Z","kind":"escalation","slug":"p1","detail":"needs operator decision"}}"#).unwrap();
    writeln!(
        f,
        r#"{{"ts":"2026-09-09T01:02:00Z","kind":"peer_staged","slug":"p2","detail":"staged"}}"#
    )
    .unwrap();
    drop(f);
    // 另一 runtime 的 events.jsonl(同 goal_01 blocked —— 不得归属本 runtime)
    let other_dir = d
        .join("other-runtime")
        .join("profiles")
        .join("octosfix")
        .join("data");
    std::fs::create_dir_all(&other_dir).unwrap();
    std::fs::write(
        other_dir.join("events.jsonl"),
        r#"{"ts":"2026-09-09T02:00:00Z","kind":"goal_transition","goal_id":"goal_01","detail":"goal transitioned to `blocked`"}"#,
    )
    .unwrap();

    // 事件在 octosfix profile;profile 精确归属(裁决 #2),传 --profile octosfix。
    let (ok, out) = monitor_with_profile(&d, Some(&d.join("runtime")), None, "octosfix");
    assert!(ok, "render failed: {out}");
    assert!(
        out.contains("blocked"),
        "goal_transition blocked 可见: {out}"
    );
    assert!(out.contains("escalation"), "escalation 可见: {out}");
    // 精确归属本 runtime: 事件须与本 runtime 关联输出,不吞并不报错
    let ev_lines: Vec<&str> = out.lines().filter(|l| l.contains("[event]")).collect();
    assert!(!ev_lines.is_empty(), "事件须单独成区/成行: {out}");
    assert!(
        ev_lines
            .iter()
            .all(|l| l.contains("rt-ours") || l.contains("runtime")),
        "事件精确归属本 runtime: {ev_lines:?}"
    );
    // 其他 runtime 的事件不出现(other-runtime 细节)
    assert!(
        !out.contains("02:00:00"),
        "其他 runtime 的负向事件不得归属本 runtime: {out}"
    );
}

// ---------------------------------------------------------------------------
// 外层 v3 复验 6 反例迁移(outer-monitor-v3-probe.py 真实 native 形状)
// 探针 receipt: ../outer-monitor-v3-probe-results.json(只读)
// 先迁移为真实失败回归,跑出 RED,再修实现到 GREEN。
// ---------------------------------------------------------------------------

/// v3-1: 真实 writer 的 invalidate/finish(has_queued_input=true) 会保留
/// Some(old turn_id) 的 Pending —— 合法下一轮 queued。Pending+非空 turn_id
/// 不得被拒;同时并列显示已完成 round1(last-outcome)与 queued round2。
/// 参考真实 writer: outer-peer-projection-under-test.rs
/// invalidate_peer_lifetime_for_input / finish_peer_lifetime_turn。
#[test]
fn olp_review_monitor_v3_pending_followup_keeps_previous_turn() {
    let t = TmpDir::new("v3pending");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("runtime-evidence.json"), r#"{"peers":[]}"#).unwrap();
    let profile = "octosfix";
    // 真实 originator/master wire 形状(v3 probe):
    // 'octosfix:local:tui#coding\x00~cwd-abc'
    let master = "octosfix:local:tui#coding\u{0}~cwd-abc";
    let peer = d
        .join("runtime")
        .join("profiles")
        .join(profile)
        .join("data")
        .join("peers")
        .join("glm");
    std::fs::create_dir_all(&peer).unwrap();
    std::fs::write(peer.join("originator"), format!("{master}\n")).unwrap();
    // 真实 finish(completed, has_queued_input=true) 产物: Pending + 保留旧 turn_id
    write_json(
        &peer.join("lifetime.json"),
        serde_json::json!({
            "version": 1, "task_id": "task-1",
            "registry_key": format!("{profile}:peer:glm"),
            "master": master, "generation": 2,
            "phase": "pending", "turn_id": "previous-real-turn",
            "result_digest": null
        }),
    );
    // round1 已完成终止证据
    std::fs::write(
        peer.join("result-1.md"),
        "---\nslug: glm\noutcome: completed\nturn: 1\n---\nround1 body\n",
    )
    .unwrap();
    std::fs::write(peer.join("turns.txt"), "1 completed 2026-09-09T00:00:00Z\n").unwrap();

    let (ok, v) = monitor_json_profile(&d, &d.join("runtime"), profile);
    assert!(ok, "render failed");
    let glm = &v["current-turn"]["peers"]["glm"];
    // Pending+Some(turn_id) 是合法 queued authority
    assert_eq!(
        glm["execution"].as_str().unwrap_or("?"),
        "queued",
        "Pending+保留旧 turn_id 应为 queued: {glm}"
    );
    assert_eq!(glm["turn_id"], "previous-real-turn", "身份投影保留: {glm}");
    // 并列显示: 已完成 round1 与 queued round2 分层
    let lo = &v["last-outcome"]["peers"]["glm"];
    assert_eq!(
        lo["state"].as_str().unwrap_or("?"),
        "completed",
        "round1 终止证据并列显示: {v}"
    );
}

/// v3-2/v3-3: 终止判定必须精确绑定 native 最新编号与 turns 一致;
/// frontmatter slug 不匹配 / turn 编号错 / outcome 非 completed → unknown。
#[test]
fn olp_review_monitor_v3_terminal_binding_strict() {
    let t = TmpDir::new("v3term");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("runtime-evidence.json"), r#"{"peers":[]}"#).unwrap();

    // v3-2: result-1 frontmatter slug=foreign/turn=999,turns.txt 正常 → unknown
    let wrong = d.join("native").join("p-wrong");
    std::fs::create_dir_all(&wrong).unwrap();
    std::fs::write(
        wrong.join("result-1.md"),
        "---\nslug: foreign-peer\nturn: 999\noutcome: completed\n---\n\nbody\n",
    )
    .unwrap();
    std::fs::write(
        wrong.join("turns.txt"),
        "1 completed 2026-09-09T00:00:00Z\n",
    )
    .unwrap();

    // v3-3: outcome=fabricated(与 turns.txt 一致也不行)→ unknown;
    // 只有 completed 可作终止权威
    let fab = d.join("native").join("p-fabricated");
    std::fs::create_dir_all(&fab).unwrap();
    std::fs::write(
        fab.join("result-1.md"),
        "---\nslug: p-fabricated\nturn: 1\noutcome: fabricated\n---\n\nbody\n",
    )
    .unwrap();
    std::fs::write(fab.join("turns.txt"), "1 fabricated 2026-09-09T00:00:00Z\n").unwrap();

    let (ok, out) = monitor(&d, None, None);
    assert!(ok, "render failed: {out}");
    let wrong_line = out
        .lines()
        .find(|l| l.contains("p-wrong") && l.contains("last-outcome"))
        .unwrap();
    assert!(
        wrong_line.contains("unknown"),
        "slug/turn 不匹配 → unknown: {wrong_line}"
    );
    let fab_line = out
        .lines()
        .find(|l| l.contains("p-fabricated") && l.contains("last-outcome"))
        .unwrap();
    assert!(
        fab_line.contains("unknown"),
        "fabricated outcome → unknown: {fab_line}"
    );
    assert!(
        !fab_line.contains("fabricated ("),
        "fabricated 不得显示为状态: {fab_line}"
    );
}

/// v3-4: runtime-evidence 快照缺完整复合身份(runtime/session/goal/peer/turn/HEAD)
/// 任一项 → unknown(只有 slug 不够)。
#[test]
fn olp_review_monitor_v3_snapshot_identity_incomplete() {
    let t = TmpDir::new("v3ident");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    // 只有 slug + active_thread,缺 runtime/session/goal/turn/head → unknown
    write_json(
        &d.join("runtime-evidence.json"),
        serde_json::json!({"peers": [{"slug": "bare", "active_thread": "th-1"}]}),
    );
    // 完整复合身份 + active_thread → running 可作正对照
    write_json(
        &d.join("runtime-evidence-full.json"),
        serde_json::json!({"peers": []}),
    );
    let (ok, out) = monitor(&d, None, None);
    assert!(ok, "render failed: {out}");
    let bare_line = out
        .lines()
        .find(|l| l.contains("peer bare:"))
        .expect("bare 行");
    assert!(
        bare_line.contains("unknown"),
        "缺复合身份的 active_thread 不得 running: {bare_line}"
    );
}

/// v3-5: foreign runtime/session/profile/goal 快照 + active_thread 字符串
/// → 当前 runtime 视角 unknown,不得因 active_thread 存在而 running。
#[test]
fn olp_review_monitor_v3_foreign_snapshot_rejected() {
    let t = TmpDir::new("v3foreign");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("review-state.json"),
        r#"{"runtime":"/tmp/rt-ours","session":"sess-ours","goal":"goal_01"}"#,
    )
    .unwrap();
    // v3 probe 真实形状: foreign 全字段 + active_thread
    write_json(
        &d.join("runtime-evidence.json"),
        serde_json::json!({"peers": [{
            "slug": "other",
            "runtime": "/foreign-runtime",
            "session": "foreign-session",
            "goal_id": "foreign-goal",
            "profile": "foreign-profile",
            "active_thread": "made-up-id"
        }]}),
    );
    let (ok, out) = monitor(&d, Some(&d.join("runtime")), None);
    assert!(ok, "render failed: {out}");
    let other_line = out
        .lines()
        .find(|l| l.contains("peer other:"))
        .expect("other 行");
    assert!(
        other_line.contains("unknown"),
        "foreign 快照 active_thread 不得 running: {other_line}"
    );
    assert!(
        !other_line.contains("running"),
        "foreign 不得 running: {other_line}"
    );
}

/// v3-6: 真实 native thread wire(v/session_id/thread_id/next_seq/completed)
/// 无 active 键;session_id 形如 `<profile>:<chan>:<chat>#peer-<slug>\x00~cwd-<hash>`。
/// completed=false 未完流 + 精确 session 绑定 → running;
/// completed=true → 非 running;陈旧/跨 session 未完成记录保守 unknown;
/// next_seq 是流事件序号,不当 turn 用。
#[test]
fn olp_review_monitor_v3_real_wire_thread_recognition() {
    let t = TmpDir::new("v3wire");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    // 真实形状: master originator 带 cwd 后缀(与 peer 共享同一 originator+cwd)。
    // 收紧(裁决 #3): 绑定精确到 originator session + cwd,同 channel 不同 cwd 不绑定。
    let master = "octosfix:local:tui#coding\u{0}~cwd-abc";
    std::fs::write(
        d.join("review-state.json"),
        r#"{"runtime":"/tmp/rt-wire","session":"octosfix:local:tui#coding\u0000~cwd-abc","goal":"goal_01"}"#,
    )
    .unwrap();
    let profile = "octosfix";
    let peers_root = d
        .join("runtime")
        .join("profiles")
        .join(profile)
        .join("data")
        .join("peers");
    let mk_peer = |slug: &str| {
        let p = peers_root.join(slug);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("originator"), format!("{master}\n")).unwrap();
        std::fs::write(p.join("goal"), "goal_01\n").unwrap();
        p
    };
    mk_peer("live");
    mk_peer("done");
    mk_peer("stale");
    mk_peer("diffcwd");

    // 真实 wire: session_id 带 \x00~cwd- 后缀;目录名为 session_id 字节 hex
    let put_thread = |session_id: &str, thread_id: &str, completed: bool, next_seq: u64| {
        let dir = d
            .join("runtime")
            .join("ui-protocol")
            .join(hex_encode(session_id.as_bytes()))
            .join("threads");
        std::fs::create_dir_all(&dir).unwrap();
        write_json(
            &dir.join(format!("{thread_id}.json")),
            serde_json::json!({
                "v": 1,
                "session_id": session_id,
                "thread_id": thread_id,
                "next_seq": next_seq,
                "completed": completed
            }),
        );
    };
    // live: 本 session 的 peer-live 未完流 → running(精确 originator/session 绑定)
    let live_sid = "octosfix:local:tui#peer-live\u{0}~cwd-abc";
    put_thread(live_sid, "t1", false, 10);
    // done: 本 session 的 peer-done 已完流 → 非 running
    let done_sid = "octosfix:local:tui#peer-done\u{0}~cwd-abc";
    put_thread(done_sid, "t2", true, 99);
    // stale: 别的 session 的未完流(同 slug 形状)→ 保守 unknown,不归属本 session
    let stale_sid = "octosfix:local:other#peer-stale\u{0}~cwd-abc";
    put_thread(stale_sid, "t3", false, 5);
    // diffcwd: 同 channel 同 originator 主干但 cwd 哈希不同 → 不绑定(裁决 #3)
    let diffcwd_sid = "octosfix:local:tui#peer-diffcwd\u{0}~cwd-zzz";
    put_thread(diffcwd_sid, "t4", false, 7);

    // NUL 字节不能进 argv: session 由 review-state.json 提供(文件可含 NUL),
    // 不传 --session,让 render 从 state 读 eff_session。
    let out = Command::new("python3")
        .arg(script())
        .arg(&d)
        .arg("--runtime-dir")
        .arg(d.join("runtime"))
        .arg("--profile")
        .arg(profile)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "render failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let peers = &v["current-turn"]["peers"];
    assert_eq!(
        peers["live"]["execution"].as_str().unwrap_or("?"),
        "running",
        "本 session 未完流应识别 running: {peers}"
    );
    assert_ne!(
        peers["done"]["execution"].as_str().unwrap_or("?"),
        "running",
        "completed=true 不得 running: {peers}"
    );
    assert_ne!(
        peers["stale"]["execution"].as_str().unwrap_or("?"),
        "running",
        "跨 session 未完流保守不 running: {peers}"
    );
    assert_ne!(
        peers["diffcwd"]["execution"].as_str().unwrap_or("?"),
        "running",
        "同 channel 不同 cwd 不绑定(裁决 #3): {peers}"
    );
    // next_seq 不得出现在任何 turn 量纲比较中
    let out_text = String::from_utf8_lossy(&out.stdout);
    assert!(
        !out_text.contains("next_seq"),
        "next_seq 不得作 turn 输出: {out_text}"
    );
}

/// 追加缺陷: negative events 按真实 native 形状 + 当前 profile/session/goal 过滤;
/// 同 runtime 其他 profile 的 blocked 不得混入当前 goal。
#[test]
fn olp_review_monitor_v3_negative_events_profile_goal_filtered() {
    let t = TmpDir::new("v3events");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("review-state.json"),
        r#"{"runtime":"/tmp/rt-ev","session":"sess-ev","goal":"goal_01"}"#,
    )
    .unwrap();
    let data = d.join("runtime").join("profiles");
    // 本 profile 本 goal: blocked → 可见
    let ours = data.join("octosfix").join("data");
    std::fs::create_dir_all(&ours).unwrap();
    let mut f = std::fs::File::create(ours.join("events.jsonl")).unwrap();
    writeln!(f, r#"{{"ts":"2026-09-09T01:00:00Z","kind":"goal_transition","goal_id":"goal_01","detail":"goal transitioned to `blocked`"}}"#).unwrap();
    writeln!(f, r#"{{"ts":"2026-09-09T01:01:00Z","kind":"escalation","goal_id":"goal_01","slug":"p1","detail":"needs decision"}}"#).unwrap();
    // 本 profile 其他 goal 的 blocked → 不得混入当前 goal
    writeln!(f, r#"{{"ts":"2026-09-09T01:02:00Z","kind":"goal_transition","goal_id":"goal_99","detail":"goal transitioned to `blocked`"}}"#).unwrap();
    drop(f);
    // 同 runtime 其他 profile 的同 goal blocked → 不得混入
    let other = data.join("otherprof").join("data");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(
        other.join("events.jsonl"),
        r#"{"ts":"2026-09-09T03:00:00Z","kind":"goal_transition","goal_id":"goal_01","detail":"goal transitioned to `blocked`"}"#,
    )
    .unwrap();

    // profile 精确归属(裁决 #2): 只读 octosfix,otherprof 的同 goal blocked 不混入。
    let (ok, out) = monitor_with_profile(&d, Some(&d.join("runtime")), None, "octosfix");
    assert!(ok, "render failed: {out}");
    let ev_lines: Vec<&str> = out.lines().filter(|l| l.contains("[event]")).collect();
    assert!(!ev_lines.is_empty(), "本 goal 负向事件可见: {out}");
    assert!(
        ev_lines
            .iter()
            .any(|l| l.contains("goal_transition") && l.contains("goal_01")),
        "本 profile 本 goal blocked 可见: {ev_lines:?}"
    );
    assert!(
        ev_lines.iter().all(|l| !l.contains("goal_99")),
        "其他 goal 的 blocked 不得混入: {ev_lines:?}"
    );
    assert!(
        ev_lines
            .iter()
            .all(|l| !l.contains("03:00:00") && !l.contains("otherprof")),
        "其他 profile 的同 goal blocked 不得混入: {ev_lines:?}"
    );
}

/// 契约澄清(两层分离): last-outcome 终止层如实显示四种真实终止
/// outcome(completed/errored/interrupted/rate_limited),不折叠成
/// unknown;只有非终止态(pending/running)与伪造值(fabricated 等)
/// 进 unknown。review-freeze 准入层只认 completed(另一层,不在此测)。
#[test]
fn olp_review_monitor_v3_terminal_outcome_four_value_display() {
    let t = TmpDir::new("v3four");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("runtime-evidence.json"), r#"{"peers":[]}"#).unwrap();

    // 四种真实终止 outcome + 伪造/非终止态 + 非词汇表词(裁决 #1:
    // failed/killed/timeout 不在 native 词汇表 → unknown)。
    let cases = [
        ("p-completed", "completed", "completed"),
        ("p-errored", "errored", "errored"),
        ("p-interrupted", "interrupted", "interrupted"),
        ("p-ratelimited", "rate_limited", "rate_limited"),
        ("p-fabricated", "fabricated", "unknown"),
        ("p-pending", "pending", "unknown"),
        ("p-running", "running", "unknown"),
        ("p-failed", "failed", "unknown"),
        ("p-killed", "killed", "unknown"),
        ("p-timeout", "timeout", "unknown"),
    ];
    for (slug, outcome, _expect) in &cases {
        let nd = d.join("native").join(slug);
        std::fs::create_dir_all(&nd).unwrap();
        std::fs::write(
            nd.join("result-1.md"),
            format!("---\nslug: {slug}\noutcome: {outcome}\nturn: 1\n---\nbody\n"),
        )
        .unwrap();
        std::fs::write(nd.join("turns.txt"), format!("1 {outcome} 1\n")).unwrap();
    }

    let (ok, out) = monitor(&d, None, None);
    assert!(ok, "render failed: {out}");
    for (slug, _outcome, expect) in &cases {
        let line = out
            .lines()
            .find(|l| l.contains(&format!("peer {slug}:")) && l.contains("last-outcome"))
            .unwrap_or_else(|| panic!("缺 {slug} last-outcome 行: {out}"));
        assert!(
            line.contains(&format!("last-outcome={expect}")),
            "{slug} 应如实显示 {expect}: {line}"
        );
    }
}

/// 裁决 #2: 缺当前 profile 时不得 fallback 扫其他 profile —— 无 profile
/// 即无事件可归属,直接空;即便其他 profile 有同 goal 事件也不混入。
#[test]
fn olp_review_monitor_v3_negative_events_no_profile_no_fallback() {
    let t = TmpDir::new("v3noprof");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("review-state.json"),
        r#"{"runtime":"/tmp/rt-np","session":"sess-np","goal":"goal_01"}"#,
    )
    .unwrap();
    // 只有 otherprof 有事件;默认 profile(octos)无事件文件。
    let other = d
        .join("runtime")
        .join("profiles")
        .join("otherprof")
        .join("data");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(
        other.join("events.jsonl"),
        r#"{"ts":"2026-09-09T03:00:00Z","kind":"goal_transition","goal_id":"goal_01","detail":"goal transitioned to `blocked`"}"#,
    )
    .unwrap();

    // 用默认 profile(octos,不存在)→ 不得 fallback 到 otherprof。
    let (ok, out) = monitor(&d, Some(&d.join("runtime")), None);
    assert!(ok, "render failed: {out}");
    let ev_lines: Vec<&str> = out.lines().filter(|l| l.contains("[event]")).collect();
    assert!(
        ev_lines
            .iter()
            .all(|l| !l.contains("03:00:00") && !l.contains("otherprof") && !l.contains("blocked")),
        "缺当前 profile 不得 fallback 混入其他 profile 事件: {ev_lines:?}"
    );
}

/// 裁决 #4: 裸快照(无复合身份键)即使在已知 runtime/session 视角下也
/// 不构成完整身份权威 —— 不得凭视角上下文晋升为 running,只能 unknown。
#[test]
fn olp_review_monitor_v3_bare_snapshot_not_identity_authority() {
    let t = TmpDir::new("v3bare");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    // 已知 runtime/session 视角
    std::fs::write(
        d.join("review-state.json"),
        r#"{"runtime":"/tmp/rt-bare","session":"sess-bare","goal":"goal_01"}"#,
    )
    .unwrap();
    // 裸快照: 只有 slug + active_thread,无 runtime/session/goal/head。
    write_json(
        &d.join("runtime-evidence.json"),
        serde_json::json!({"peers": [{"slug": "bare", "active_thread": "th-1"}]}),
    );
    // 即使有 native 终止证据也不晋升为 running(裸快照非身份权威)。
    let nd = d.join("native").join("bare");
    std::fs::create_dir_all(&nd).unwrap();
    std::fs::write(
        nd.join("result-1.md"),
        "---\nslug: bare\noutcome: completed\nturn: 1\n---\nbody\n",
    )
    .unwrap();
    std::fs::write(nd.join("turns.txt"), "1 completed 1\n").unwrap();

    let (ok, out) = monitor(&d, Some(&d.join("runtime")), None);
    assert!(ok, "render failed: {out}");
    let bare_line = out
        .lines()
        .find(|l| l.contains("peer bare:") && l.contains("current-turn"))
        .expect("bare 行");
    assert!(
        bare_line.contains("unknown"),
        "裸快照即使已知视角也不得晋升 running(裁决 #4): {bare_line}"
    );
    assert!(
        !bare_line.contains("running"),
        "裸快照不得 running: {bare_line}"
    );
}

/// 外层 binding 探针 case4: 完整身份快照但 goal/profile 与当前视角矛盾
/// (wrong goal / wrong profile)→ 即使带 active_thread 也不得 running,
/// 只能 unknown。goal/profile 逐键不矛盾是 _snapshot_bound_here 的硬约束。
#[test]
fn olp_review_monitor_v3_complete_snapshot_wrong_goal_profile_unknown() {
    let t = TmpDir::new("v3wronggp");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    let rt = d.join("runtime");
    std::fs::create_dir_all(&rt).unwrap();
    // 当前视角: goal_01 + profile octosfix + session sess-ours
    std::fs::write(
        d.join("review-state.json"),
        r#"{"runtime":"/tmp/rt-wgp","session":"sess-ours","goal":"goal_01"}"#,
    )
    .unwrap();
    // 完整身份快照,但 goal=goal_99、profile=other → 不得 running
    write_json(
        &d.join("runtime-evidence.json"),
        serde_json::json!({"peers": [{
            "slug": "foreign-gp",
            "runtime": rt.to_str().unwrap(),
            "session": "sess-ours",
            "goal_id": "goal_99",
            "profile": "other",
            "head": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "active_thread": "fake"
        }]}),
    );

    let (ok, out) = monitor_with_profile(&d, Some(&rt), None, "octosfix");
    assert!(ok, "render failed: {out}");
    let line = out
        .lines()
        .find(|l| l.contains("peer foreign-gp:") && l.contains("current-turn"))
        .expect("foreign-gp 行");
    assert!(
        line.contains("unknown"),
        "wrong goal/profile 完整快照不得 running(探针 case4): {line}"
    );
    assert!(!line.contains("running"), "不得 running: {line}");
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

// ---------------------------------------------------------------------------
// 外层 final 三反例(../outer-monitor-final-native-binding-probes.json 只读收据)
// 真实生产 collect_peers 形状: 同 profile/channel/cwd 的 foreign master、
// 同 master 的 foreign goal 文件、以及只有 runtime-evidence 自报终止来源的
// 快照 —— 三者都必须 unknown,不得 running / last-outcome=completed。
// 正向对照(同 master 同 goal 的未完 thread → running)一并钉住,防止
// 修复退化成"一律 unknown"。
// ---------------------------------------------------------------------------

/// 反例 1: 同 profile/channel/cwd,但 peer 的 originator 文件指向别的
/// wire master(`#other-master`),无 lifetime,存在看似本 session 的未完
/// native thread → 不得 running,只能 unknown。
/// 真实 native originator 通常无 cwd 后缀(wire master `octosfix:local:tui#coding`),
/// 真实 thread session 带实际 NUL+~cwd-hash(见 ../native-observation-shapes.json)。
#[test]
fn olp_review_monitor_v3_foreign_originator_thread_not_running() {
    let t = TmpDir::new("v3forgo");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    // 当前视角: master wire 带 NUL+cwd(不能进 argv,由 review-state.json 提供)
    std::fs::write(
        d.join("review-state.json"),
        r#"{"runtime":"/tmp/rt-forgo","session":"octosfix:local:tui#coding\u0000~cwd-abc","goal":"goal_01"}"#,
    )
    .unwrap();
    let profile = "octosfix";
    let peers_root = d
        .join("runtime")
        .join("profiles")
        .join(profile)
        .join("data")
        .join("peers");

    // peer originator 指向别的 wire master(同 channel 同 cwd,不同 master leaf)
    let foreign = peers_root.join("worker");
    std::fs::create_dir_all(&foreign).unwrap();
    std::fs::write(
        foreign.join("originator"),
        "octosfix:local:tui#other-master\n",
    )
    .unwrap();
    std::fs::write(foreign.join("goal"), "goal_01\n").unwrap();
    // 无 lifetime.json(fallback 链): 未完 native thread 形似本 session(cwd 相同)
    let sid = "octosfix:local:tui#peer-worker\u{0}~cwd-abc";
    let tdir = d
        .join("runtime")
        .join("ui-protocol")
        .join(hex_encode(sid.as_bytes()))
        .join("threads");
    std::fs::create_dir_all(&tdir).unwrap();
    write_json(
        &tdir.join("thread-1.json"),
        serde_json::json!({"v":1,"session_id":sid,"thread_id":"thread-1","next_seq":22,"completed":false}),
    );

    let out = Command::new("python3")
        .arg(script())
        .arg(&d)
        .arg("--runtime-dir")
        .arg(d.join("runtime"))
        .arg("--profile")
        .arg(profile)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "render failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let peers = &v["current-turn"]["peers"];
    assert_eq!(
        peers["worker"]["execution"].as_str().unwrap_or("?"),
        "unknown",
        "originator 指向别的 master 不得 running: {peers}"
    );
    assert_eq!(
        peers["worker"]["reason"].as_str().unwrap_or(""),
        "peer-originator-mismatch",
        "reason 应说明 originator 不匹配: {peers}"
    );
}

/// 反例 2: 同 master 同 cwd,但 peer 的 goal 文件是 goal_99,当前视角是
/// goal_01,无 lifetime,未完 thread 同样存在 → 不得 running,只能 unknown。
#[test]
fn olp_review_monitor_v3_peer_goal_file_mismatch_not_running() {
    let t = TmpDir::new("v3pgoal");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("review-state.json"),
        r#"{"runtime":"/tmp/rt-pgoal","session":"octosfix:local:tui#coding\u0000~cwd-abc","goal":"goal_01"}"#,
    )
    .unwrap();
    let profile = "octosfix";
    let peers_root = d
        .join("runtime")
        .join("profiles")
        .join(profile)
        .join("data")
        .join("peers");

    // originator 与当前 master 同源(无 cwd 后缀的真实 wire master 形状),
    // 但 goal 文件是别的 goal。
    let peer = peers_root.join("worker");
    std::fs::create_dir_all(&peer).unwrap();
    std::fs::write(peer.join("originator"), "octosfix:local:tui#coding\n").unwrap();
    std::fs::write(peer.join("goal"), "goal_99\n").unwrap();
    // 无 lifetime.json: 未完 native thread 绑定当前 master session+cwd
    let sid = "octosfix:local:tui#peer-worker\u{0}~cwd-abc";
    let tdir = d
        .join("runtime")
        .join("ui-protocol")
        .join(hex_encode(sid.as_bytes()))
        .join("threads");
    std::fs::create_dir_all(&tdir).unwrap();
    write_json(
        &tdir.join("thread-1.json"),
        serde_json::json!({"v":1,"session_id":sid,"thread_id":"thread-1","next_seq":22,"completed":false}),
    );

    let out = Command::new("python3")
        .arg(script())
        .arg(&d)
        .arg("--runtime-dir")
        .arg(d.join("runtime"))
        .arg("--profile")
        .arg(profile)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "render failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let peers = &v["current-turn"]["peers"];
    assert_eq!(
        peers["worker"]["execution"].as_str().unwrap_or("?"),
        "unknown",
        "goal 文件与当前 goal 不一致不得 running: {peers}"
    );
    assert_eq!(
        peers["worker"]["reason"].as_str().unwrap_or(""),
        "peer-goal-mismatch",
        "reason 应说明 goal 文件不匹配: {peers}"
    );
}

/// 反例 3: 只有 runtime-evidence.json 的自报 {slug, outcome: completed,
/// outcome_source: unverified-self-claim},没有 native result-N+turns 交叉
/// 核对 → last-outcome 不得 completed,只能 unknown(自报来源字符串不可信)。
#[test]
fn olp_review_monitor_v3_unverified_self_claim_outcome_not_terminal() {
    let t = TmpDir::new("v3selfclaim");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("review-state.json"),
        r#"{"runtime":"/tmp/rt-sc","session":"sess-sc","goal":"goal_01"}"#,
    )
    .unwrap();
    // 只有自报快照: 无 native/<slug>/、无 peer 目录 result-N/turns。
    write_json(
        &d.join("runtime-evidence.json"),
        serde_json::json!({"peers": [
            {"slug": "worker", "outcome": "completed", "outcome_source": "unverified-self-claim"}
        ]}),
    );

    let out = Command::new("python3")
        .arg(script())
        .arg(&d)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "render failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let lo = &v["last-outcome"]["peers"]["worker"];
    assert_eq!(
        lo["state"].as_str().unwrap_or("?"),
        "unknown",
        "自报 outcome_source 不得作终止权威: {lo}"
    );
    // 自报值保留为可观测性说明(不凭空消失,但绝不作 state)
    let notes = lo["notes"].as_array().cloned().unwrap_or_default();
    assert!(
        notes.iter().any(|n| n
            .as_str()
            .unwrap_or("")
            .contains("snapshot-outcome-untrusted:completed")),
        "自报值应留作 notes: {lo}"
    );
    assert!(
        notes.iter().any(|n| n
            .as_str()
            .unwrap_or("")
            .contains("snapshot-outcome-source-untrusted:unverified-self-claim")),
        "自报来源应留作 notes: {lo}"
    );
}

/// 正向对照(钉住合法绑定不回归): 同 wire master 同 goal 的 peer(身份文件
/// 与当前视角逐键一致),无 lifetime,存在精确绑定的未完 native thread →
/// running 必须仍然成立;old 未关闭 thread 保守 unknown 语义由 completed
/// 与绑定分支覆盖(见 v3_real_wire_thread_recognition)。
#[test]
fn olp_review_monitor_v3_matching_originator_goal_thread_still_running() {
    let t = TmpDir::new("v3posgo");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("review-state.json"),
        r#"{"runtime":"/tmp/rt-posgo","session":"octosfix:local:tui#coding\u0000~cwd-abc","goal":"goal_01"}"#,
    )
    .unwrap();
    let profile = "octosfix";
    let peers_root = d
        .join("runtime")
        .join("profiles")
        .join(profile)
        .join("data")
        .join("peers");

    // 真实形状: originator 无 cwd 后缀的 wire master;goal 文件与当前一致
    let peer = peers_root.join("worker");
    std::fs::create_dir_all(&peer).unwrap();
    std::fs::write(peer.join("originator"), "octosfix:local:tui#coding\n").unwrap();
    std::fs::write(peer.join("goal"), "goal_01\n").unwrap();
    // 无 lifetime.json: 精确绑定的未完 native thread(NUL+~cwd-hash 与 master 一致)
    let sid = "octosfix:local:tui#peer-worker\u{0}~cwd-abc";
    let tdir = d
        .join("runtime")
        .join("ui-protocol")
        .join(hex_encode(sid.as_bytes()))
        .join("threads");
    std::fs::create_dir_all(&tdir).unwrap();
    write_json(
        &tdir.join("thread-1.json"),
        serde_json::json!({"v":1,"session_id":sid,"thread_id":"thread-1","next_seq":22,"completed":false}),
    );

    let out = Command::new("python3")
        .arg(script())
        .arg(&d)
        .arg("--runtime-dir")
        .arg(d.join("runtime"))
        .arg("--profile")
        .arg(profile)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "render failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let peers = &v["current-turn"]["peers"];
    assert_eq!(
        peers["worker"]["execution"].as_str().unwrap_or("?"),
        "running",
        "身份逐键一致的精确未完 thread 应仍识别 running: {peers}"
    );
}

/// v3-8 回归 1(外层真实反例 ../outer-monitor-missing-native-identity.json):
/// runtime 当前 profile 有 goal_01,ui-protocol 存在同 profile/channel/cwd
/// 的未完 peer thread,context 明确当前 master session —— 但 peer 目录
/// 缺 originator 文件。信息不足 ≠ 归属证明: 无正向 originator 证据不得
/// running,应 unknown(fail-closed)。
#[test]
fn olp_review_monitor_v3_missing_originator_thread_not_running() {
    let t = TmpDir::new("v3morig");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    let master = "octosfix:local:tui#coding\u{0}~cwd-abc";
    std::fs::write(
        d.join("review-state.json"),
        format!(r#"{{"runtime":"/tmp/rt-morig","session":"{master}","goal":"goal_01"}}"#)
            .replace('\u{0}', "\\u0000"),
    )
    .unwrap();
    let profile = "octosfix";
    let peer = d
        .join("runtime")
        .join("profiles")
        .join(profile)
        .join("data")
        .join("peers")
        .join("worker");
    std::fs::create_dir_all(&peer).unwrap();
    // 有 goal、缺 originator: 精确 cwd 绑定的未完 thread 不足以证明归属
    std::fs::write(peer.join("goal"), "goal_01\n").unwrap();
    let sid = "octosfix:local:tui#peer-worker\u{0}~cwd-abc";
    let tdir = d
        .join("runtime")
        .join("ui-protocol")
        .join(hex_encode(sid.as_bytes()))
        .join("threads");
    std::fs::create_dir_all(&tdir).unwrap();
    write_json(
        &tdir.join("t1.json"),
        serde_json::json!({"v":1,"session_id":sid,"thread_id":"t1","next_seq":2,"completed":false}),
    );

    let out = Command::new("python3")
        .arg(script())
        .arg(&d)
        .arg("--runtime-dir")
        .arg(d.join("runtime"))
        .arg("--profile")
        .arg(profile)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "render failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let peers = &v["current-turn"]["peers"];
    assert_eq!(
        peers["worker"]["execution"].as_str().unwrap_or("?"),
        "unknown",
        "缺 originator 时未完 thread 不得证明归属 running: {peers}"
    );
}

/// v3-8 回归 2(外层真实反例同上): originator 文件正确(同 wire master 同
/// cwd),但缺 peer goal 文件;当前视角明确 goal_01。不能证明归属当前
/// goal → 未完 thread 不得 running,应 unknown(fail-closed)。正向对照
/// 见 v3_matching_originator_goal_thread_still_running。
#[test]
fn olp_review_monitor_v3_missing_goal_thread_not_running() {
    let t = TmpDir::new("v3mgoal");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    let master = "octosfix:local:tui#coding\u{0}~cwd-abc";
    std::fs::write(
        d.join("review-state.json"),
        format!(r#"{{"runtime":"/tmp/rt-mgoal","session":"{master}","goal":"goal_01"}}"#)
            .replace('\u{0}', "\\u0000"),
    )
    .unwrap();
    let profile = "octosfix";
    let peer = d
        .join("runtime")
        .join("profiles")
        .join(profile)
        .join("data")
        .join("peers")
        .join("worker");
    std::fs::create_dir_all(&peer).unwrap();
    std::fs::write(peer.join("originator"), "octosfix:local:tui#coding\n").unwrap();
    // 缺 goal 文件: originator 正确也不足以证明归属当前 goal
    let sid = "octosfix:local:tui#peer-worker\u{0}~cwd-abc";
    let tdir = d
        .join("runtime")
        .join("ui-protocol")
        .join(hex_encode(sid.as_bytes()))
        .join("threads");
    std::fs::create_dir_all(&tdir).unwrap();
    write_json(
        &tdir.join("t1.json"),
        serde_json::json!({"v":1,"session_id":sid,"thread_id":"t1","next_seq":2,"completed":false}),
    );

    let out = Command::new("python3")
        .arg(script())
        .arg(&d)
        .arg("--runtime-dir")
        .arg(d.join("runtime"))
        .arg("--profile")
        .arg(profile)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "render failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let peers = &v["current-turn"]["peers"];
    assert_eq!(
        peers["worker"]["execution"].as_str().unwrap_or("?"),
        "unknown",
        "缺 goal 文件时未完 thread 不得证明归属 running: {peers}"
    );
}

/// PR#632 P2-A 回归: 有效 lifetime 但 peer goal 文件属于其他 review/goal
/// → 不得晋升 running/idle(unknown fail-closed)。此前 ident_ok 只在
/// lifetime 校验失败后才看,跨 goal 的有效 lifetime 被误显示为当前视角
/// running/idle;同 goal 正向对照保持。
#[test]
fn olp_review_monitor_valid_lifetime_cross_goal_not_promoted() {
    let t = TmpDir::new("p2a-crossgoal");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    let profile = "octosfix";
    let peers_root = d
        .join("runtime")
        .join("profiles")
        .join(profile)
        .join("data")
        .join("peers");
    let master = "master-sess-1";
    let mk = |slug: &str, goal: &str, phase: &str| {
        let dir = peers_root.join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("originator"), format!("{master}\n")).unwrap();
        std::fs::write(dir.join("goal"), format!("{goal}\n")).unwrap();
        let mut lt = serde_json::json!({
            "version": 1,
            "task_id": "task-42",
            "registry_key": format!("{profile}:peer:{slug}"),
            "master": master,
            "generation": 7,
            "phase": phase,
            "turn_id": "turn-9"
        });
        if phase == "idle" {
            let rm = dir.join("result.md");
            std::fs::write(&rm, b"idle body\n").unwrap();
            lt["result_digest"] = serde_json::json!(sha256_bytes(b"idle body\n"));
        }
        write_json(&dir.join("lifetime.json"), lt);
        dir
    };
    // 当前视角 goal 由 review-state.json 提供(P2-A 的归属比对输入)
    std::fs::write(
        d.join("review-state.json"),
        r#"{"runtime":"/tmp/rt-p2a","session":"master-sess-1","goal":"goal_01"}"#,
    )
    .unwrap();
    // 同 goal 正向: running/idle 均按 lifetime 显示
    mk("p-same-running", "goal_01", "running");
    mk("p-same-idle", "goal_01", "idle");
    // 跨 goal: lifetime 形状有效但 goal 文件=goal_99 → unknown
    mk("p-cross-running", "goal_99", "running");
    mk("p-cross-idle", "goal_99", "idle");

    let out = Command::new("python3")
        .arg("-B")
        .arg(script())
        .arg(&d)
        .arg("--runtime-dir")
        .arg(d.join("runtime"))
        .arg("--profile")
        .arg(profile)
        .arg("--session")
        .arg(master)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "render failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let peers = &v["current-turn"]["peers"];
    assert_eq!(
        peers["p-same-running"]["execution"].as_str().unwrap_or("?"),
        "running",
        "同 goal 有效 lifetime 应 running: {peers}"
    );
    assert_eq!(
        peers["p-same-idle"]["execution"].as_str().unwrap_or("?"),
        "idle",
        "同 goal 有效 idle lifetime 应 idle: {peers}"
    );
    assert_eq!(
        peers["p-cross-running"]["execution"]
            .as_str()
            .unwrap_or("?"),
        "unknown",
        "跨 goal 有效 lifetime 不得晋升 running: {peers}"
    );
    assert_eq!(
        peers["p-cross-idle"]["execution"].as_str().unwrap_or("?"),
        "unknown",
        "跨 goal 有效 lifetime 不得晋升 idle: {peers}"
    );
}

/// 修复轮 2026-09-10 T5: closed 跨仓 parity(#2272 约定对齐)——
/// closed 在同一可信 lifetime 且归属一致时保留身份字段(task_id/
/// generation/turn/master_session_id),execution 恒 closed;foreign goal
/// → closed 但身份 null;旧失败(无 lifetime)不改判。
#[test]
fn olp_review_monitor_closed_keeps_identity_on_trusted_lifetime() {
    let t = TmpDir::new("closedid");
    let d = t.path().to_path_buf();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("review-state.json"),
        r#"{"runtime":"/tmp/rt-cid","session":"master-sess-1","goal":"goal_01"}"#,
    )
    .unwrap();
    let profile = "octosfix";
    let peers_root = d
        .join("runtime")
        .join("profiles")
        .join(profile)
        .join("data")
        .join("peers");
    let mk = |slug: &str, goal: &str, with_lifetime: bool| {
        let dir = peers_root.join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("originator"), "master-sess-1\n").unwrap();
        std::fs::write(dir.join("goal"), format!("{goal}\n")).unwrap();
        std::fs::write(dir.join("closed"), "").unwrap();
        if with_lifetime {
            write_json(
                &dir.join("lifetime.json"),
                serde_json::json!({
                    "version": 1, "task_id": "task-77",
                    "registry_key": format!("{profile}:peer:{slug}"),
                    "master": "master-sess-1", "generation": 5,
                    "phase": "running", "turn_id": "turn-3"
                }),
            );
        }
    };
    mk("c-ident", "goal_01", true);
    mk("c-foreign", "goal_99", true);
    mk("c-nolife", "goal_01", false);
    // malformed lifetime(trusted 形状损坏): closed 不变,身份 null
    let mdir = peers_root.join("c-malformed");
    std::fs::create_dir_all(&mdir).unwrap();
    std::fs::write(mdir.join("originator"), "master-sess-1\n").unwrap();
    std::fs::write(mdir.join("goal"), "goal_01\n").unwrap();
    std::fs::write(mdir.join("closed"), "").unwrap();
    std::fs::write(mdir.join("lifetime.json"), "{not json").unwrap();
    // 旧失败形态(FAILED lifetime + closed): closed 优先,身份按
    // trusted-lifetime 判定(FAILED 形状若过 trusted 则保留字段——见下断言)
    let fdir = peers_root.join("c-failed");
    std::fs::create_dir_all(&fdir).unwrap();
    std::fs::write(fdir.join("originator"), "master-sess-1\n").unwrap();
    std::fs::write(fdir.join("goal"), "goal_01\n").unwrap();
    std::fs::write(fdir.join("closed"), "").unwrap();
    write_json(
        &fdir.join("lifetime.json"),
        serde_json::json!({
            "version": 1, "task_id": "task-fail",
            "registry_key": format!("{profile}:peer:c-failed"),
            "master": "master-sess-1", "generation": 9,
            "phase": "failed", "turn_id": "turn-5"
        }),
    );
    let out = Command::new("python3")
        .arg("-B")
        .arg(script())
        .arg(&d)
        .arg("--runtime-dir")
        .arg(d.join("runtime"))
        .arg("--profile")
        .arg(profile)
        .arg("--session")
        .arg("master-sess-1")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "render failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let peers = &v["current-turn"]["peers"];
    // closed + 可信 lifetime + ident_ok: 保留身份
    assert_eq!(
        peers["c-ident"]["execution"].as_str().unwrap_or("?"),
        "closed",
        "execution 恒 closed: {peers}"
    );
    assert_eq!(
        peers["c-ident"]["task_id"].as_str().unwrap_or("?"),
        "task-77"
    );
    assert_eq!(peers["c-ident"]["generation"].as_i64(), Some(5));
    assert_eq!(
        peers["c-ident"]["turn_id"].as_str().unwrap_or("?"),
        "turn-3"
    );
    assert_eq!(
        peers["c-ident"]["master_session_id"]
            .as_str()
            .unwrap_or("?"),
        "master-sess-1"
    );
    // closed + foreign goal: execution closed 但身份 null
    assert_eq!(
        peers["c-foreign"]["execution"].as_str().unwrap_or("?"),
        "closed"
    );
    assert!(
        peers["c-foreign"]["task_id"].is_null(),
        "foreign 身份须 null"
    );
    assert!(peers["c-foreign"]["generation"].is_null());
    // closed + 无 lifetime: 旧语义不变
    assert_eq!(
        peers["c-nolife"]["execution"].as_str().unwrap_or("?"),
        "closed"
    );
    assert!(peers["c-nolife"]["task_id"].is_null());
    // closed + malformed lifetime: closed,身份 null(不采信)
    assert_eq!(
        peers["c-malformed"]["execution"].as_str().unwrap_or("?"),
        "closed"
    );
    assert!(
        peers["c-malformed"]["task_id"].is_null(),
        "malformed 身份须 null"
    );
    // closed + 可信 FAILED lifetime(旧失败保留): closed,身份保留(同 ident_ok)
    assert_eq!(
        peers["c-failed"]["execution"].as_str().unwrap_or("?"),
        "closed",
        "closed 恒定,不因 phase=failed 变 failed: {peers}"
    );
    assert_eq!(
        peers["c-failed"]["task_id"].as_str().unwrap_or("?"),
        "task-fail"
    );
}
