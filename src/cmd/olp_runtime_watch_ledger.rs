//! Runtime guard 的账本读取模块（#12 子模块，外环黑板 #8）。
//!
//! 只做 ledger JSONL 的增量 scoped 读取；不做进程探测/Herdr/CLI watch 循环/
//! 进度哈希，不接受权威、不产生 task_accepted 裁定。本模块不打印——caller
//! 负责 NDJSON 输出。

use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

/// 每 poll 最多读取的字节数（预算；耗尽时下个 poll 从断点继续）。
const MAX_BYTES_PER_POLL: usize = 4 * 1024 * 1024;
/// 每 poll 最多解析的完整行数（预算）。
const MAX_LINES_PER_POLL: usize = 10_000;
/// 每 poll 最多产生的告警数（预算）。
const MAX_ALERTS_PER_POLL: usize = 1000;
/// 单行字节上限；超限按坏行处理（unknown 一次），不无限累积。
const MAX_LINE_BYTES: usize = 1024 * 1024;
/// 连续多少个"无新增字节"poll 后开始检查同目录后继/报 RotationNeeded。
const IDLE_POLLS_BEFORE_ROTATION_CHECK: u32 = 2;

/// 白名单告警：固定枚举 + 白名单字段；无 reason 原文/output_preview/参数/secret。
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct LedgerAlert {
    pub kind: LedgerAlertKind,
    pub seq: u64,
    pub session: String,
    pub turn: String,
    pub tool_name: Option<String>,
    pub tool_call_id: Option<String>,
    /// 固定错误码枚举，非自由文本。
    pub error_code: Option<LedgerErrorCode>,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LedgerAlertKind {
    ToolFailed,
    TurnError,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LedgerErrorCode {
    ToolReportedFailure,
    TurnFailed,
}

/// turn_state 一旦观察到终态就保持（后续 poll 不回退），供 guard 消费。
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerTurnStateView {
    pub state: LedgerTurnState,
    pub observed_seq: u64,
}

/// 一次 poll 的结构化结果（caller 序列化为 NDJSON 的一行）。
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct LedgerPoll {
    pub path: PathBuf,
    pub byte_offset: u64,
    pub initial_scan: bool,
    pub last_seq: Option<u64>,
    pub alerts: Vec<LedgerAlert>,
    pub turn_state: Option<LedgerTurnStateView>,
    /// 观测通道不可靠的明确说明（坏行/不可读/截短/替换/预算）。
    pub observation_unknown: Option<String>,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LedgerTurnState {
    /// turn_completed 观察到；完成 ≠ 采认。
    CompletionUnverified,
    /// turn_cancelled/turn_aborted 观察到。
    AwaitingReview,
    /// 旧 ledger 停更且无明确后继；沉默不是无进展。
    RotationNeeded,
}

/// 严格白名单反序列化目标（**无** deny_unknown_fields：真实行的 reasoning/
/// output_preview/命令参数等字段直接忽略，绝不输出）。session/turn 可选：
/// 合法但没有 session/turn 的其他通知（如全局通知）按"不属于此 turn"忽略，
/// 不是坏 JSON。
#[derive(Deserialize, Debug)]
struct LedgerRecord {
    seq: u64,
    #[serde(default)]
    event: Option<LedgerEvent>,
}

#[derive(Deserialize, Debug)]
struct LedgerEvent {
    kind: String,
    #[serde(rename = "session_id", default)]
    session_id: Option<String>,
    #[serde(rename = "turn_id", default)]
    turn_id: Option<String>,
    #[serde(default)]
    success: Option<bool>,
    #[serde(rename = "tool_call_id", default)]
    tool_call_id: Option<String>,
    #[serde(rename = "tool_name", default)]
    tool_name: Option<String>,
}

/// 增量游标：生命周期内不重扫整个 ledger；保留未完成行尾巴；scope 水位去重。
pub struct LedgerCursor {
    ledger: PathBuf,
    session: String,
    turn: String,
    /// 已消费到的字节偏移（下一 poll 从此继续）。
    offset: u64,
    /// 当前文件身份（dev,ino）——替换检测。
    identity: Option<(u64, u64)>,
    /// 尚未见换行的部分行字节（跨 poll 保留）。
    partial: Vec<u8>,
    /// 目标 scope 的单调 seq 水位：同 seq 或更小 = 已处理；foreign seq 不抬水位。
    watermark: Option<u64>,
    /// 终态观察（CompletionUnverified/AwaitingReview）。
    terminal: Option<LedgerTurnStateView>,
    first: bool,
    idle_polls: u32,
}

impl LedgerCursor {
    fn parse_record(&mut self, line: &[u8]) -> Result<Option<LedgerAlert>, ParseFailure> {
        let text = std::str::from_utf8(line).map_err(|_| ParseFailure::InvalidUtf8)?;
        let rec: LedgerRecord = serde_json::from_str(text).map_err(|_| ParseFailure::Malformed)?;
        let Some(ev) = rec.event else { return Ok(None) };
        // 严格 scope：session 与 turn 必须同时精确匹配;foreign 一律忽略
        // (合法行,不是坏 JSON,也不抬水位)。
        if ev.session_id.as_deref() != Some(self.session.as_str())
            || ev.turn_id.as_deref() != Some(self.turn.as_str())
        {
            return Ok(None);
        }
        // 单调水位去重:同 seq 或更小 = 已处理;仅目标 scope 抬水位。
        if self.watermark.is_some_and(|w| rec.seq <= w) {
            return Ok(None);
        }
        self.watermark = Some(rec.seq);
        // 闭包捕获克隆值（不借 self），随后才能可变写 terminal/pending。
        let session = self.session.clone();
        let turn = self.turn.clone();
        let tool_name = ev.tool_name.clone();
        let tool_call_id = ev.tool_call_id.clone();
        let make = |kind, err| LedgerAlert {
            kind,
            seq: rec.seq,
            session: session.clone(),
            turn: turn.clone(),
            tool_name: tool_name.clone(),
            tool_call_id: tool_call_id.clone(),
            error_code: err,
        };
        let mut alert = None;
        match ev.kind.as_str() {
            "tool_completed" if ev.success == Some(false) => {
                alert = Some(make(
                    LedgerAlertKind::ToolFailed,
                    Some(LedgerErrorCode::ToolReportedFailure),
                ));
            }
            "turn_error" | "error" | "task_failed" => {
                alert = Some(make(
                    LedgerAlertKind::TurnError,
                    Some(LedgerErrorCode::TurnFailed),
                ));
            }
            "turn_completed" => {
                self.terminal = Some(LedgerTurnStateView {
                    state: LedgerTurnState::CompletionUnverified,
                    observed_seq: rec.seq,
                });
            }
            "turn_cancelled" | "turn_aborted" => {
                self.terminal = Some(LedgerTurnStateView {
                    state: LedgerTurnState::AwaitingReview,
                    observed_seq: rec.seq,
                });
            }
            _ => {}
        }
        Ok(alert)
    }
}

enum ParseFailure {
    Malformed,
    InvalidUtf8,
}

/// 后继查找结果：普通无后继是 **正常 idle**（不是 rotation_needed）；
/// 只有歧义候选或目录读失败才需要上层注意。
enum Succession {
    Idle,
    Ambiguous,
    Found(PathBuf),
    DirError,
}

impl LedgerCursor {
    pub fn new(ledger: PathBuf, session: String, turn: String) -> Self {
        Self {
            ledger,
            session,
            turn,
            offset: 0,
            identity: None,
            partial: Vec::new(),
            watermark: None,
            terminal: None,
            first: true,
            idle_polls: 0,
        }
    }

    /// 同父目录后继发现：只解析合法 `ledger-<数字>-<数字>.log` 文件名
    /// (真实 wire 格式)，选当前文件名之后的最小明确后继。inode 不参与
    /// 排序——它只用于同路径替换检测。
    fn find_successor(&self) -> Succession {
        let Some(parent) = self.ledger.parent() else {
            return Succession::Idle;
        };
        let Some(cur_name) = self.ledger.file_name() else {
            return Succession::Idle;
        };
        let Ok(cur_name) = cur_name.to_str().map(|s| s.to_string()).ok_or(()) else {
            return Succession::Idle;
        };
        let Some(cur_key) = Self::ledger_name_key(&cur_name) else {
            return Succession::Idle;
        };
        let entries = match fs::read_dir(parent) {
            Ok(e) => e,
            Err(_) => return Succession::DirError,
        };
        let mut best: Option<((u64, u64), PathBuf)> = None;
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let p = entry.path();
            if p == self.ledger {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(key) = Self::ledger_name_key(&name) else {
                continue;
            };
            if key <= cur_key {
                continue;
            }
            match &best {
                Some((bk, _)) => {
                    if bk.0 == key.0 && bk.1 != key.1 {
                        return Succession::Ambiguous; // 同 ts 不同 pid 候选
                    }
                    if key < *bk {
                        best = Some((key, p));
                    }
                }
                None => best = Some((key, p)),
            }
        }
        match best {
            Some((_, p)) => Succession::Found(p),
            None => Succession::Idle,
        }
    }

    /// `ledger-<timestamp>-<pid>.log` → (timestamp, pid)；不合法返回 None。
    fn ledger_name_key(name: &str) -> Option<(u64, u64)> {
        let stem = name.strip_prefix("ledger-")?.strip_suffix(".log")?;
        let mut it = stem.split('-');
        let ts: u64 = it.next()?.parse().ok()?;
        let pid: u64 = it.next()?.parse().ok()?;
        if it.next().is_some() {
            return None;
        }
        Some((ts, pid))
    }

    fn unknown(msg: &str, base: &LedgerPoll) -> LedgerPoll {
        LedgerPoll {
            observation_unknown: Some(msg.to_string()),
            ..base.clone()
        }
    }

    /// 增量 poll。不打印；预算耗尽时保持断点，下个 poll 继续。
    pub fn poll(&mut self) -> LedgerPoll {
        let initial_scan = self.first;
        self.first = false;
        let mut out = LedgerPoll {
            path: self.ledger.clone(),
            byte_offset: self.offset,
            initial_scan,
            last_seq: self.watermark,
            alerts: Vec::new(),
            turn_state: self.terminal,
            observation_unknown: None,
        };
        // 文件不存在：可重试的 unknown（部署后文件可能才出现）。
        let meta = match fs::metadata(&self.ledger) {
            Ok(m) => m,
            Err(_) => return LedgerCursor::unknown("ledger_unreadable", &out),
        };
        use std::os::unix::fs::MetadataExt;
        let id = (meta.dev(), meta.ino());
        let len = meta.len();
        // 截短（长度小于已消费 offset）或替换（identity 变化）→ 明确 unknown,
        // 不静默沿用旧 offset。
        if let Some(prev) = self.identity {
            if id != prev {
                self.identity = Some(id);
                self.offset = 0;
                self.partial.clear();
                self.watermark = None;
                return LedgerCursor::unknown("ledger_replaced", &out);
            }
        }
        self.identity = Some(id);
        if len < self.offset {
            return LedgerCursor::unknown("ledger_truncated", &out);
        }
        // #8 边界①:先把上次预算残留的完整行从 partial 里解析出来,
        // 再判 EOF/轮转——否则 len==offset 提前 return 会让剩余记录
        // 永远不被解析(1001 条 fixture 证明跨 poll 不丢)。
        let budget_leftover = !self.partial.is_empty() && self.partial.contains(&b'\n');
        if len == self.offset && !budget_leftover {
            // #8 边界②:普通无新增是正常 idle——只有后继候选歧义或
            // 目录读失败才 unknown/rotation;绝不把第 2 次 idle 当
            // rotation_needed 永久遮蔽后续 stall。
            self.idle_polls += 1;
            if self.idle_polls >= IDLE_POLLS_BEFORE_ROTATION_CHECK {
                match self.find_successor() {
                    Succession::Found(next) => {
                        // watermark 跨正常轮转保留(同 turn seq 去重);
                        // identity 对新文件重新发现。
                        self.ledger = next;
                        self.offset = 0;
                        self.partial.clear();
                        self.idle_polls = 0;
                        self.identity = None;
                        return LedgerCursor::unknown("ledger_rotated_to_sibling", &out);
                    }
                    Succession::Ambiguous => {
                        return LedgerPoll {
                            turn_state: Some(LedgerTurnStateView {
                                state: LedgerTurnState::RotationNeeded,
                                observed_seq: self.watermark.unwrap_or(0),
                            }),
                            observation_unknown: Some("ledger_rotation_ambiguous".into()),
                            ..out
                        };
                    }
                    Succession::DirError => {
                        return LedgerCursor::unknown("ledger_dir_unreadable", &out);
                    }
                    Succession::Idle => {
                        // 目录可读且无合法后继:正常 idle,保持观察。
                        self.idle_polls = 0;
                        return out;
                    }
                }
            }
            return out;
        }
        self.idle_polls = 0;
        if len == self.offset {
            // len==offset 但 partial 里仍有完整行:直接走下面的行解析
            // 路径,只解析 partial(不再读文件)。
        }
        let mut f = match File::open(&self.ledger) {
            Ok(f) => f,
            Err(_) => return LedgerCursor::unknown("ledger_unreadable", &out),
        };
        if f.seek(SeekFrom::Start(self.offset)).is_err() {
            return LedgerCursor::unknown("ledger_unreadable", &out);
        }
        let budget = (len - self.offset).min(MAX_BYTES_PER_POLL as u64) as usize;
        let mut buf = vec![0u8; budget];
        let mut read_total = 0usize;
        while read_total < budget {
            match f.read(&mut buf[read_total..]) {
                Ok(0) => break,
                Ok(n) => read_total += n,
                Err(_) => return LedgerCursor::unknown("ledger_unreadable", &out),
            }
        }
        buf.truncate(read_total);
        self.offset += read_total as u64;
        self.partial.extend_from_slice(&buf);
        // 只解析完整换行行；尾巴留在 partial。
        let mut alerts = Vec::new();
        let mut lines = 0usize;
        let mut unknown: Option<String> = None;
        let mut rest = std::mem::take(&mut self.partial);
        loop {
            if lines >= MAX_LINES_PER_POLL || alerts.len() >= MAX_ALERTS_PER_POLL {
                // 预算耗尽：保留未解析尾巴，下个 poll 继续（不丢数据）。
                self.partial = rest;
                unknown = Some("budget_exhausted".into());
                break;
            }
            let Some(nl) = rest.iter().position(|&b| b == b'\n') else {
                if rest.len() > MAX_LINE_BYTES {
                    // 超限无换行：丢弃该超长"行"，标一次 unknown。
                    rest.clear();
                    unknown = Some("line_too_long".into());
                }
                self.partial = rest;
                break;
            };
            let mut line = rest.split_off(nl + 1);
            std::mem::swap(&mut line, &mut rest);
            line.pop(); // drop '\n'
            lines += 1;
            match self.parse_record(&line) {
                Ok(Some(a)) => {
                    if !alerts.contains(&a) {
                        alerts.push(a);
                    }
                }
                Ok(None) => {}
                Err(ParseFailure::Malformed) => {
                    unknown = Some("malformed_line".into());
                }
                Err(ParseFailure::InvalidUtf8) => {
                    unknown = Some("invalid_utf8_line".into());
                }
            }
        }
        out.byte_offset = self.offset;
        out.last_seq = self.watermark;
        out.alerts = alerts;
        out.turn_state = self.terminal;
        out.observation_unknown = unknown;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "olp-ledger-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn line(seq: u64, kind: &str, session: &str, turn: &str, success: Option<bool>) -> String {
        let succ = match success {
            Some(b) => format!(",\"success\":{b}"),
            None => String::new(),
        };
        format!(
            "{{\"v\":1,\"seq\":{seq},\"event\":{{\"record_kind\":\"notification\",\"kind\":\"{kind}\",\"session_id\":\"{session}\",\"turn_id\":\"{turn}\",\"tool_call_id\":\"tc{seq}\",\"tool_name\":\"bash\"{succ},\"reasoning\":\"MUST_NOT_LEAK\",\"output_preview\":\"MUST_NOT_LEAK\"}}}}\n"
        )
    }

    /// 真实 wire 形:合法通知但 **无 turn_id**(如全局通知)——必须被忽略,
    /// 不是坏 JSON,也不是 foreign 告警。
    fn line_without_turn(seq: u64, session: &str) -> String {
        format!(
            "{{\"v\":1,\"seq\":{seq},\"event\":{{\"record_kind\":\"notification\",\"kind\":\"tool_completed\",\"session_id\":\"{session}\",\"success\":false,\"reasoning\":\"MUST_NOT_LEAK\"}}}}\n"
        )
    }

    #[test]
    fn current_turn_tool_failure_is_observable() {
        let d = tmpdir("toolfail");
        let p = d.join("ledger-1.log");
        std::fs::write(&p, line(1, "tool_completed", "s1", "t1", Some(false))).unwrap();
        let mut c = LedgerCursor::new(p.clone(), "s1".into(), "t1".into());
        let out = c.poll();
        assert_eq!(out.alerts.len(), 1);
        let a = &out.alerts[0];
        assert_eq!(a.kind, LedgerAlertKind::ToolFailed);
        assert_eq!(a.tool_name.as_deref(), Some("bash"));
        assert_eq!(a.error_code, Some(LedgerErrorCode::ToolReportedFailure));
        assert!(!format!("{a:?}").contains("MUST_NOT_LEAK"));
        assert!(out.initial_scan);
        assert_eq!(out.last_seq, Some(1));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn foreign_session_or_turn_no_alerts() {
        let d = tmpdir("foreign");
        let p = d.join("ledger-1.log");
        std::fs::write(
            &p,
            line(1, "tool_completed", "other", "t1", Some(false))
                + &line(2, "tool_completed", "s1", "other-turn", Some(false))
                + &line_without_turn(3, "s1"),
        )
        .unwrap();
        let mut c = LedgerCursor::new(p, "s1".into(), "t1".into());
        let out = c.poll();
        assert!(out.alerts.is_empty());
        assert_eq!(out.last_seq, None); // foreign seq 不抬水位
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn duplicate_seq_deduped_by_monotonic_watermark() {
        let d = tmpdir("dedup");
        let p = d.join("ledger-1.log");
        std::fs::write(
            &p,
            line(1, "tool_completed", "s1", "t1", Some(false))
                + &line(1, "tool_completed", "s1", "t1", Some(false)),
        )
        .unwrap();
        let mut c = LedgerCursor::new(p, "s1".into(), "t1".into());
        let out = c.poll();
        assert_eq!(out.alerts.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn partial_utf8_tail_across_polls() {
        let d = tmpdir("partial");
        let p = d.join("ledger-1.log");
        // 真实多字节切分:payload 带中文,断点选在一个多字节字符 **中间**。
        let full = "{\"v\":1,\"seq\":1,\"event\":{\"record_kind\":\"notification\",\"kind\":\"tool_completed\",\"session_id\":\"s1\",\"turn_id\":\"t1\",\"tool_call_id\":\"tc1\",\"tool_name\":\"bash\",\"success\":false,\"output_preview\":\"中文采集边界测试\"}}\n";
        let bytes = full.as_bytes();
        // 找一个 UTF-8 续字节位置切(0b10xxxxxx),保证劈开多字节字符。
        let mut cut = bytes.len() / 2;
        while cut < bytes.len() && (bytes[cut] & 0xC0) == 0x80 {
            cut += 1;
        }
        cut = cut.saturating_sub(1).max(1); // 落回上一个字节 → 劈开该字符
        std::fs::write(&p, &bytes[..cut]).unwrap();
        let mut c = LedgerCursor::new(p.clone(), "s1".into(), "t1".into());
        let out1 = c.poll();
        assert!(out1.alerts.is_empty());
        // append 写剩余字节(不重写整文件)。
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        use std::io::Write;
        f.write_all(&bytes[cut..]).unwrap();
        let out2 = c.poll();
        assert_eq!(out2.alerts.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn complete_bad_line_unknown_then_recovers() {
        let d = tmpdir("badline");
        let p = d.join("ledger-1.log");
        std::fs::write(&p, "{not json}\n").unwrap();
        let mut c = LedgerCursor::new(p.clone(), "s1".into(), "t1".into());
        let out1 = c.poll();
        assert_eq!(out1.observation_unknown.as_deref(), Some("malformed_line"));
        // 恢复:追加好行,poll 不无限吃同一坏行
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        use std::io::Write;
        f.write_all(line(5, "tool_completed", "s1", "t1", Some(false)).as_bytes())
            .unwrap();
        let out2 = c.poll();
        assert_eq!(out2.observation_unknown, None);
        assert_eq!(out2.alerts.len(), 1);
        assert_eq!(out2.last_seq, Some(5));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn truncate_replace_and_read_failure_unknown() {
        let d = tmpdir("trunc");
        let p = d.join("ledger-1.log");
        std::fs::write(&p, line(1, "tool_completed", "s1", "t1", Some(false))).unwrap();
        let mut c = LedgerCursor::new(p.clone(), "s1".into(), "t1".into());
        let _ = c.poll();
        // 截短
        std::fs::write(&p, "short\n").unwrap();
        let out = c.poll();
        assert!(out.observation_unknown.is_some());
        // 替换(新 inode)
        let q = d.join("ledger-2.log");
        std::fs::write(&q, line(2, "turn_error", "s1", "t1", None)).unwrap();
        std::fs::remove_file(&p).unwrap();
        std::fs::rename(&q, &p).unwrap();
        let out = c.poll();
        assert!(out.observation_unknown.is_some());
        // 不存在的文件
        let mut c2 = LedgerCursor::new(d.join("nope.log"), "s1".into(), "t1".into());
        assert!(c2.poll().observation_unknown.is_some());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn sibling_successor_continues() {
        let d = tmpdir("rotate");
        // 真实文件名格式:ledger-<timestamp>-<pid>.log。
        let p1 = d.join("ledger-1695000000-111.log");
        std::fs::write(&p1, line(1, "tool_completed", "s1", "t1", Some(false))).unwrap();
        let mut c = LedgerCursor::new(p1, "s1".into(), "t1".into());
        let out1 = c.poll();
        assert_eq!(out1.alerts.len(), 1);
        // 新后继出现,旧文件停更
        let p2 = d.join("ledger-1695000100-222.log");
        std::fs::write(&p2, line(2, "tool_completed", "s1", "t1", Some(false))).unwrap();
        let _ = c.poll(); // idle 1
        let out = c.poll(); // idle 2 -> rotation check
        assert!(out.observation_unknown.is_some() || out.turn_state.is_some());
        // 再 poll 应已切到后继并读到新事件
        let out3 = c.poll();
        assert!(out3.path.ends_with("ledger-1695000100-222.log"));
        assert_eq!(out3.alerts.len(), 1);
        assert_eq!(out3.last_seq, Some(2));
        let _ = std::fs::remove_dir_all(&d);
    }
}
