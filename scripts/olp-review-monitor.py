#!/usr/bin/env python3
"""olp-review-monitor — Herdr pane 可读监控入口.

Spec: specs/task-evo-review-evidence.spec.md (review-monitor rules)
渲染四区块: lifecycle / current-turn / last-outcome / deliverables。
runtime 复合身份(runtime+session+goal+profile,非裸 goal_01)。

只读纪律(fail-closed):
  * review-state.json 只读,任何渲染(含损坏时)不改其字节。
  * ACK 基线缓存在自家 monitor-state.json,原子替换,JSON key 类型一致,
    按内容 hash 追踪(宽松子串匹配,不认行号/前缀),支持新增/原地改/删除。
  * 不 rglob 取首个 lifetime.json;只认
    <runtime>/profiles/<profile>/data/peers/<slug>/lifetime.json 精确路径,
    符号链接不采信。
  * lifetime 严格校验: version==1 && task_id 非空 &&
    registry_key=='<profile>:peer:<slug>' && originator 文件==master &&
    (--session 给定时 master==session) && writer-shape turn 绑定;
    Idle 还须实算 SHA256(result.md)==result_digest。任一失败 → unknown。
  * 无 CURRENT authority 且无精确活跃 thread → unknown(即使旧 completed)。
  * peer 身份绑定: thread/快照 fallback 前先比对 peer 自身身份文件 ——
    originator 文件必须与当前 wire master 同源(真实 native originator 通常
    无 cwd 后缀,如 `octosfix:local:tui#coding`;真实 thread session 带
    NUL+~cwd-hash),goal 文件必须与当前 goal 一致;矛盾 → unknown,不晋升。
  * 快照自报 outcome/outcome_source 不是终止权威 —— 无 native result-N+turns
    交叉核对时 last-outcome=unknown(自报值仅作 notes 说明)。
  * ui-protocol next_seq(流事件序号)与 result-N turn(轮次计数)不同量纲,
    禁止数值比较。
  * last-outcome 恒取终止证据(result-N.md + turns.txt 交叉核对),
    坏行/矛盾/缺失 → unknown 但保留来源说明;旧失败不变成成功;四种真实
    终止 outcome(completed/errored/interrupted/rate_limited)分层如实显示。
  * events/supervisor 负向事件(goal_transition blocked / escalation)只从
    本 runtime 的 events.jsonl 归属,不吞并其他 runtime。
  * 官方 olp-watch-board.sh 正哨协议不改,板文件只读。
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import tempfile
import time
from pathlib import Path

PROTOCOL = "olp-review-monitor/v1"
DEFAULT_PROFILE = "octos"

_FM_RE = re.compile(r"\A---\s*\n(.*?)\n---\s*\n", re.S)
_ACK_RE = re.compile(r"ACK\(([^)]*)\)")
_RESULT_RE = re.compile(r"result-(\d+)\.md")

KNOWN_PHASES = {"pending", "running", "idle", "failed"}

# 真实 native writer 的四种终止 outcome —— last-outcome 层如实显示(不折叠)。
# 非终止态(pending/running)与伪造值(fabricated 等)→ unknown/拒绝。
# 注意: review-freeze 准入层只认 completed(另一层,见反例 3),与本层分开。
_KNOWN_TERMINAL_OUTCOMES = {"completed", "errored", "interrupted", "rate_limited"}


def parse_frontmatter(path: Path) -> dict:
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return {}
    m = _FM_RE.match(text)
    fm: dict = {}
    if m:
        for line in m.group(1).splitlines():
            if ":" in line:
                k, _, v = line.partition(":")
                fm[k.strip()] = v.strip()
    return fm


def sha256_text(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _read_regular_file(path: Path) -> str | None:
    """fd-anchored read; returns None for symlink/missing/non-regular files."""
    try:
        if path.is_symlink():
            return None
        st = path.stat()
        if not st or not path.is_file():
            return None
        return path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None


# ---------------------------------------------------------------------------
# identity: composite runtime identity (never bare goal id)
# ---------------------------------------------------------------------------


def composite_identity(
    runtime: str,
    session: str | None,
    goal: str | None,
    profile: str | None,
    master: str | None = None,
) -> str:
    parts = [runtime or "runtime:unknown"]
    if profile:
        parts.append(f"profile:{profile}")
    if session:
        parts.append(f"session:{session}")
    if master and master != session:
        parts.append(f"master:{master}")
    if goal:
        parts.append(f"goal:{goal}")
    return " | ".join(parts)


# ---------------------------------------------------------------------------
# review-state.json — read-only, never mutated (even when corrupt)
# ---------------------------------------------------------------------------


def read_review_state(review_dir: Path) -> tuple[dict, bool]:
    """Returns (state, corrupt). Never writes review-state.json."""
    state_file = review_dir / "review-state.json"
    if not state_file.exists():
        return {}, False
    raw = _read_regular_file(state_file)
    if raw is None:
        return {"__unreadable__": True}, True
    try:
        state = json.loads(raw)
    except json.JSONDecodeError:
        return {"__corrupt__": True}, True
    if not isinstance(state, dict):
        return {"__corrupt__": True}, True
    return state, False


# ---------------------------------------------------------------------------
# monitor-state.json — own ACK cache, atomic replace, consistent JSON keys
# ---------------------------------------------------------------------------


def read_monitor_state(review_dir: Path) -> tuple[dict, str | None]:
    """Own cache; never read from review-state.json."""
    cache = review_dir / "monitor-state.json"
    if not cache.exists():
        return {}, None
    raw = _read_regular_file(cache)
    if raw is None:
        return {}, "monitor-state.json unreadable"
    try:
        data = json.loads(raw)
    except json.JSONDecodeError:
        return {}, "monitor-state.json corrupt (keeping evidence, starting fresh)"
    if not isinstance(data, dict):
        return {}, "monitor-state.json wrong shape"
    return data, None


def write_monitor_state_atomic(review_dir: Path, state: dict) -> str | None:
    """Atomic replace (tmp + os.replace). Returns diagnostic on failure."""
    cache = review_dir / "monitor-state.json"
    tmp = None
    try:
        fd, tmp = tempfile.mkstemp(
            prefix=".monitor-state-", suffix=".tmp", dir=str(review_dir)
        )
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(json.dumps(state, ensure_ascii=False, indent=1) + "\n")
        os.replace(tmp, cache)
        return None
    except OSError as e:
        if tmp:
            try:
                os.unlink(tmp)
            except OSError:
                pass
        return f"monitor-state.json write failed: {e}"


# ---------------------------------------------------------------------------
# lifetime authority — strict fail-closed projection
# ---------------------------------------------------------------------------


def trusted_lifetime(
    peer_dir: Path, profile: str, slug: str, session: str | None
) -> tuple[dict | None, str]:
    """Trusted read-only projection of lifetime.json.

    Returns (record, reason). record is None when ANY fail-closed check
    fails; reason explains why (for observability).
    """
    lt_path = peer_dir / "lifetime.json"
    if not lt_path.exists():
        return None, "no-lifetime-json"
    if lt_path.is_symlink():
        return None, "lifetime-json-is-symlink"
    raw = _read_regular_file(lt_path)
    if raw is None:
        return None, "lifetime-json-unreadable"
    try:
        rec = json.loads(raw)
    except json.JSONDecodeError:
        return None, "lifetime-json-corrupt"
    if not isinstance(rec, dict):
        return None, "lifetime-json-wrong-shape"
    # version==1 strictly (unknown versions untrusted)
    if rec.get("version") != 1:
        return None, f"lifetime-version-untrusted:{rec.get('version')!r}"
    task_id = rec.get("task_id")
    if not isinstance(task_id, str) or not task_id:
        return None, "lifetime-empty-task-id"
    if rec.get("registry_key") != f"{profile}:peer:{slug}":
        return None, "lifetime-registry-key-mismatch"
    master = rec.get("master")
    if not isinstance(master, str) or not master:
        return None, "lifetime-empty-master"
    # originator leaf must equal master
    orig_raw = _read_regular_file(peer_dir / "originator")
    if orig_raw is None or orig_raw.strip() != master:
        return None, "lifetime-originator-mismatch"
    # explicit session binding (防跨 runtime/同 slug 串线)
    if session and master != session:
        return None, "lifetime-master-not-current-session"
    phase = rec.get("phase")
    if phase not in KNOWN_PHASES:
        return None, f"lifetime-phase-untrusted:{phase!r}"
    turn_id = rec.get("turn_id")
    # writer-shape turn binding (v3-1): 真实 writer 的 invalidate/finish
    # (has_queued_input=true) 保留 Some(old turn_id) 的 Pending —— 合法下一轮
    # queued。Pending 允许 turn_id 为 None 或非空合法 string;拒绝空串/错类型。
    if phase == "pending":
        if turn_id is not None and (not isinstance(turn_id, str) or not turn_id):
            return None, "lifetime-pending-bad-turn-id"
    else:
        if not isinstance(turn_id, str) or not turn_id:
            return None, f"lifetime-{phase}-without-turn-id"
    generation = rec.get("generation")
    if not isinstance(generation, int) or isinstance(generation, bool):
        return None, "lifetime-generation-wrong-type"
    result_digest = rec.get("result_digest")
    if result_digest is not None and not isinstance(result_digest, str):
        return None, "lifetime-result-digest-wrong-type"
    if phase == "idle":
        if not result_digest:
            return None, "lifetime-idle-empty-digest"
        result_path = peer_dir / "result.md"
        if result_path.is_symlink():
            return None, "lifetime-idle-result-symlink"
        try:
            body = result_path.read_bytes()
        except OSError:
            return None, "lifetime-idle-result-missing"
        if sha256_bytes(body) != result_digest:
            return None, "lifetime-idle-digest-mismatch"
    trusted = {
        "task_id": task_id,
        "registry_key": rec["registry_key"],
        "master": master,
        "generation": generation,
        "phase": phase,
        "turn_id": turn_id,
        "result_digest": result_digest,
    }
    return trusted, "trusted"


PHASE_TO_EXECUTION = {
    "pending": "queued",
    "running": "running",
    "failed": "failed",
    "idle": "idle",
}


# ---------------------------------------------------------------------------
# runtime-evidence — per-peer precise binding (never broadcast)
# ---------------------------------------------------------------------------

# v3-4: 完整复合身份键(runtime/session/goal/peer/turn/HEAD)。snapshot 缺任一
# 必填键即 unknown。键别名兼容真实 writer 的 goal_id / commit 命名。
_ID_REQUIRED = {
    "runtime": ("runtime",),
    "session": ("session", "session_id", "master"),
    "goal": ("goal", "goal_id"),
    "peer": ("peer", "slug"),
    "head": ("head", "HEAD", "commit"),
}


def _identity_complete_strict(re_peer: dict) -> bool:
    """内部用: 严格完整复合身份判定(见 `_identity_incomplete` 后定义)。"""
    return not _identity_incomplete(re_peer)


def _snapshot_identity_incomplete(re_peer: dict) -> bool:
    """带复合身份键的快照缺任一身份维度 → unknown。

    只有 slug 的裸快照(旧 writer/测试形状)不算 incomplete —— 由调用方按
    精确绑定另行把关;一旦 snapshot 携带 runtime/session/goal/peer/turn/head
    任一复合身份键,全部维度都必须是非空字符串,否则视为残缺/串线证据。
    """
    identity_keys = {k for aliases in _ID_REQUIRED.values() for k in aliases} - {"slug"}
    if not any(re_peer.get(k) is not None for k in identity_keys):
        return False
    return _identity_incomplete(re_peer)


def _snapshot_bound_here(
    re_peer: dict,
    runtime_dir: Path | None,
    session: str | None,
    goal: str | None = None,
    profile: str | None = None,
) -> bool:
    """snapshot 是否精确归属当前 runtime/session 视角(active_thread 逐 peer 绑定)。

    foreign 快照(显式 runtime/session 与当前视角矛盾)永不采信 —— 即使带
    active_thread 字符串也不得 running。带身份键的快照必须完整且逐键不
    矛盾。裸快照(无复合身份键)一律 unknown(外层裁决 #4)—— 即使在已知
    runtime/session 视角下也不构成完整身份权威,不得凭视角上下文晋升为
    running;只有完整身份快照或精确线程/原生文件绑定才可供来源判定。
    """
    if _snapshot_identity_incomplete(re_peer):
        return False
    snap_rt = re_peer.get("runtime")
    if isinstance(snap_rt, str) and snap_rt and runtime_dir is not None:
        if snap_rt != str(runtime_dir):
            return False
    snap_sess = next(
        (re_peer.get(k) for k in ("session", "session_id", "master")
         if re_peer.get(k) is not None),
        None,
    )
    if isinstance(snap_sess, str) and snap_sess and session:
        if snap_sess != session:
            return False
    # 外层裁决: goal / profile 逐键不矛盾 —— 快照显式携带的 goal/profile
    # 与当前视角不符 → 不采信(即使完整身份+active_thread 也不得 running)。
    snap_goal = next(
        (re_peer.get(k) for k in ("goal", "goal_id") if re_peer.get(k) is not None),
        None,
    )
    if isinstance(snap_goal, str) and snap_goal and goal:
        if snap_goal != goal:
            return False
    snap_profile = re_peer.get("profile")
    if isinstance(snap_profile, str) and snap_profile and profile:
        if snap_profile != profile:
            return False
    # v3-4: 裸快照(无复合身份键)只有在当前视角本身可被精确锚定
    # (runtime/session 已知)时才可采信 —— 否则 active_thread 无归属,
    # 不得 running(v3-1 反广播)。
    # 任何"只有 slug 的旧 writer/测试形状"不得宽松采信 —— 裸快照(无复合
    # 身份键)一律 unknown,无论是否有终止锚/视角。只有携带完整复合身份
    # 且不矛盾(上文已校验)的快照,或精确线程/原生文件绑定,才可供来源判定。
    identity_keys = {k for aliases in _ID_REQUIRED.values() for k in aliases} - {"slug"}
    if not any(re_peer.get(k) is not None for k in identity_keys):
        return False
    return True


def load_runtime_evidence(review_dir: Path) -> tuple[list[dict], list[str]]:
    """Collect peer snapshots from runtime-evidence.json.

    Accepts both flat {"peers": [...]} and fixture {"goal": {"peers": [...]}}
    shapes. Returns (peers, notes). Missing/contradictory identity → the
    peer entry is kept but flagged incomplete by callers.
    """
    rep = review_dir / "runtime-evidence.json"
    if not rep.exists():
        return [], ["runtime-evidence-missing"]
    raw = _read_regular_file(rep)
    if raw is None:
        return [], ["runtime-evidence-unreadable"]
    try:
        data = json.loads(raw)
    except json.JSONDecodeError:
        return [], ["runtime-evidence-corrupt"]
    peers: list[dict] = []
    notes: list[str] = []
    if isinstance(data, dict):
        if isinstance(data.get("peers"), list):
            peers.extend(p for p in data["peers"] if isinstance(p, dict))
        goal = data.get("goal")
        if isinstance(goal, dict) and isinstance(goal.get("peers"), list):
            peers.extend(p for p in goal["peers"] if isinstance(p, dict))
    # dedupe by slug, keep LAST record (later snapshot rows are fresher)
    seen: dict[str, dict] = {}
    for p in peers:
        slug = p.get("slug")
        if not isinstance(slug, str) or not slug:
            notes.append("runtime-evidence-peer-missing-slug")
            continue
        seen[slug] = p
    return list(seen.values()), notes


def peer_has_active_thread(
    runtime_evidence_peer: dict | None, ui_threads: list[dict], slug: str
) -> bool:
    """active_thread belongs to exactly ONE peer — never broadcast."""
    if runtime_evidence_peer is not None:
        at = runtime_evidence_peer.get("active_thread")
        if isinstance(at, str) and at:
            return True
    # ui-protocol thread records bound to this slug's exact session
    for th in ui_threads:
        if th.get("peer") == slug and th.get("active"):
            return True
    return False


_CWD_SUFFIX_RE = re.compile(r"(?:\x00|\\x00)~cwd-[0-9A-Za-z]+\Z")


def _wire_trunk_and_cwd(session_id: str) -> tuple[str, str | None]:
    """wire session → (剥 cwd 后的主干, cwd 后缀或 None)。"""
    m = _CWD_SUFFIX_RE.search(session_id)
    cwd = m.group(0) if m else None
    trunk = _CWD_SUFFIX_RE.sub("", session_id)
    return trunk, cwd


def _wire_session_same_origin(a: str, b: str) -> bool:
    """两个 wire session 是否同一 originator(channel + master leaf)。

    真实 native originator 文件通常无 cwd 后缀(`octosfix:local:tui#coding`),
    thread session 才带真实 NUL+~cwd-hash。双方 trunk(channel+master leaf)
    必须一致;双方都带 cwd 后缀时哈希还须相等(同 trunk 不同 cwd = 不同
    工作区,不绑定);只有一方带后缀时以 trunk 为准(该侧不携带 cwd 信息,
    cwd 由 thread 绑定另行把关)。
    """
    trunk_a, cwd_a = _wire_trunk_and_cwd(a)
    trunk_b, cwd_b = _wire_trunk_and_cwd(b)
    if not trunk_a or not trunk_b or trunk_a != trunk_b:
        return False
    if cwd_a is not None and cwd_b is not None:
        return cwd_a == cwd_b
    return True


def peer_identity_bound_here(
    peer_dir: Path | None,
    session: str | None,
    goal: str | None,
) -> tuple[bool, str | None]:
    """peer 目录自身身份文件(originator/goal)与当前视角逐键绑定。

    真实 native 形状(native-observation-shapes.json):
      * originator 文件 = 派发该 peer 的 wire master session(通常无 cwd
        后缀)。与当前 master 不同 originator(如 `#other-master`)→ 该 peer
        属于别的 master,即使存在看似本 session 的未完 thread 也不得 running。
      * goal 文件 = 派发时 goal id,与当前 goal 不一致 → 不归属当前视角。

    身份文件与当前视角矛盾 → 不绑定(fail-closed,reason 说明);文件缺失 =
    信息不足,不构成矛盾,由其余证据链(lifetime/thread/快照)把关;当前
    session/goal 未知时无法比对,同样不构成矛盾。
    """
    if peer_dir is None:
        return True, None
    orig_raw = _read_regular_file(peer_dir / "originator")
    if orig_raw is not None:
        orig = orig_raw.strip()
        if orig:
            if not isinstance(session, str) or not session:
                return True, None  # 当前 session 未知: 无法比对,不构成矛盾
            if not _wire_session_same_origin(orig, session):
                return False, "peer-originator-mismatch"
    goal_raw = _read_regular_file(peer_dir / "goal")
    if goal_raw is not None:
        gf = goal_raw.strip()
        if gf:
            if not isinstance(goal, str) or not goal:
                return True, None  # 当前 goal 未知: 无法比对,不构成矛盾
            if gf != goal:
                return False, "peer-goal-mismatch"
    return True, None


def peer_identity_proves_binding(
    peer_dir: Path | None,
    session: str | None,
    goal: str | None,
) -> tuple[bool, str | None]:
    """正向身份证明(外层裁决 v3-8: 信息不足 ≠ 归属证明)。

    thread 单独晋升 running 前,peer 目录身份文件必须对当前视角中**明确
    已知**的身份字段给出实际 native 匹配证据:
      * session 已知 → originator 文件必须在场且与当前 master 同源;
      * goal 已知 → goal 文件必须在场且等于当前 goal。
    字段在当前视角未知时不强求对应文件(无 goal context 的通用视角);
    文件缺失/不匹配 → 无正向证明(fail-closed),与
    `peer_identity_bound_here` 的"矛盾才拒绝"互补: 那里只排冲突,
    这里要求证明。没有 lifetime 时 native thread 不是可绕过身份的权威。
    """
    if peer_dir is None:
        return True, None
    orig_raw = _read_regular_file(peer_dir / "originator")
    orig = (orig_raw or "").strip()
    if isinstance(session, str) and session:
        if not orig:
            return False, "peer-originator-unproven-missing"
        if not _wire_session_same_origin(orig, session):
            return False, "peer-originator-unproven-mismatch"
    goal_raw = _read_regular_file(peer_dir / "goal")
    gf = (goal_raw or "").strip()
    if isinstance(goal, str) and goal:
        if not gf:
            return False, "peer-goal-unproven-missing"
        if gf != goal:
            return False, "peer-goal-unproven-mismatch"
    return True, None


def wire_peer_slug(session_id) -> str | None:
    """真实 native thread wire 的 session_id → peer slug(仅 peer- 会话)。

    形状: `<profile>:<chan>:<chat>#peer-<slug>` + 可选 cwd 后缀
    (`\\x00~cwd-<hash>` 或真实 `\\x00~cwd-<hash>`)。master 会话(无
    `peer-` 前缀)与畸形输入 → None,绝不把 peer- 前缀或 cwd 后缀算进 slug。
    """
    if not isinstance(session_id, str):
        return None
    base = _CWD_SUFFIX_RE.sub("", session_id)
    if "#" not in base:
        return None
    leaf = base.rsplit("#", 1)[1]
    if not leaf.startswith("peer-"):
        return None
    slug = leaf[len("peer-"):]
    return slug or None


def ui_protocol_threads(runtime_dir: Path, session: str | None) -> list[dict]:
    """ui-protocol threads: read-only, decode session binding.

    v3-6: 真实 native wire 只有 v/session_id/thread_id/next_seq/completed
    字段,没有 active 键。未完流(completed is False)且精确绑定本 session →
    active;其他一切(缺 completed、completed=True、跨 session)保守不 active。

    NOTE (spec): never compare next_seq (stream event counter) with result-N
    turn (finished-turn counter) — different dimensions.
    """
    base = runtime_dir / "ui-protocol"
    if not base.exists():
        return []
    out = []
    for th in sorted(base.glob("*/threads/*.json")):
        try:
            data = json.loads(th.read_text(errors="replace"))
        except (json.JSONDecodeError, OSError):
            out.append({"file": th.name, "corrupt": True})
            continue
        sid = data.get("session_id")
        # 仅接受真实 wire 形状(v==1 + thread_id 非空);猜测形状不采信
        wire_ok = data.get("v") == 1 and isinstance(data.get("thread_id"), str)
        slug = wire_peer_slug(sid)
        # 精确 session 绑定(外层裁决 #3): peer 与 master 必须共享同一
        # channel 主干('#' 前,剥 cwd 后缀)且 cwd 哈希一致 —— 同 channel
        # 不同 master leaf 由 channel 主干一致容纳(peer 挂在 master 所在
        # channel),但不同 cwd / 跨 channel 一律不绑定。master 无 cwd 后缀
        # 时(信息不足)保守不绑定,不做"同 channel 即同源"推断。不得匹配
        # 字面 `\x00` 四字符转义。
        bound = False
        if isinstance(session, str) and session and isinstance(sid, str) and slug:
            sess_cwd = _CWD_SUFFIX_RE.search(session)
            sid_cwd = _CWD_SUFFIX_RE.search(sid)
            sess_chan = _CWD_SUFFIX_RE.sub("", session).rsplit("#", 1)[0]
            sid_chan = _CWD_SUFFIX_RE.sub("", sid).rsplit("#", 1)[0]
            bound = (
                bool(sess_chan)
                and sess_chan == sid_chan
                and sess_cwd is not None
                and sid_cwd is not None
                and sess_cwd.group(0) == sid_cwd.group(0)
            )
        entry = {
            "file": th.name,
            "session_id": sid,
            "completed": data.get("completed"),
            "peer": slug,
            # active 只在真实 wire 形状 + completed 恰为 False + 本 session 绑定
            "active": bool(wire_ok and slug and bound and data.get("completed") is False),
        }
        if session and entry["session_id"] and not bound:
            entry["session_mismatch"] = True
        # next_seq is recorded for display only; NEVER compared with turn.
        out.append(entry)
    return out


# ---------------------------------------------------------------------------
# last-outcome — terminal evidence cross-check (result-N.md × turns.txt)
# ---------------------------------------------------------------------------


def parse_turns_txt(path: Path) -> tuple[list[tuple[int, str]], list[str]]:
    """Parse '<round> <outcome> <ts>' rows; bad rows collected, not fatal.

    M6 fail-closed: 同一轮次多行(冲突或重复)→ 整个索引不可信: 标记
    duplicate 后**整个解析结束统一返回空 rows**+notes,后续 valid 行
    不得重新 append 恢复可信(RED 实测: `1 completed/1 errored/2
    completed` 曾被后续 append 恢复为 [(2,completed)],违背 M6 语义)。
    """
    rows: list[tuple[int, str]] = []
    bad: list[str] = []
    raw = _read_regular_file(path)
    if raw is None:
        return rows, ["turns-txt-missing"]
    seen: dict[int, str] = {}
    duplicate = False
    for i, line in enumerate(raw.splitlines(), 1):
        if not line.strip():
            continue
        parts = line.split()
        if len(parts) >= 2 and parts[0].isdigit():
            turn_n = int(parts[0])
            if turn_n in seen:
                bad.append(f"turns-duplicate-row:{i}")
                duplicate = True
                continue
            seen[turn_n] = parts[1]
            rows.append((turn_n, parts[1]))
        else:
            bad.append(f"turns-txt-bad-row:{i}")
    if duplicate:
        # 任意重复 → 全体不可信,空 rows(后续 valid 行不得恢复可信)
        return [], bad
    return rows, bad


def latest_terminated(native_dir: Path, expected_slug: str | None = None) -> dict:
    """Most recent TERMINAL outcome, cross-checked result-N × turns.txt.

    Untrusted/bad rows/missing files → unknown (source preserved); an old
    failure is never shown as success.

    v3-2/v3-3 strict binding:
      * frontmatter slug (when present) must equal the peer's slug;
      * frontmatter turn (when present) must equal the result file number;
      * ONLY outcome == "completed" is a terminal authority — any other
        outcome (even one consistent with turns.txt) → unknown.
    """
    if not native_dir.exists():
        return {"state": "unknown", "source": "native-dir-missing"}
    best_n, best = None, None
    for f in sorted(native_dir.iterdir()):
        m = _RESULT_RE.fullmatch(f.name)
        if m and f.is_file() and not f.is_symlink():
            n = int(m.group(1))
            if best_n is None or n > best_n:
                best_n, best = n, f
    if best is None:
        return {"state": "unknown", "source": "no-result-files"}
    fm = parse_frontmatter(best)
    outcome = fm.get("outcome")
    turn = fm.get("turn")
    notes: list[str] = []
    result = {
        "state": "unknown",
        "turn": turn,
        "source": best.name,
        "notes": notes,
    }
    # v3-2: report must bind to THIS peer's slug (foreign report → unknown)
    fm_slug = fm.get("slug")
    if expected_slug is not None and fm_slug is not None and fm_slug != expected_slug:
        notes.append(f"result-slug-mismatch:fm={fm_slug},peer={expected_slug}")
        return result
    # v3-2: frontmatter turn(存在时)必须精确等于 result 文件编号 —— 警示性
    # 不一致(伪造/串线报告)不得当终止权威。
    if turn is not None and turn != str(best_n):
        notes.append(f"result-turn-mismatch:fm={turn},file={best_n}")
        return result
    # v3-2: frontmatter turn must match file number (strict, both directions)
    if turn is None or not turn.isdigit() or int(turn) != best_n:
        notes.append(f"result-turn-mismatch:fm={turn},file={best_n}")
        return result
    if not outcome:
        notes.append("result-frontmatter-no-outcome")
        return result
    # v3-3 两层分离(契约澄清):
    #   * last-outcome 终止层(本函数): 四种真实终止 outcome
    #     (completed/errored/interrupted/rate_limited)如实显示,不折叠成
    #     unknown;旧失败不改判,也永不晋升为 completed。
    #   * review-freeze 准入层(另一处): 只有 completed 算有效终止初审。
    # M7(v3-cross 兑现): 四值终止态**全部**与 turns.txt 交叉核对 ——
    # 非 completed 终止值(errored/interrupted/rate_limited)此前直接返回,
    # 缺失/冲突的 turns.txt 不影响显示;现改为: turns.txt 缺失、最新轮
    # 与 result-N 不一致、或同轮 outcome 冲突 → 降 unknown + note
    # (与 completed 同一把尺;不支持/冲突的证据不得直接显示终止值)。
    # 非终止态(pending/running)与伪造值(fabricated 等)仍一律 unknown。
    if outcome not in _KNOWN_TERMINAL_OUTCOMES:
        notes.append(f"result-outcome-untrusted:{outcome}")
        return result
    rows, turn_notes = parse_turns_txt(native_dir / "turns.txt")
    notes.extend(turn_notes)
    if not rows:
        # turns.txt missing/全部坏行 → 四值终止一律不可信 → unknown + note
        notes.append("turns-txt-untrusted")
        return result
    last_round, last_outcome = max(rows, key=lambda r: r[0])
    result["turns_max"] = last_round
    if last_round != best_n:
        notes.append(f"turns-txt-round-mismatch:{last_round}!={best_n}")
        return result
    if last_outcome != outcome:
        notes.append(f"outcome-conflict:result={outcome},turns={last_outcome}")
        return result
    result["state"] = outcome
    return result


# ---------------------------------------------------------------------------
# peers — precise runtime/profile/master/session/slug binding
# ---------------------------------------------------------------------------


def collect_peers(
    review_dir: Path,
    runtime_dir: Path | None,
    profile: str,
    session: str | None,
    context: dict | None = None,
) -> tuple[dict[str, dict], list[str]]:
    """Per-peer state. Never leaks one peer's liveness to another."""
    notes: list[str] = []
    peers: dict[str, dict] = {}
    # 当前 goal(用于快照 goal 逐键校验);context['goal'] 为字符串时生效。
    ctx_goal = None
    if isinstance(context, dict):
        g = context.get("goal")
        if isinstance(g, str) and g:
            ctx_goal = g

    re_peers, re_notes = load_runtime_evidence(review_dir)
    notes.extend(re_notes)
    re_by_slug = {p["slug"]: p for p in re_peers}

    # native evidence dirs (last-outcome authority)
    native_root = review_dir / "native"
    native_slugs: list[str] = []
    if native_root.exists():
        native_slugs = sorted(
            d.name for d in native_root.iterdir() if d.is_dir() and not d.is_symlink()
        )

    # runtime peers root (lifetime authority + closed)
    peers_root: Path | None = None
    runtime_slugs: list[str] = []
    if runtime_dir is not None:
        cand = runtime_dir / "profiles" / profile / "data" / "peers"
        if cand.exists():
            peers_root = cand
            runtime_slugs = sorted(
                d.name for d in cand.iterdir() if d.is_dir() and not d.is_symlink()
            )

    ui_threads = ui_protocol_threads(runtime_dir, session) if runtime_dir else []

    all_slugs = sorted(set(native_slugs) | set(runtime_slugs) | set(re_by_slug))
    for slug in all_slugs:
        entry: dict = {
            "slug": slug,
            "execution": "unknown",
            "execution_source": "none",
            "task_id": None,
            "generation": None,
            "turn_id": None,
            "master_session_id": None,
        }
        peer_dir = peers_root / slug if peers_root else None

        # 真实身份绑定: peer 自身身份文件(originator/goal)与当前视角逐键
        # 比对 —— originator 指向别的 wire master 或 goal 文件与当前 goal
        # 不一致 → 该 peer 不属于当前视角,任何 fallback(thread/快照)都
        # 不得把它晋升 running(不只是"lifetime 存在时拒绝矛盾")。
        ident_ok, ident_reason = peer_identity_bound_here(peer_dir, session, ctx_goal)

        # closed 独立: 终态标记优先,不把旧失败变成功。跨仓 parity
        # (#2272 约定对齐): closed 在同一可信 lifetime 且归属一致
        # (ident_ok)时**保留身份字段**(task_id/generation/turn/
        # master_session_id 取 lifetime),便于关联终止事件;execution
        # 恒为 "closed" 不受 lifetime phase 影响;foreign/malformed
        # lifetime → 身份保持 null(不采信);旧失败不改判。
        if peer_dir is not None and (peer_dir / "closed").exists():
            entry["execution"] = "closed"
            entry["execution_source"] = "closed-marker"
            closed_rec, _closed_reason = trusted_lifetime(
                peer_dir, profile, slug, session
            )
            if closed_rec is not None and ident_ok:
                entry["task_id"] = closed_rec.get("task_id")
                entry["generation"] = closed_rec.get("generation")
                entry["turn_id"] = closed_rec.get("turn_id")
                entry["master_session_id"] = closed_rec.get("master")
        elif peer_dir is not None:
            rec, reason = trusted_lifetime(peer_dir, profile, slug, session)
            if rec is not None and ident_ok:
                # PR#632 P2-A: 有效 lifetime 晋升前同样要求归属 —— 同
                # session 但 peer goal 文件属于其他 review/goal 的
                # lifetime 不是本视角证据,不得显示 running/idle
                # (ident_ok 在此分支前已算好,此前只在 lifetime 失败后
                # 才看,跨 goal 有效 lifetime 被误晋升)。
                entry["execution"] = PHASE_TO_EXECUTION[rec["phase"]]
                entry["execution_source"] = "lifetime"
                entry["task_id"] = rec["task_id"]
                entry["generation"] = rec["generation"]
                entry["turn_id"] = rec["turn_id"]
                entry["master_session_id"] = rec["master"]
            elif rec is not None and not ident_ok:
                # 有效形状但归属矛盾(goal/originator 属其他视角) →
                # fail-closed unknown,不走 thread 兜底。
                entry["execution_reason"] = ident_reason or "peer-identity-mismatch"
            elif not ident_ok:
                # 身份文件矛盾优先于一切 fallback: 证据属于别的 master/goal,
                # unknown 且不再走 active-thread 兜底(fail-closed)。
                entry["execution_reason"] = ident_reason or "peer-identity-mismatch"
            else:
                entry["execution_reason"] = reason
                # lifetime 存在但因 session 归属被拒 → 证据属于别的 session,
                # 不再采信无 session 绑定的 active-thread 兜底(fail-closed)。
                if reason != "lifetime-master-not-current-session":
                    # 无 CURRENT authority: 只有该 peer 自己的精确活跃 thread
                    # 且身份绑定通过,才 running。
                    re_peer = re_by_slug.get(slug)
                    thread_active = any(
                        th.get("peer") == slug and th.get("active") for th in ui_threads
                    )
                    if re_peer is not None and _snapshot_bound_here(
                        re_peer, runtime_dir, session, ctx_goal, profile
                    ):
                        prov_ok, prov_reason = peer_identity_proves_binding(
                            peer_dir, session, ctx_goal
                        )
                        if not prov_ok:
                            entry["execution_reason"] = prov_reason
                        elif peer_has_active_thread(re_peer, ui_threads, slug):
                            entry["execution"] = "running"
                            entry["execution_source"] = "active-thread"
                        elif thread_active and prov_ok:
                            # 精确 session 绑定的未完 native thread 也是
                            # CURRENT authority(快照无 active 声明时)。
                            entry["execution"] = "running"
                            entry["execution_source"] = "active-thread"
                    elif re_peer is None and thread_active:
                        prov_ok, prov_reason = peer_identity_proves_binding(
                            peer_dir, session, ctx_goal
                        )
                        if prov_ok:
                            # 无 runtime-evidence 快照: 精确绑定的未完 native
                            # thread 单独构成 CURRENT authority(身份已正向证明)。
                            entry["execution"] = "running"
                            entry["execution_source"] = "active-thread"
                        else:
                            entry["execution_reason"] = prov_reason
        else:
            # no runtime peers root: runtime-evidence precise binding only
            re_peer = re_by_slug.get(slug)
            if re_peer is not None:
                if not _snapshot_bound_here(re_peer, runtime_dir, session, ctx_goal, profile):
                    entry["execution"] = "unknown"
                    entry["execution_reason"] = "runtime-evidence-identity-incomplete"
                elif peer_has_active_thread(re_peer, ui_threads, slug):
                    entry["execution"] = "running"
                    entry["execution_source"] = "active-thread"
                else:
                    entry["execution"] = "unknown"
                    entry["execution_reason"] = "no-active-thread"

        # last-outcome: terminal evidence only
        native_dir = native_root / slug
        if native_dir.exists():
            entry["last-outcome"] = latest_terminated(native_dir, expected_slug=slug)
        elif peer_dir is not None:
            entry["last-outcome"] = latest_terminated(peer_dir, expected_slug=slug)
        else:
            # 无 native result-N/turns 终止证据时,快照自报 outcome/outcome_source
            # 不是终止权威(自报来源字符串不可信)—— unknown,但保留自报值
            # 作可观测性说明;终止层仍由 native 证据按四值分层显示。
            re_peer = re_by_slug.get(slug)
            outcome = (re_peer or {}).get("outcome")
            src = (re_peer or {}).get("outcome_source")
            snap_notes = []
            if isinstance(outcome, str) and outcome:
                snap_notes.append(f"snapshot-outcome-untrusted:{outcome}")
            if isinstance(src, str) and src:
                snap_notes.append(f"snapshot-outcome-source-untrusted:{src}")
            entry["last-outcome"] = {
                "state": "unknown",
                "source": "runtime-evidence-self-claim",
                "notes": snap_notes,
            }
        peers[slug] = entry
    return peers, notes


def _identity_incomplete(re_peer: dict) -> bool:
    """v3-4: 完整复合身份(runtime/session/goal/peer/head)缺任一 → incomplete。

    只有 slug 不够 —— 这也是外层探针 `snapshot-identity-missing` 的契约。
    内部 collect_peers 渲染路径用 `_snapshot_bound_here`(裸快照宽松)。
    """
    slug = re_peer.get("slug")
    if not isinstance(slug, str) or not slug:
        return True
    for aliases in _ID_REQUIRED.values():
        v = next((re_peer.get(k) for k in aliases if re_peer.get(k) is not None), None)
        if not isinstance(v, str) or not v:
            return True
    return False


def _negative_event_goal_ok(ev: dict, goal: str | None) -> bool:
    """goal 过滤: goal 未知时不过滤(旧兼容);已知时仅匹配或无 goal 的
    slug 归属事件(escalation 等)可见。"""
    if goal is None:
        return True
    ev_goal = ev.get("goal_id")
    return not (isinstance(ev_goal, str) and ev_goal) or ev_goal == goal


# ---------------------------------------------------------------------------
# ACK board observation — content-hash keyed, own atomic cache
# ---------------------------------------------------------------------------


def ack_lines(board: Path) -> dict[str, dict]:
    """Extract ACK lines keyed by content identity (宽松子串, not line-no/prefix).

    Key: the task-id token inside ACK(...) when it contains a digit
    (e.g. `ACK(9 done)` → `9`); otherwise the ordinal among digit-less ACKs.
    A status-word edit (done→blocked) keeps the same key → in-place edit,
    never add+remove. Multiple ACKs sharing one task-id are distinguished by
    occurrence index.
    """
    out: dict[str, dict] = {}
    if not board.exists() or board.is_symlink():
        return out
    raw = _read_regular_file(board)
    if raw is None:
        return out
    counts: dict[str, int] = {}
    ordinal = 0
    for i, line in enumerate(raw.splitlines()):
        m = _ACK_RE.search(line)
        if not m:
            continue
        token = m.group(1).strip()
        # 数字 token(任务号)为稳定内容键;status 词不计入键
        digit_word = next((w for w in token.split() if any(c.isdigit() for c in w)), None)
        if digit_word is not None:
            base = digit_word
        else:
            base = f"ack-{ordinal}"
        ordinal += 1
        idx = counts.get(base, 0)
        counts[base] = idx + 1
        key = base if idx == 0 else f"{base}#{idx}"
        out[key] = {"line": i + 1, "sha256": sha256_text(line)}
    return out


def ack_events(current: dict[str, dict], baseline: dict[str, dict]) -> list[dict]:
    """add / in-place edit / remove, by content hash. Never line-count."""
    events: list[dict] = []
    for key, cur in current.items():
        if key not in baseline:
            events.append(
                {"type": "ack-added", "ack": key, "line": cur["line"], "sha256": cur["sha256"][:12]}
            )
        elif baseline[key]["sha256"] != cur["sha256"]:
            events.append(
                {
                    "type": "ack-inplace-edit",
                    "ack": key,
                    "line": cur["line"],
                    "old_sha256": baseline[key]["sha256"][:12],
                    "new_sha256": cur["sha256"][:12],
                }
            )
    for key, old in baseline.items():
        if key not in current:
            events.append(
                {"type": "ack-removed", "ack": key, "line": old["line"], "sha256": old["sha256"][:12]}
            )
    return events


# ---------------------------------------------------------------------------
# negative events — goal_transition blocked / escalation, this runtime only
# ---------------------------------------------------------------------------


def negative_events(
    runtime_dir: Path | None,
    goal: str | None,
    profile: str | None = None,
    session: str | None = None,
) -> list[dict]:
    """events.jsonl of THIS runtime only — precise attribution.

    v3-7: 按当前 profile + goal(+ originator session,事件携带时)过滤:
    同 runtime 其他 profile 的 blocked 不混入当前 goal;同 profile 其他
    goal 的事件也不混入。goal 未知(None)时保守按 profile 过滤。
    """
    if runtime_dir is None:
        return []
    out: list[dict] = []
    base = runtime_dir / "profiles"
    if not base.exists():
        return []
    # profile 精确归属(外层裁决 #2): 无 profile 即无事件可归属 —— 不得
    # fallback 扫其他 profile(会把其他 profile 的事件混入当前视角)。
    profiles = [profile] if profile else []
    for prof in profiles:
        ev_file = base / prof / "data" / "events.jsonl"
        if not ev_file.exists() or ev_file.is_symlink():
            continue
        raw = _read_regular_file(ev_file)
        if raw is None:
            continue
        runtime_tag = str(runtime_dir)
        for line in raw.splitlines():
            line = line.strip()
            if not line:
                continue
            try:
                ev = json.loads(line)
            except json.JSONDecodeError:
                continue
            if not isinstance(ev, dict):
                continue
            kind = ev.get("kind")
            ev_goal = ev.get("goal_id")
            ev_session = ev.get("session")
            ev_slug = ev.get("slug")
            # goal 过滤(当前 goal 已知时): 事件带其他 goal → 不混入;
            # 无 goal 的 slug 归属事件(escalation 等)按 profile 保留。
            if not _negative_event_goal_ok(ev, goal):
                continue
            # originator session 过滤: 事件带 session 且当前 session 已知 →
            # 必须相等(防同 runtime 其他 master session 串线)。
            if session and isinstance(ev_session, str) and ev_session and ev_session != session:
                continue
            if kind == "goal_transition":
                detail = str(ev.get("detail", ""))
                if "blocked" in detail or "escalation" in detail:
                    out.append(
                        {
                            "kind": kind,
                            "goal_id": ev_goal,
                            "detail": detail,
                            "runtime": runtime_tag,
                        }
                    )
            elif kind == "escalation":
                out.append(
                    {
                        "kind": kind,
                        "goal_id": ev_goal,
                        "slug": ev_slug,
                        "detail": str(ev.get("detail", "")),
                        "runtime": runtime_tag,
                    }
                )
    return out


# ---------------------------------------------------------------------------
# renderer
# ---------------------------------------------------------------------------


def render(
    review_dir: Path,
    board: Path | None,
    runtime_dir: Path | None,
    profile: str,
    session: str | None,
) -> dict:
    state, state_corrupt = read_review_state(review_dir)
    eff_session = session or state.get("session")
    goal = state.get("goal")
    # negative_events 过滤只用原始 goal 值: 非字符串/缺失 → None(不过滤),
    # 不得用 display fallback 误过滤。
    filter_goal = goal if isinstance(goal, str) and goal else None
    master = (state.get("master") or None) if isinstance(state, dict) else None

    peers, peer_notes = collect_peers(
        review_dir, runtime_dir, profile, eff_session, {"goal": filter_goal}
    )

    # deliverables: listed, never declared accepted by mere file existence
    deliverables = []
    if review_dir.exists():
        deliverables = sorted(
            p.name
            for p in review_dir.iterdir()
            if p.is_file() and not p.is_symlink() and p.suffix in (".md", ".json")
            and p.name != "monitor-state.json"
        )

    # ACK cache: own monitor-state.json, atomic, content-hash keyed
    ack_evs: list[dict] = []
    ack_diag: str | None = None
    if board is not None:
        current = ack_lines(board)
        mon, mon_diag = read_monitor_state(review_dir)
        if mon_diag:
            ack_diag = mon_diag
        baseline = mon.get("acks") if isinstance(mon.get("acks"), dict) else {}
        # normalize: keys must be strings (JSON object keys always are)
        baseline = {str(k): v for k, v in baseline.items() if isinstance(v, dict)}
        ack_evs = ack_events(current, baseline)
        mon["protocol"] = PROTOCOL
        mon["acks"] = current
        wdiag = write_monitor_state_atomic(review_dir, mon)
        if wdiag:
            ack_diag = (ack_diag + "; " if ack_diag else "") + wdiag

    # lifetime summary for current-turn block (per-peer detail in peers)
    lifetime_states = {
        slug: (p["execution_source"] if p["execution_source"] == "lifetime" else "untrusted")
        for slug, p in peers.items()
    }

    result = {
        "protocol": PROTOCOL,
        "identity": composite_identity(
            str(runtime_dir or state.get("runtime", "")),
            eff_session,
            goal,
            profile,
            master,
        ),
        "review_state_corrupt": state_corrupt,
        "lifecycle": {
            "frozen": bool(state.get("frozen", False)),
            "challenge_accepted": bool((state.get("challenge") or {}).get("accepted", False))
            if isinstance(state.get("challenge") or {}, dict)
            else False,
            "cross_count": len(state.get("cross", []))
            if isinstance(state.get("cross", []), list)
            else 0,
            "corrupt": state_corrupt,
        },
        "current-turn": {
            "lifetime": {
                # aggregate: authoritative only if ≥1 peer trusted via lifetime
                "state": (
                    "authoritative"
                    if any(p["execution_source"] == "lifetime" for p in peers.values())
                    else "unknown"
                ),
                "peers": lifetime_states,
            },
            "peers": {
                slug: {
                    "execution": p["execution"],
                    "source": p["execution_source"],
                    **({"reason": p["execution_reason"]} if p.get("execution_reason") else {}),
                    "task_id": p["task_id"],
                    "generation": p["generation"],
                    "turn_id": p["turn_id"],
                    "master_session_id": p["master_session_id"],
                }
                for slug, p in peers.items()
            },
            "threads": ui_protocol_threads(runtime_dir, eff_session) if runtime_dir else [],
        },
        "last-outcome": {
            "peers": {slug: p["last-outcome"] for slug, p in peers.items()},
            "note": "最近终止结果;与 current-turn 分层显示,不混写",
        },
        "deliverables": {
            "files": deliverables,
            "accepted": False,
            "note": "deliverables 不凭文件存在宣称 accepted",
        },
        "ack_events": ack_evs,
        "events": negative_events(runtime_dir, filter_goal, profile, eff_session),
    }
    diags = []
    if state_corrupt:
        diags.append("review-state.json corrupt — rendered with degraded state, file untouched")
    if ack_diag:
        diags.append(ack_diag)
    diags.extend(peer_notes)
    if diags:
        result["diagnostics"] = diags
    return result


# ---------------------------------------------------------------------------
# human renderer
# ---------------------------------------------------------------------------


def human(result: dict) -> str:
    out = []
    out.append(f"== OctoLoop review monitor [{PROTOCOL}] == {result['identity']}")
    lc = result["lifecycle"]
    corrupt_note = " CORRUPT(read-only)" if lc.get("corrupt") else ""
    out.append(
        f"[lifecycle] frozen={lc['frozen']} challenge={lc['challenge_accepted']} "
        f"cross={lc['cross_count']}{corrupt_note}"
    )
    lt = result["current-turn"]["lifetime"]
    out.append(f"[current-turn] lifetime={lt['state']}")
    for slug, st in result["current-turn"]["peers"].items():
        reason = f" ({st['reason']})" if st.get("reason") else ""
        out.append(f"  peer {slug}: current-turn={st['execution']} [{st['source']}]{reason}")
    if not result["current-turn"]["peers"]:
        out.append("  (no peers)")
    lo = result["last-outcome"]["peers"]
    if not lo:
        out.append("[last-outcome] (no peers yet)")
    for slug, st in lo.items():
        notes = f" notes={';'.join(st.get('notes') or [])}" if st.get("notes") else ""
        out.append(
            f"  peer {slug}: last-outcome={st['state']} (最近终止, source={st.get('source')}){notes}"
        )
    d = result["deliverables"]
    files = d["files"] if isinstance(d, dict) else d
    out.append("[deliverables] " + (", ".join(files) or "(none)") + " (accepted=false)")
    for ev in result["ack_events"]:
        out.append(f"[ack-event] {ev['type']} ack={ev.get('ack')} line={ev.get('line')} "
                   f"{ev.get('new_sha256') or ev.get('sha256') or ''}")
    for ev in result.get("events", []):
        g = f" goal={ev['goal_id']}" if ev.get("goal_id") else ""
        s = f" slug={ev['slug']}" if ev.get("slug") else ""
        out.append(f"[event] {ev['kind']}{g}{s} {ev['detail']} (runtime={ev['runtime']})")
    for diag in result.get("diagnostics", []):
        out.append(f"[diag] {diag}")
    return "\n".join(out)


# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------


def main() -> int:
    p = argparse.ArgumentParser(prog="olp-review-monitor")
    p.add_argument("review_dir")
    p.add_argument("--runtime-dir")
    p.add_argument("--board")
    p.add_argument("--profile", default=DEFAULT_PROFILE)
    p.add_argument("--session")
    p.add_argument("--format", choices=["json", "human"], default="human")
    p.add_argument("--watch", action="store_true")
    p.add_argument("--interval", type=float, default=2.0)
    args = p.parse_args()

    review_dir = Path(args.review_dir)
    board = Path(args.board) if args.board else None
    runtime_dir = Path(args.runtime_dir) if args.runtime_dir else None

    def emit(res: dict) -> None:
        if args.format == "json":
            print(json.dumps(res, ensure_ascii=False, indent=1), flush=True)
        else:
            print(human(res), flush=True)

    if args.watch:
        prev_sig = None
        try:
            while True:
                result = render(review_dir, board, runtime_dir, args.profile, args.session)
                sig = sha256_text(json.dumps(result, sort_keys=True, default=str))
                if sig != prev_sig:
                    emit(result)
                    prev_sig = sig
                time.sleep(args.interval)
        except KeyboardInterrupt:
            return 0
    emit(render(review_dir, board, runtime_dir, args.profile, args.session))
    return 0


if __name__ == "__main__":
    sys.exit(main())
