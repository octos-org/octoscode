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
    /// #9:初始积压边界——首次成功打开常规文件时的长度。积压未消费完
    /// 期间 initial_scan 恒为 true(历史读入不是新进展);消费完后的
    /// 追加才以 initial_scan=false 报告。
    initial_backlog_end: Option<u64>,
    /// #9:超长行丢弃状态。Some(discarded_bytes) 表示正丢弃字节直到
    /// 换行;期间任何"恰好合法"的尾巴都不是新事件。
    discarding_oversize: Option<u64>,
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
            initial_backlog_end: None,
            discarding_oversize: None,
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
        let mut out = LedgerPoll {
            path: self.ledger.clone(),
            byte_offset: self.offset,
            initial_scan: false, // 真值在成功打开并定积压边界后写入
            last_seq: self.watermark,
            alerts: Vec::new(),
            turn_state: self.terminal,
            observation_unknown: None,
        };
        // #9 历史标记:initial_scan = 首次尚未完成 或 初始积压未消费完。
        // 读失败不消耗 first;只在首次成功打开并记录积压边界后推进。
        let mut initial_scan = self.first;
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
        // 首次成功打开常规文件:记录积压边界,消耗 first。
        if self.first {
            self.initial_backlog_end = Some(len);
            self.first = false;
        }
        if let Some(backlog_end) = self.initial_backlog_end {
            initial_scan = self.offset < backlog_end || self.first;
        }
        if len < self.offset {
            // #9 截短恢复:安全重置游标,给一拍 unknown,watermark 保留
            // (目标 scope seq 去重,不重复告警旧 seq),下一 poll 恢复读取。
            self.offset = 0;
            self.partial.clear();
            self.discarding_oversize = None;
            self.initial_backlog_end = Some(len);
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
        // #9 fd 身份核对:对实际打开的 fd 取 metadata,不能旧路径 meta
        // 配新 fd 静默读取。
        let fd_meta = match f.metadata() {
            Ok(m) => m,
            Err(_) => return LedgerCursor::unknown("ledger_unreadable", &out),
        };
        let fd_id = (fd_meta.dev(), fd_meta.ino());
        if self.identity.is_some_and(|prev| prev != fd_id) {
            return LedgerCursor::unknown("ledger_replaced", &out);
        }
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
        // #9 预算与内存:partial 有界——已缓冲完整行先消化,再补读;
        // partial 长度上限 = MAX_BYTES_PER_POLL + MAX_LINE_BYTES,持续
        // 追加压力下不会无限增长。
        self.offset += read_total as u64;
        self.partial.extend_from_slice(&buf);
        out.initial_scan = initial_scan;
        // 只解析完整换行行；尾巴留在 partial。
        let mut alerts = Vec::new();
        let mut lines = 0usize;
        let mut unknown: Option<String> = None;
        let mut consumed = 0usize; // #9: 已消费索引,一次 drain,无逐行大复制
        loop {
            let rest = &self.partial[consumed..];
            if lines >= MAX_LINES_PER_POLL || alerts.len() >= MAX_ALERTS_PER_POLL {
                // 预算耗尽：保留未解析尾巴，下个 poll 继续（不丢数据）。
                unknown = Some("budget_exhausted".into());
                break;
            }
            let Some(nl) = rest.iter().position(|&b| b == b'\n') else {
                // 无换行尾巴。
                if let Some(discarded) = self.discarding_oversize.as_mut() {
                    // #9 丢弃态:吞掉这些字节,不是事件。
                    *discarded += rest.len() as u64;
                    consumed = self.partial.len();
                    continue;
                }
                if rest.len() > MAX_LINE_BYTES {
                    // 超限无换行：进入丢弃态,丢弃字节直到换行。
                    self.discarding_oversize = Some(rest.len() as u64);
                    consumed = self.partial.len();
                    unknown = Some("line_too_long".into());
                }
                break;
            };
            let line = &rest[..nl];
            if self.discarding_oversize.is_some() {
                // 丢弃态遇到换行:该超长行(含此前全部尾巴)整体作废,
                // 从换行之后重新开始;后续合法行正常恢复。
                let d = self.discarding_oversize.take().unwrap();
                let _ = d;
                consumed += nl + 1;
                continue;
            }
            if line.len() > MAX_LINE_BYTES {
                // 换行分隔的超长行:同样限长作废,不解析。
                self.discarding_oversize = Some(0);
                consumed += nl + 1;
                unknown = Some("line_too_long".into());
                continue;
            }
            consumed += nl + 1;
            lines += 1;
            // 行内容先拷入有界缓冲(≤MAX_LINE_BYTES),释放 partial 借用,
            // parse_record 才能可变借用 self。
            let owned = line.to_vec();
            match self.parse_record(&owned) {
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
        // #9:一次 drain 已消费前缀(非逐行复制)。
        let _ = self.partial.drain(..consumed);
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
    use std::path::Path;

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

    /// 真实错误 payload 形状:含 quota/permission message、reasoning、
    /// output_preview 与 secret 哨兵——输出必须是固定白名单告警,
    /// 绝不含这些原文。
    fn error_line(seq: u64, kind: &str, session: &str, turn: &str) -> String {
        format!(
            "{{\"v\":1,\"seq\":{seq},\"event\":{{\"record_kind\":\"notification\",\"kind\":\"{kind}\",\"session_id\":\"{session}\",\"turn_id\":\"{turn}\",\"message\":\"quota exceeded quota_exceeded::context_limit\",\"reason\":\"permission denied /Users/secret/path\",\"reasoning\":\"SECRET_REASONING_SENTINEL\",\"output_preview\":\"SECRET_PREVIEW_SENTINEL\",\"api_key\":\"sk-SECRET-SENTINEL\"}}}}\n"
        )
    }

    fn append(p: &Path, s: &str) {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(p).unwrap();
        f.write_all(s.as_bytes()).unwrap();
    }

    /// #9-1/2: 1001 条真实 tool 失败跨 poll 无丢失,initial 积压未消费完
    /// 期间 initial_scan=true,真实 append 后变 false。
    #[test]
    fn thousand_one_failures_across_polls_and_initial_scan_semantics() {
        let d = tmpdir("budget1001");
        let p = d.join("ledger-1695000000-111.log");
        let mut all = String::new();
        for seq in 1..=1001u64 {
            all.push_str(&line(seq, "tool_completed", "s1", "t1", Some(false)));
        }
        std::fs::write(&p, &all).unwrap();
        let mut c = LedgerCursor::new(p.clone(), "s1".into(), "t1".into());
        let mut total = 0usize;
        let mut polls = 0;
        let mut saw_false = false;
        loop {
            let out = c.poll();
            total += out.alerts.len();
            polls += 1;
            if polls > 50 {
                panic!("runaway polls");
            }
            if out.initial_scan {
                assert!(!saw_false, "initial_scan must not flip back to true");
            } else {
                saw_false = true;
            }
            if total >= 1001 && out.observation_unknown.is_none() {
                break;
            }
        }
        assert_eq!(total, 1001, "all failures across polls, no loss/dup");
        // 消费完积压后的真实 append → initial_scan=false 的新事件。
        append(&p, &line(1002, "tool_completed", "s1", "t1", Some(false)));
        let out = c.poll();
        assert!(!out.initial_scan, "post-backlog append is new progress");
        assert_eq!(out.alerts.len(), 1);
        assert_eq!(out.last_seq, Some(1002));
        // 内存界限:持续追加压力下 partial 有固定上限。
        let max_partial = MAX_BYTES_PER_POLL + MAX_LINE_BYTES;
        assert!(c.partial.len() <= max_partial);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// #9-1: 初始积压跨 **多个** poll 期间 initial_scan 恒 true
    /// (历史读入不是新进展),4MiB 跨界由上一测的 1001 条 ×行宽覆盖。
    #[test]
    fn initial_backlog_spans_polls_marks_history() {
        let d = tmpdir("backlog2");
        let p = d.join("ledger-1695000000-111.log");
        let mut all = String::new();
        // > MAX_ALERTS_PER_POLL 条 → 必然跨 poll。
        for seq in 1..=2100u64 {
            all.push_str(&line(seq, "tool_completed", "s1", "t1", Some(false)));
        }
        std::fs::write(&p, &all).unwrap();
        let mut c = LedgerCursor::new(p.clone(), "s1".into(), "t1".into());
        let mut total = 0usize;
        let mut polls = 0;
        while total < 2100 {
            let out = c.poll();
            polls += 1;
            if polls > 50 {
                panic!("runaway");
            }
            assert!(
                out.initial_scan || total > 0,
                "backlog phase must stay initial_scan=true"
            );
            total += out.alerts.len();
        }
        assert!(polls >= 3, "2100 failures must span multiple polls");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// #9-3: 未终止超长行进入 discard-until-newline 态;后续"恰好合法
    /// JSON"的尾巴不是事件;换行后新合法行正常恢复。分段 append 真实文件。
    #[test]
    fn oversize_line_discards_until_newline_then_recovers() {
        let d = tmpdir("oversize");
        let p = d.join("ledger-1695000000-111.log");
        // 未终止超长行:合法 JSON 开头 + 大 padding,无换行。
        let mut huge = String::from(
            "{\"v\":1,\"seq\":1,\"event\":{\"record_kind\":\"notification\",\"kind\":\"tool_completed\",\"session_id\":\"s1\",\"turn_id\":\"t1\",\"success\":false,\"padding\":\"",
        );
        huge.push_str(&"x".repeat(MAX_LINE_BYTES + 4096));
        huge.push_str("\"}");
        let mut c = LedgerCursor::new(p.clone(), "s1".into(), "t1".into());
        // 分段 1:超长行前半。
        std::fs::write(&p, huge.as_bytes()).unwrap();
        let out1 = c.poll();
        assert_eq!(out1.alerts.len(), 0);
        assert_eq!(out1.observation_unknown.as_deref(), Some("line_too_long"));
        // 分段 2:伪装成合法 JSON 的尾巴 + 换行 + 新合法行。
        let decoy = format!(
            "{}\n{}",
            line(99, "tool_completed", "s1", "t1", Some(false)),
            line(2, "tool_completed", "s1", "t1", Some(false))
        );
        // 注意:decoy 的 seq=99 在超长行内部,必须被丢弃态吞掉,不产生告警;
        // seq=2 是换行后的新合法行,必须恢复。
        append(&p, &decoy);
        let out2 = c.poll();
        // 若实现在此 poll 仍在丢弃,再 poll 一次消化(确定性:两 poll 内完成)。
        let out3 = c.poll();
        let alerts: Vec<_> = out2.alerts.iter().chain(out3.alerts.iter()).collect();
        assert!(
            alerts.iter().all(|a| a.seq != 99),
            "discarded oversize interior must never alert: {:?}",
            alerts
        );
        assert!(
            alerts.iter().any(|a| a.seq == 2),
            "post-newline valid line must recover: {:?}",
            alerts
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// #9-4: 截短→一拍 unknown→重置后恢复读取,目标 scope watermark 保留,
    /// 旧 seq 重写不重复告警;boundary duplicate 只发一次。
    #[test]
    fn truncate_resets_once_and_dedups_old_seqs() {
        let d = tmpdir("trunc9");
        let p = d.join("ledger-1695000000-111.log");
        std::fs::write(
            &p,
            line(1, "tool_completed", "s1", "t1", Some(false))
                + &line(2, "tool_completed", "s1", "t1", Some(false)),
        )
        .unwrap();
        let mut c = LedgerCursor::new(p.clone(), "s1".into(), "t1".into());
        let out1 = c.poll();
        assert_eq!(out1.alerts.len(), 2);
        // 截短成更短的历史(旧 seq 重写)。
        std::fs::write(&p, line(1, "tool_completed", "s1", "t1", Some(false))).unwrap();
        let out2 = c.poll();
        assert_eq!(
            out2.observation_unknown.as_deref(),
            Some("ledger_truncated")
        );
        assert_eq!(out2.alerts.len(), 0, "one-beat unknown, no re-read yet");
        let out3 = c.poll(); // 恢复读取,但 seq=1 已在水位下 → 不重复。
        assert_eq!(out3.observation_unknown, None);
        assert_eq!(
            out3.alerts.len(),
            0,
            "old seq must not re-alert (watermark kept)"
        );
        // 追加 boundary duplicate(seq=2 重写)与 seq=3 新事件。
        append(
            &p,
            &(line(2, "tool_completed", "s1", "t1", Some(false))
                + &line(3, "tool_completed", "s1", "t1", Some(false))),
        );
        let out4 = c.poll();
        assert_eq!(
            out4.alerts.len(),
            1,
            "seq=2 dup once more at boundary? no—watermark=2; only seq=3"
        );
        assert_eq!(out4.alerts[0].seq, 3);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// #9-5: 隐私——quota/permission 形状 payload 经 **Serialize** 输出,
    /// 不含原 message/reason/reasoning/output_preview/api_key 哨兵;
    /// 固定错误码 turn_failed。不能只查 Debug。
    #[test]
    fn serialize_output_never_leaks_error_payload() {
        let d = tmpdir("privacy");
        let p = d.join("ledger-1695000000-111.log");
        std::fs::write(
            &p,
            error_line(1, "turn_error", "s1", "t1") + &error_line(2, "error", "s1", "t1"),
        )
        .unwrap();
        let mut c = LedgerCursor::new(p, "s1".into(), "t1".into());
        let out = c.poll();
        assert_eq!(out.alerts.len(), 2);
        let json = serde_json::to_string(&out).unwrap();
        for sentinel in [
            "quota exceeded",
            "quota_exceeded::context_limit",
            "permission denied",
            "SECRET_REASONING_SENTINEL",
            "SECRET_PREVIEW_SENTINEL",
            "sk-SECRET-SENTINEL",
        ] {
            assert!(
                !json.contains(sentinel),
                "serialized poll leaked payload: {sentinel}"
            );
        }
        assert!(json.contains("turn_error"));
        assert!(json.contains("turn_failed"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// #9-5: 终态——turn_completed/取消仅观察态;普通 idle 不抹终态;
    /// Done/ACK 不构成任何采认;未知 kind 合法忽略,不当进度。
    #[test]
    fn terminal_states_persist_across_idle_and_done_means_nothing() {
        let d = tmpdir("terminal");
        let p = d.join("ledger-1695000000-111.log");
        std::fs::write(
            &p,
            line(1, "turn_completed", "s1", "t1", None)
                + &line(2, "turn_cancelled", "s1", "t1", None),
        )
        .unwrap();
        let mut c = LedgerCursor::new(p.clone(), "s1".into(), "t1".into());
        let out1 = c.poll();
        assert_eq!(
            out1.turn_state.map(|t| t.state),
            Some(LedgerTurnState::AwaitingReview)
        );
        let out2 = c.poll();
        assert_eq!(
            out2.turn_state.map(|t| t.state),
            Some(LedgerTurnState::AwaitingReview),
            "plain idle must not erase terminal state"
        );
        assert_eq!(out2.observation_unknown, None);
        append(&p, &line(3, "ack_recorded", "s1", "t1", None));
        let out3 = c.poll();
        assert_eq!(
            out3.turn_state.map(|t| t.state),
            Some(LedgerTurnState::AwaitingReview)
        );
        assert!(out3.alerts.is_empty());
        assert_eq!(
            out3.last_seq,
            Some(3),
            "unknown kind still advances seq watermark"
        );
        append(&p, &line(4, "turn_completed", "s1", "t1", None));
        let out4 = c.poll();
        assert!(out4.turn_state.is_some());
        let _ = std::fs::remove_dir_all(&d);
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
