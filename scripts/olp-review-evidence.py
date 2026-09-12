#!/usr/bin/env python3
"""olp-review-evidence — OctoLoop 行为证据互审流程管理入口.

Spec: specs/task-evo-review-evidence.spec.md (23 scenarios)
Plan: docs/superpowers/plans/2026-09-09-review-evidence.md

子命令:
  init     — 初始化评审目录(HEAD 真实解析 + runtime/session/goal 复合身份)
  freeze   — 冻结两份独立初审(native result-N/turns.txt 同轮严格 completed +
             originator/goal 身份绑定 + 报告 turn 恰等 native 最新 + 报告
             session/goal/runtime/repo 与调用视角绑定 + --head 锚定 +
             claims 块 + 事务锁)
  challenge— 行为证据。唯一 live 入口: --live-cargo 由本入口实时执行固定的
             生产 adapter(scripts/olp-review-evidence-cargo.py);外部 JSON
             receipt 字段不是执行证明;--imported 一律 not-replayed;
             --exec-argv 任意执行体一律拒绝(executor-not-trusted)
  cross    — 收录交叉报告(原 reviewer slug + 新 native completed 轮次 +
             HEAD 绑定 + 结构化 cross_claims 逐 claim accept/refute/pending;
             子串包含不算覆盖;refute 需已验证新行为证据或显式
             --allow-operator-refute 人工裁决)
  status   — 汇总判词状态(claim 级两层词表 + review_accepted 收口标志;
             review_accepted 需全部冻结 claim 有可核对证据且两个原
             reviewer 各自新 native completed cross，且初审/cross 的原生
             turn_id 均绑定完整 ledger 中对应 lane 的实际模型)

错误输出一律可解析 JSON: {"error": {"code": "...", "message": "..."}} 且 exit != 0.
冻结/manifest 写入: per-review-dir flock 包住 load→validate→save(事务),
落盘用临时文件 + os.replace(原子),并发/失败不出现半写状态。

v5(evidence-core-k3-rescue): 外层六反例最终门
(../outer-evidence-final-gate-probes.json 0/6 → 本实现修复):
1. turns.txt 同轮 outcome 与 result-N 冲突(errored)→ freeze 拒绝;
2. 报告 turn 为未来轮次(> native 最新编号)→ turn-mismatch 拒绝;
3. native originator/goal 属于别的 master/goal → peer-authority-mismatch;
4. 手写 JSON receipt(无执行)→ evidence-not-executed,challenge 不接纳;
5. 伪造 receipt + --imported → 一律 not-replayed,--imported 不旁路;
6. 纯文字 cross("人工裁决"等)+ 未挑战 claim → missing-claim-coverage /
   review 不收口,已复现失败不得被文字改判 challenge-refuted。
"""
from __future__ import annotations

import argparse
import contextlib
import fcntl
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

PROTOCOL = "olp-review-evidence/v1"
STATE_FILENAME = "review-state.json"
LOCK_FILENAME = ".review-state.lock"
REVIEWER_LANES = ("glm", "k3")

# cross claim 裁决词表(结构化 cross_claims 块)
CROSS_VERDICTS = ("accept", "refute", "pending")
# 由本入口真实执行得出的判词依据(review_accepted 的"可核对证据")
EXECUTED_REASONS = ("executed-probe-passed", "challenge-evidence-executed")

# --format human: main() 设置,emit_json 切换为人类渲染(非 JSON 信封)。
CLI_HUMAN_FORMAT = False

# ---------------------------------------------------------------------------
# error handling: structured JSON on stdout, non-zero exit
# ---------------------------------------------------------------------------


class ReviewError(Exception):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code
        self.message = message


def emit_json(obj: dict) -> None:
    if CLI_HUMAN_FORMAT:
        sys.stdout.write(render_human(obj) + "\n")
        return
    json.dump(obj, sys.stdout, ensure_ascii=False, indent=1)
    sys.stdout.write("\n")


def fail(code: str, message: str) -> "ReviewError":
    return ReviewError(code, message)


# ---------------------------------------------------------------------------
# state store (transactional: flock around load→validate→save; atomic write)
# ---------------------------------------------------------------------------


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def atomic_write_json(path: Path, obj: dict) -> None:
    """Write JSON atomically: temp file in same dir + fsync + os.replace."""
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=str(path.parent), prefix=".tmp-state-")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            json.dump(obj, fh, ensure_ascii=False, indent=1)
            fh.write("\n")
            fh.flush()
            os.fsync(fh.fileno())
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def load_state(review_dir: Path) -> dict:
    p = review_dir / STATE_FILENAME
    if not p.exists():
        return {}
    try:
        return json.loads(p.read_text(encoding="utf-8"))
    except json.JSONDecodeError as e:
        raise fail("state-corrupt", f"{p} 不是合法 JSON: {e}")


def save_state(review_dir: Path, state: dict) -> None:
    state["updated_unix"] = int(time.time())
    atomic_write_json(review_dir / STATE_FILENAME, state)


@contextlib.contextmanager
def review_lock(review_dir: Path, *, read_only: bool = False):
    """同一 lock inode:写事务排他,只读入口共享且不创建路径/锁。"""
    if read_only:
        try:
            fd = os.open(review_dir / LOCK_FILENAME, os.O_RDONLY)
        except OSError as e:
            raise fail("review-lock-unavailable", f"无法读取评审锁: {review_dir}: {e}")
    else:
        review_dir.mkdir(parents=True, exist_ok=True)
        fd = os.open(review_dir / LOCK_FILENAME, os.O_CREAT | os.O_RDWR, 0o644)
    try:
        try:
            fcntl.flock(fd, fcntl.LOCK_SH if read_only else fcntl.LOCK_EX)
        except OSError as e:
            if read_only:
                raise fail("review-lock-unavailable", f"无法获取评审锁: {review_dir}: {e}")
            raise
        yield
    finally:
        # close 同时释放 flock,包括加锁失败和事务内部抛异常的路径。
        os.close(fd)


# ---------------------------------------------------------------------------
# frontmatter parsing (review reports)
# ---------------------------------------------------------------------------

_FM_RE = re.compile(r"\A---\s*\n(.*?)\n---\s*\n", re.S)
_CLAIMS_BLOCK_RE = re.compile(r"```json\s*\n(.*?)```", re.S)


def parse_frontmatter(path: Path) -> dict:
    text = path.read_text(encoding="utf-8", errors="replace")
    m = _FM_RE.match(text)
    if not m:
        return {"__missing__": True, "text_head": text[:200]}
    fm: dict = {}
    for line in m.group(1).splitlines():
        if ":" in line:
            k, _, v = line.partition(":")
            fm[k.strip()] = v.strip()
    fm["__sha256__"] = sha256_file(path)
    return fm


def _json_block(path: Path) -> dict | None:
    """报告正文首个 ```json 块解析为 dict;块非法 JSON → claims-block-invalid。"""
    text = path.read_text(encoding="utf-8", errors="replace")
    m = _CLAIMS_BLOCK_RE.search(text)
    if not m:
        return None
    try:
        obj = json.loads(m.group(1))
    except json.JSONDecodeError as e:
        raise fail("claims-block-invalid", f"{path.name} claims 块 JSON 非法: {e}")
    return obj if isinstance(obj, dict) else None


# 初审 claim verdict 白名单(与 cross_claims 的 accept/refute/pending 区分)
FIRST_REVIEW_VERDICTS = {"approve", "request-changes", "comment"}


def parse_claims(path: Path) -> list[dict]:
    """解析初审报告的显式 JSON claims 块: {"claims": [{"id","verdict","evidence"}]}.

    无块返回 [];块存在但非 claims 结构 → claims-block-invalid。
    """
    obj = _json_block(path)
    if obj is None:
        return []
    claims = obj.get("claims")
    if not isinstance(claims, list) or not claims:
        raise fail("claims-block-invalid", f"{path.name} claims 块缺非空 claims 数组")
    for c in claims:
        if not isinstance(c, dict) or not isinstance(c.get("id"), str) or not c["id"]:
            raise fail("claims-block-invalid", f"{path.name} claim 缺合法 id")
        # 审计尾项: 初审 verdict 白名单(真实报告语义: approve/request-
        # changes/comment)。**不**生搬 cross 的 accept/refute 词表 ——
        # 初审与交叉互审是不同合约层。非法/空/非 str → 结构化错误。
        v = c.get("verdict")
        if not isinstance(v, str) or v not in FIRST_REVIEW_VERDICTS:
            raise fail(
                "claims-verdict-invalid",
                f"{path.name} claim[{c['id']}] verdict={v!r} 非法"
                f"(合法: {sorted(FIRST_REVIEW_VERDICTS)})",
            )
    return claims


def parse_cross_claims(path: Path) -> list[dict] | None:
    """解析 cross 报告的结构化 cross_claims 块.

    无块或无 cross_claims 键 → None(由覆盖校验报 missing-claim-coverage);
    cross_claims 存在但形态非法 → cross-claim-invalid。每个条目必须含
    合法 id 与 verdict(accept/refute/pending);reference 为可选 dict。
    子串包含不算覆盖 — 只认此结构化块。
    """
    obj = _json_block(path)
    if obj is None:
        return None
    claims = obj.get("cross_claims")
    if claims is None:
        return None
    if not isinstance(claims, list) or not claims:
        raise fail("cross-claim-invalid", f"{path.name} cross_claims 缺非空数组")
    for c in claims:
        if not isinstance(c, dict) or not isinstance(c.get("id"), str) or not c["id"]:
            raise fail("cross-claim-invalid", f"{path.name} cross claim 缺合法 id")
        if c.get("verdict") not in CROSS_VERDICTS:
            raise fail(
                "cross-claim-invalid",
                f"{path.name} claim {c['id']} verdict={c.get('verdict')} 非法"
                f"(须为 {CROSS_VERDICTS})",
            )
        ref = c.get("reference")
        if ref is not None and not isinstance(ref, dict):
            raise fail("cross-claim-invalid", f"{path.name} claim {c['id']} reference 须为对象")
    return claims


def resolve_head(repo: Path) -> str:
    """解析 repo 真实 HEAD;失败 fail-closed,不回退到调用方自报。"""
    try:
        out = subprocess.run(
            ["git", "rev-parse", "HEAD"], cwd=str(repo),
            capture_output=True, text=True, timeout=30,
        )
    except (OSError, subprocess.TimeoutExpired):
        out = None
    head = (out.stdout.strip() if out else "")
    if out and out.returncode == 0 and re.fullmatch(r"[0-9a-f]{40}", head):
        return head
    raise fail("head-unresolvable", f"无法解析 repo 真实 HEAD: {repo}")


# ---------------------------------------------------------------------------
# peer validity: external signals override self-reported outcome
# ---------------------------------------------------------------------------


def latest_result(peer_dir: Path) -> tuple[int, Path] | None:
    """Highest-numbered result-N.md in a native peer dir."""
    best = None
    for f in peer_dir.glob("result-*.md"):
        m = re.fullmatch(r"result-(\d+)\.md", f.name)
        if m:
            n = int(m.group(1))
            if best is None or n > best[0]:
                best = (n, f)
    return best


def turns_index(peer_dir: Path) -> dict[int, str] | None:
    """turns.txt 轮次索引: {turn: outcome}(每行 '<turn> <outcome> ...')。

    M6 fail-closed: 同一轮次出现多行(即使 outcome 相同)→ 冲突,返回
    None —— 调用方必须按"轮次索引不完整/不可信"拒绝,不得后行覆盖前行
    (此前 `"1 errored\\n1 completed"` 会以 completed 通过核对)。
    """
    p = peer_dir / "turns.txt"
    idx: dict[int, str] = {}
    if not p.exists():
        return idx
    for line in p.read_text(encoding="utf-8", errors="replace").splitlines():
        parts = line.split()
        if parts and parts[0].isdigit():
            turn_n = int(parts[0])
            outcome = parts[1] if len(parts) > 1 else ""
            if turn_n in idx:
                # 同轮重复(冲突或重复)→ 索引不可信
                return None
            idx[turn_n] = outcome
    return idx


_CWD_SUFFIX_RE = re.compile(r"(?:\x00|\\x00)~cwd-[0-9A-Za-z]+\Z")


def _wire_trunk(session_id: str) -> str:
    """wire session → 剥 cwd 后缀的主干(channel+leaf),归一比较用。

    真实 native originator 文件通常无 cwd 后缀(`octosfix:local:tui#coding`),
    而 thread/快照 session 带 `NUL~cwd-hash`;身份比较须按主干归一
    (与 monitor `_wire_session_same_origin` 同语义),否则真实 wire 形状
    会因字节不精确被误拒(fail-closed 假阴性)。
    """
    if not isinstance(session_id, str):
        return ""
    return _CWD_SUFFIX_RE.sub("", session_id)


def _wire_trunk_and_cwd(session_id: str) -> tuple[str, str | None]:
    """wire session → (剥 cwd 后的主干, cwd 后缀或 None)。

    与 monitor `_wire_trunk_and_cwd` 完全同语义。
    """
    if not isinstance(session_id, str):
        return "", None
    m = _CWD_SUFFIX_RE.search(session_id)
    cwd = m.group(0) if m else None
    trunk = _CWD_SUFFIX_RE.sub("", session_id)
    return trunk, cwd


def _wire_session_same_origin(a: str, b: str) -> bool:
    """两个 wire session 是否同源(channel 主干 + cwd 语义)。

    与 monitor `_wire_session_same_origin` EXACT 语义:
    - 主干(channel+leaf)必须一致;
    - **双方都带 cwd 后缀时哈希必须相等**(同 trunk 不同 cwd = 不同
      工作区,不得匹配——只剥两端会造成跨工作区错误接受);
    - 只有一方带后缀时以主干为准(该侧不携带 cwd 信息,由 originator
      文件通常无后缀的真实形状决定)。
    """
    trunk_a, cwd_a = _wire_trunk_and_cwd(a)
    trunk_b, cwd_b = _wire_trunk_and_cwd(b)
    if not trunk_a or not trunk_b or trunk_a != trunk_b:
        return False
    if cwd_a is not None and cwd_b is not None:
        return cwd_a == cwd_b
    return True


def _identity_file_matches(peer_dir: Path, name: str, expected: str) -> bool:
    p = peer_dir / name
    if not p.exists():
        return False
    try:
        got = p.read_text(encoding="utf-8", errors="replace").strip()
    except OSError:
        return False
    if got == expected:
        return True
    # cwd 语义归一(monitor 同源判定): 双方后缀都在必须相等,
    # 单侧无后缀以主干为准 —— 不做无脑两端 strip。
    return _wire_session_same_origin(got, expected)


def _load_ledger_turns(runtime_root: Path, session: str | None, slug: str) -> list[dict]:
    """Read one complete native peer stream; never infer a turn from output chunks.

    Native ledger directories encode the peer session and optional cwd scope.
    Sequence gaps, malformed rows, ambiguous streams and missing turn_started
    records make native turn identity/model attribution unverifiable.
    """
    if not isinstance(session, str) or not session or not isinstance(slug, str) or not slug:
        return []
    peer_session = session.split("#", 1)[0] + f"#peer-{slug}"
    streams = []
    try:
        for directory in (runtime_root / "ui-protocol").iterdir():
            if not directory.is_dir():
                continue
            try:
                wire = bytes.fromhex(directory.name).decode("utf-8")
            except (ValueError, UnicodeError):
                continue
            if _CWD_SUFFIX_RE.sub("", wire) != peer_session:
                continue
            logs = list(directory.glob("ledger-*.log"))
            if logs:
                streams.append((wire, logs))
        if len(streams) != 1:
            return []
        wire, logs = streams[0]
        rows = {}
        for log in logs:
            for line in log.read_text(encoding="utf-8").splitlines():
                row = json.loads(line)
                if not isinstance(row, dict):
                    return []
                seq = row.get("seq")
                if type(seq) is not int or seq < 1 or seq in rows:
                    return []
                if not isinstance(row.get("event"), dict):
                    return []
                rows[seq] = row["event"]
        if not rows or sorted(rows) != list(range(1, len(rows) + 1)):
            return []
        turns = {}
        for seq in sorted(rows):
            ev = rows[seq]
            tid = ev.get("turn_id")
            if tid is None:
                md = ev.get("metadata")
                if (ev.get("kind") in ("turn_started", "turn_completed", "turn_error", "turn_interrupted")
                    or isinstance(md, dict) and md.get("kind") == "token_cost_update"):
                    return []
                continue  # envelope events may have no turn identity
            if not isinstance(tid, str) or not tid:
                return []
            if ev.get("session_id") != peer_session:
                return []
            kind = ev.get("kind") if ev.get("record_kind") == "notification" else None
            if tid not in turns:
                if kind != "turn_started":
                    return []
                turns[tid] = {"turn_id": tid, "models": [], "terminal": None,
                              "events": [], "stream": wire}
            elif kind == "turn_started":
                return []
            turn = turns[tid]
            md = ev.get("metadata")
            is_model = (ev.get("record_kind") == "progress" and isinstance(md, dict)
                        and md.get("kind") == "token_cost_update")
            terminal = kind in ("turn_completed", "turn_error", "turn_interrupted")
            if kind == "turn_started" or is_model or terminal:
                if turn["terminal"] is not None:
                    return []
                turn["events"].append({"seq": seq, "event": ev})
            if is_model:
                tc = md.get("token_cost")
                model = tc.get("model") if isinstance(tc, dict) else None
                if not isinstance(model, str) or not model:
                    return []
                if model not in turn["models"]:
                    turn["models"].append(model)
            if terminal:
                turn["terminal"] = kind
        result = []
        for ordinal, turn in enumerate(turns.values(), 1):
            evidence = {"stream": wire, "events": turn.pop("events")}
            turn["sha256"] = hashlib.sha256(
                json.dumps(evidence, sort_keys=True, separators=(",", ":")).encode()
            ).hexdigest()
            result.append({"turn_no": ordinal, **turn})
        return result
    except (OSError, UnicodeError, ValueError):
        return []


def _verify_peer_model_from_ledger(runtime_root: Path, session: str | None,
                                   slug: str, lane: str, report_turn: int,
                                   *, native_turn_id: str | None = None) -> dict:
    if not isinstance(native_turn_id, str) or not native_turn_id.strip():
        raise fail("peer-model-unverified", f"{slug} 原生报告缺 turn_id；请升级 octos，"
                   "新建短 reviewer peer 并在新评审上下文重做初审与 cross；不得按文件编号补 ID")
    turns = _load_ledger_turns(runtime_root, session, slug)
    matches = [turn for turn in turns if turn["turn_id"] == native_turn_id]
    if type(report_turn) is not int or report_turn < 1 or len(matches) != 1:
        raise fail("peer-model-unverified", f"{slug} turn={report_turn} 无完整 runtime 轮次证据；"
                   "若 ledger 前段已轮转丢失，请新建短 reviewer peer，"
                   "在新评审上下文重做初审与 cross；不得将剩余尾段重新编号")
    # result-N counts persisted files; a failed result write can make N
    # differ from the ledger ordinal. Only the runtime's ID joins the two.
    turn = matches[0]
    if turn["terminal"] != "turn_completed" or not turn["models"]:
        raise fail("peer-model-unverified", f"{slug} turn={report_turn} 未完成或缺实际模型")
    if lane not in REVIEWER_LANES or any(
        not re.fullmatch(re.escape(lane) + r"(?:-[A-Za-z0-9][A-Za-z0-9._-]*)?", model)
        for model in turn["models"]
    ):
        raise fail("peer-model-mismatch", f"{slug} lane={lane},实际模型={turn['models']}")
    return {"model": turn["models"], "turn_id": turn["turn_id"],
            "turn_no": report_turn, "ledger_turn_no": turn["turn_no"],
            "stream": turn["stream"], "sha256": turn["sha256"]}


def _positive_report_turn(value: object) -> int | None:
    if type(value) not in (int, str) or not re.fullmatch(r"[0-9]+", str(value)):
        return None
    try:
        number = int(value)
        return number if number > 0 else None
    except ValueError:
        return None


def _capture_native_report(native_root: Path | None, slug: str, turn: object) -> dict | None:
    if native_root is None:
        return None  # Legacy snapshot authority remains audit-only.
    number = _positive_report_turn(turn)
    if number is None:
        raise fail("peer-model-unverified", "原生报告编号无效")
    path = (native_root / slug / f"result-{number}.md").resolve()
    try:
        return {"path": str(path), "sha256": sha256_file(path)}
    except FileNotFoundError:
        # A completed runtime snapshot may authorize legacy audit collection
        # even when only another lane has native files. Model verification
        # still rejects the absent binding; never infer or backfill identity.
        return None
    except OSError:
        raise fail("peer-model-unverified", f"{slug} 原生报告不可读取")


def _native_report_turn_id(binding: object, slug: str, number: int) -> str:
    if (not isinstance(binding, dict)
        or any(not isinstance(binding.get(key), str) or not binding[key]
               for key in ("path", "sha256"))):
        raise fail("peer-model-unverified", f"{slug} 缺冻结的原生报告绑定；"
                   "请新建短 reviewer peer 并在新评审上下文重做初审与 cross")
    try:
        path = Path(binding["path"])
        raw = path.read_bytes()
        if (path.name != f"result-{number}.md"
            or hashlib.sha256(raw).hexdigest() != binding["sha256"]):
            raise fail("peer-model-unverified", f"{slug} 原生报告已变更或编号不匹配")
        # Identity and digest must describe the SAME read, not two reads
        # across an atomic replacement of a best-effort native result.
        match = _FM_RE.match(raw.decode("utf-8"))
        if match is None:
            raise fail("peer-model-unverified", f"{slug} 原生报告缺 frontmatter")
        fm = {}
        for line in match.group(1).splitlines():
            if ":" in line:
                key, _, value = line.partition(":")
                key = key.strip()
                if key in fm:
                    raise fail("peer-model-unverified", f"{slug} 原生报告身份键重复")
                fm[key] = value.strip()
    except (OSError, ValueError, UnicodeError):
        raise fail("peer-model-unverified", f"{slug} 原生报告不可读取")
    if (fm.get("slug") != slug or fm.get("outcome") != "completed"
        or fm.get("turn") != str(number)):
        raise fail("peer-model-unverified", f"{slug} 原生报告身份/编号/终态不匹配")
    if not fm.get("turn_id"):
        raise fail("peer-model-unverified", f"{slug} 原生报告缺 turn_id；请升级 octos，"
                   "新建短 reviewer peer 并在新评审上下文重做初审与 cross；不得按文件编号补 ID")
    return fm["turn_id"]


def _model_pair(state: dict, slug: str, cross_turn: object,
                cross_native_report: object = None) -> dict:
    """Both the frozen initial review and its later cross must use the lane model."""
    reviews = state.get("reviews") or {}
    lanes = [lane for lane in REVIEWER_LANES if isinstance(reviews, dict)
             and isinstance(reviews.get(lane), dict) and reviews[lane].get("peer") == slug]
    if len(lanes) != 1 or any(not isinstance(state.get(key), str) or not state[key]
                           for key in ("runtime", "session")):
        raise fail("peer-model-unverified", "缺 runtime/session/reviewer 绑定")
    lane = lanes[0]
    try:
        first = int(str(reviews[lane].get("turn")))
        later = int(str(cross_turn))
    except (ValueError, TypeError):
        raise fail("peer-model-unverified", "报告轮次无效")
    if first < 1 or later <= first:
        raise fail("peer-model-unverified", "cross 必须属于初审后的新轮次")
    runtime = Path(state["runtime"])
    initial_id = _native_report_turn_id(reviews[lane].get("native_report"), slug, first)
    cross_id = _native_report_turn_id(cross_native_report, slug, later)
    pair = {
        "initial": _verify_peer_model_from_ledger(runtime, state["session"], slug, lane, first,
                                                  native_turn_id=initial_id),
        "cross": _verify_peer_model_from_ledger(runtime, state["session"], slug, lane, later,
                                                native_turn_id=cross_id),
    }
    if pair["cross"]["ledger_turn_no"] <= pair["initial"]["ledger_turn_no"]:
        raise fail("peer-model-unverified", "cross 的 runtime turn_id 必须晚于初审，不得复用旧轮")
    return pair


def models_verified(state: dict) -> bool:
    """Recheck both distinct reviewers against their stored native evidence anchors."""
    reviews = state.get("reviews") or {}
    if (not isinstance(reviews, dict) or set(reviews) != set(REVIEWER_LANES)
        or any(not isinstance(r, dict) or not isinstance(r.get("peer"), str)
               or not r["peer"] for r in reviews.values())):
        return False
    peers = {r.get("peer") for r in reviews.values()}
    cross = state.get("cross") or []
    if (not isinstance(cross, list)
        or any(not isinstance(c, dict) or not isinstance(c.get("slug"), str)
               or not c["slug"] for c in cross)):
        return False
    if len(peers) != 2 or not all(peers) or {c.get("slug") for c in cross} != peers:
        return False
    # Cross is an append-only audit history. A fresh verified submission can
    # supersede a legacy one; a later unverified submission must downgrade it.
    latest = {c.get("slug"): c for c in cross}
    try:
        return all(c.get("model_evidence") == _model_pair(
                       state, c.get("slug"), c.get("turn"), c.get("native_report"))
                   for c in latest.values())
    except ReviewError:
        return False


def check_peer_validity(
    fm: dict,
    report_path: Path,
    native_dir: Path | None,
    runtime_evidence: dict | None,
    slug: str,
    require_authority: bool = False,
    min_turn_exclusive: int = 0,
    expected_session: str | None = None,
    expected_goal: str | None = None,
) -> bool:
    """Reject pending/errored/stale/foreign peers; external signals veto self-report.

    正向证明完整才采信(fail-closed),不是只排除 errored:
    - 报告自报 outcome 必须精确 completed;
    - native 最高编号 result-N: slug 匹配、outcome 精确 completed、
      frontmatter turn 恰等编号 N;
    - turns.txt 必须存在且同轮 N 精确 completed(缺项/冲突/未来轮拒绝);
    - 报告 turn 恰等 native 最新编号 N(< N stale-turn,> N turn-mismatch);
    - native originator/goal 文件必须与调用视角 session/goal 精确绑定
      (任意外来 native-root 不得冒充 → peer-authority-mismatch);
    - runtime-evidence 终止快照须 session 匹配当前视角;
    - cross 阶段报告 turn 必须晚于冻结轮次(min_turn_exclusive)。
    """
    outcome = fm.get("outcome")
    if outcome != "completed":
        raise fail(
            "peer-outcome-invalid",
            f"{slug} 自报 outcome={outcome}(非 completed),不得作为有效初审/交叉",
        )
    has_authority = False
    # 报告 turn 必须是数字(与权威来源无关;非数字无法对账,fail-closed)。
    fm_turn = fm.get("turn")
    if fm_turn is None or not str(fm_turn).isdigit():
        raise fail(
            "peer-outcome-invalid",
            f"{slug} 报告 turn={fm_turn!r} 缺失/非数字,无法与权威轮次对账",
        )
    fm_turn_n = int(str(fm_turn))
    # External signal 1: runtime-evidence(终止快照须绑定当前 session 视角)
    if runtime_evidence is not None:
        peers = runtime_evidence.get("peers")
        if peers is not None:
            for peer in peers:
                if peer.get("slug") == slug:
                    if peer.get("active_thread"):
                        raise fail(
                            "peer-outcome-invalid",
                            f"{slug} runtime-evidence active_thread 非空"
                            "(仍有活跃轮次),报告声明不可采信",
                        )
                    if (
                        peer.get("status") == "terminated"
                        and expected_session is not None
                        and peer.get("session") == expected_session
                        and (
                            peer.get("goal") is None
                            or peer.get("goal") == expected_goal
                        )
                    ):
                        # 与 native 路径同等严格: 终止快照必须精确 completed
                        # (errored/interrupted 等失败态不是有效终止初审权威),
                        # 且轮次必须可核验 —— source=result-N.md 的 N 即最新
                        # 终止轮,报告 turn 恰等 N(< N 旧轮,> N 未来轮)。
                        ev_outcome = peer.get("outcome")
                        if ev_outcome != "completed":
                            raise fail(
                                "peer-outcome-invalid",
                                f"{slug} runtime-evidence 终止快照 outcome="
                                f"{ev_outcome}(非 completed),不构成终止权威",
                            )
                        ev_source = peer.get("source")
                        src_m = re.match(r"^result-(\d+)\.md$", str(ev_source or ""))
                        if not src_m:
                            # 无可核验轮次来源: 快照自报不可当权威(fail-closed)
                            raise fail(
                                "peer-outcome-invalid",
                                f"{slug} runtime-evidence 终止快照缺可核验轮次来源"
                                f"(source={ev_source!r}),不得当终止权威",
                            )
                        src_n = int(src_m.group(1))
                        if fm_turn_n < src_n:
                            raise fail(
                                "stale-turn",
                                f"{slug} 报告 turn={fm_turn_n} 低于快照来源轮次"
                                f" {src_n}(result-{src_n}.md)",
                            )
                        if fm_turn_n > src_n:
                            raise fail(
                                "turn-mismatch",
                                f"{slug} 报告 turn={fm_turn_n} 超过快照来源轮次"
                                f" {src_n}(未来轮次)",
                            )
                        has_authority = True
    # External signal 2: native result-N.md(严格对账)
    if native_dir is not None and native_dir.exists():
        latest = latest_result(native_dir)
        if latest is not None:
            n, path = latest
            native_fm = parse_frontmatter(path)
            native_slug = native_fm.get("slug")
            native_slug = native_fm.get("slug")
            if native_slug != slug:
                # 缺 slug 键与不匹配同罪: 外来/不可归属收据一律拒
                # (此前 `if native_slug and ...` 使缺键短路跳过 slug 关)。
                raise fail(
                    "peer-authority-mismatch",
                    f"{slug} native {path.name} slug={native_slug!r} 缺失/不匹配",
                )
            native_outcome = native_fm.get("outcome")
            if native_outcome != "completed":
                raise fail(
                    "peer-outcome-invalid",
                    f"{slug} native 最高编号 {path.name} outcome={native_outcome}"
                    "(非 completed),不构成终止权威",
                )
            native_turn = native_fm.get("turn")
            if native_turn is None or not str(native_turn).isdigit() or int(native_turn) != n:
                raise fail(
                    "peer-outcome-invalid",
                    f"{slug} native {path.name} turn={native_turn} 与编号 {n} 不一致",
                )
            idx = turns_index(native_dir)
            if not idx:
                raise fail(
                    "peer-outcome-invalid",
                    f"{slug} native turns.txt 缺失/为空/同轮重复,轮次索引不完整",
                )
            mx = max(idx)
            if n < mx:
                raise fail(
                    "peer-outcome-invalid",
                    f"{slug} native 最高 result-{n} 低于 turns.txt 已结束轮次 {mx}"
                    "(turns 索引不严格)",
                )
            if n not in idx:
                raise fail(
                    "peer-outcome-invalid",
                    f"{slug} turns.txt 缺最新轮 {n} 的记录(缺项不构成终止权威)",
                )
            if idx[n] != "completed":
                raise fail(
                    "peer-outcome-invalid",
                    f"{slug} turns.txt 第 {n} 轮 outcome={idx[n]} 与 {path.name}"
                    " completed 冲突(同轮必须精确 completed)",
                )
            turn = fm.get("turn")
            if turn is None or not str(turn).isdigit():
                raise fail(
                    "peer-outcome-invalid",
                    f"{slug} 报告 turn 缺失/非法,无法与 native 权威对账",
                )
            if int(turn) < n:
                raise fail(
                    "stale-turn",
                    f"{slug} 报告 turn={turn} 低于 native 最新轮次 {n}",
                )
            if int(turn) > n:
                raise fail(
                    "turn-mismatch",
                    f"{slug} 报告 turn={turn} 超过 native 最新轮次 {n}(未来轮次)",
                )
            # originator/goal 身份绑定: 外来 native-root 不得冒充本视角
            if expected_session is not None and not _identity_file_matches(
                native_dir, "originator", expected_session
            ):
                raise fail(
                    "peer-authority-mismatch",
                    f"{slug} native originator 与当前 session 视角不符/缺失"
                    "(外来 native-root 不得冒充)",
                )
            if expected_goal is not None and not _identity_file_matches(
                native_dir, "goal", expected_goal
            ):
                raise fail(
                    "peer-authority-mismatch",
                    f"{slug} native goal 与当前 goal 不符/缺失"
                    "(跨 goal 收据不得作权威)",
                )
            has_authority = True
    turn = fm.get("turn")
    if (
        min_turn_exclusive > 0
        and turn is not None
        and str(turn).isdigit()
        and int(turn) <= min_turn_exclusive
    ):
        raise fail(
            "stale-turn",
            f"{slug} 报告 turn={turn} 不晚于冻结轮次 {min_turn_exclusive}"
            "(旧初审不得冒充 cross)",
        )
    if require_authority and not has_authority:
        raise fail(
            "peer-authority-missing",
            f"{slug} 无外部终止权威(native result-N 或 runtime-evidence 终止快照),"
            " 仅自报 outcome 不足以采信",
        )
    return has_authority


# ---------------------------------------------------------------------------
# behavioral evidence gates
# ---------------------------------------------------------------------------

# necessary structural anchors (NOT sufficient — execution is the real gate)
_ANCHOR_PANIC = re.compile(r"panicked at [^\s:]+:\d+:\d+")
_ANCHOR_SUMMARY = re.compile(r"test result: FAILED\..*\d+ failed")
_PASS_SUMMARY = re.compile(r"test result: ok\.", re.M)


def structural_anchors(ev_path: Path, test_name: str) -> list[str]:
    """Return list of missing structural anchors for a behavioral log."""
    text = ev_path.read_text(encoding="utf-8", errors="replace")
    fail_form = _ANCHOR_PANIC.search(text) and _ANCHOR_SUMMARY.search(text)
    pass_form = _PASS_SUMMARY.search(text)
    missing = []
    if test_name and test_name not in text:
        missing.append(f"测试名行缺失: {test_name}")
    if not fail_form and not pass_form:
        missing.append(
            "行为锚缺失: 需失败形态(panicked at + test result: FAILED. N failed)"
            "或通过形态(test result: ok.)"
        )
    return missing


def test_name_resolvable(test_name: str, harness_roots: list[Path]) -> bool:
    """Test filter name must resolve to a definition in harness sources."""
    needle = re.escape(test_name.split("::")[-1])
    pat = re.compile(r"fn\s+" + needle + r"\s*\(")
    for root in harness_roots:
        if not root.exists():
            continue
        for f in root.rglob("*.rs") if root.is_dir() else [root]:
            try:
                if pat.search(f.read_text(encoding="utf-8", errors="replace")):
                    return True
            except OSError:
                continue
    return False


def production_adapter_path() -> Path:
    """固定的生产 Cargo adapter(本入口唯一可执行体,路径不可由调用方指定)。"""
    return Path(__file__).resolve().with_name("olp-review-evidence-cargo.py")


def load_execution_receipt(ev_path: Path, state_head: str) -> tuple[dict, str, str]:
    """校验本入口刚执行生产 adapter 落盘的 receipt 并做身份绑定校验.

    只用于 --live-cargo 实时执行后的自产 receipt 复核;外部传入的 JSON
    receipt 永不进入本函数(cmd_challenge 前置拒绝/分层 not-replayed)。
    Returns (receipt, observed, selector)。
    """
    try:
        rc = json.loads(ev_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        raise fail("receipt-invalid", f"{ev_path.name} 非合法 JSON,不是执行 receipt")
    if not isinstance(rc, dict) or rc.get("receipt_kind") != "cargo-test-execution":
        raise fail(
            "receipt-invalid",
            f"{ev_path.name} 缺 receipt_kind=cargo-test-execution"
            "(非生产 cargo adapter 执行回执)",
        )
    observed = rc.get("observed")
    if observed not in ("pass", "fail"):
        raise fail(
            "receipt-not-conclusive",
            f"receipt observed={observed} 非终态(pass/fail),不得作行为证据",
        )
    head_before = rc.get("head_before")
    if state_head and re.fullmatch(r"[0-9a-f]{40}", state_head or ""):
        if head_before != state_head:
            raise fail(
                "receipt-head-mismatch",
                f"receipt head_before={head_before} != 评审 HEAD={state_head}",
            )
    if rc.get("head_after") != head_before:
        raise fail(
            "receipt-head-mismatch",
            "执行期间 HEAD 发生变化(head_before != head_after),树不一致",
        )
    if not rc.get("stdout_sha256") or rc.get("exit_code") is None:
        raise fail("receipt-invalid", "receipt 缺输出摘要/退出码绑定")
    selector = rc.get("selector") or ""
    return rc, observed, selector


def run_live_adapter(review_dir: Path, state: dict, args: argparse.Namespace, claim: str) -> dict:
    """--live-cargo: 由本入口实时执行固定的生产 Cargo adapter.

    可靠解析 repo/manifest/精确 selector/HEAD/source 与完整输出工件/退出码
    均由 adapter 保证(其语义经外层 8 真实 PR 探针重放验证,SHA 2e17a359);
    本函数只做参数转发、deadline 约束与自产 receipt 复核,不引入任何
    通用任意 shell 执行旁路。
    """
    adapter = production_adapter_path()
    if not adapter.exists():
        raise fail("adapter-missing", f"生产 cargo adapter 缺失: {adapter}")
    if not args.selector:
        raise fail("live-args-missing", "--live-cargo 需 --selector 精确测试名")
    if args.expect not in ("pass", "fail"):
        raise fail("live-args-missing", "--live-cargo 需 --expect pass|fail(显式预期)")
    repo = Path(args.exec_repo).resolve() if args.exec_repo else Path(state["repo"])
    if not repo.is_dir():
        raise fail("repo-missing", f"live 执行 repo 不存在: {repo}")
    ev_dir = review_dir / "live-evidence"
    ev_dir.mkdir(parents=True, exist_ok=True)
    stamp = f"{claim}-{int(time.time())}-{os.getpid()}"
    receipt_path = ev_dir / f"{stamp}.receipt.json"
    argv = [
        sys.executable or "python3",
        str(adapter),
        "--repo", str(repo),
        "--selector", args.selector,
        "--expect", args.expect,
        "--receipt", str(receipt_path),
        "--artifact-dir", str(ev_dir),
        "--timeout", str(args.timeout),
    ]
    if args.lib:
        argv.append("--lib")
    if args.test_target:
        argv += ["--test-target", args.test_target]
    if args.manifest:
        argv += ["--manifest", args.manifest]
    if args.cargo_target_dir:
        argv += ["--target-dir", args.cargo_target_dir]
    try:
        proc = subprocess.run(
            argv, capture_output=True, text=True, timeout=args.timeout + 120,
        )
    except FileNotFoundError:
        raise fail("adapter-missing", f"无法执行生产 adapter: {argv[0]}")
    except subprocess.TimeoutExpired:
        raise fail(
            "evidence-not-executed",
            f"adapter 执行超时(>{args.timeout + 120}s),已回收子进程",
        )
    expectation_violated = False
    if proc.returncode != 0:
        envelope = {}
        try:
            envelope = json.loads(proc.stdout or "")
        except json.JSONDecodeError:
            envelope = {}
        err = envelope.get("error") if isinstance(envelope, dict) else None
        code = (err or {}).get("code") or "adapter-failed"
        if code == "expectation-violated" and receipt_path.exists():
            # 真实执行但结果与预期相反 — 仍是有意义的行为证据(如实标记)
            expectation_violated = True
        else:
            raise fail(
                "evidence-not-executed",
                f"生产 adapter 未产生可信终态 [{code}]: {(err or {}).get('message', '')}",
            )
    if not receipt_path.exists():
        raise fail("evidence-not-executed", "adapter 未落 receipt,无执行证明")
    rc, observed, selector = load_execution_receipt(receipt_path, state.get("head") or "")
    if selector != args.selector:
        raise fail(
            "receipt-invalid",
            f"receipt selector={selector} 与请求 {args.selector} 不一致",
        )
    return {
        "receipt_path": receipt_path,
        "receipt": rc,
        "observed": observed,
        "expectation_violated": expectation_violated,
    }


# ---------------------------------------------------------------------------
# verdict state machine
# ---------------------------------------------------------------------------

VERDICT_STATES = (
    "approve",
    "flipped",
    "blocked-on-evidence",
    "pending-behavioral-evidence",
    "unverified",
    "not-replayed",
    "challenge-refuted",
)


def claim_evidence_backed(verdict: dict) -> bool:
    """判词是否有可核对证据(本入口真实执行或有效反驳依据)。"""
    if verdict.get("state") == "challenge-refuted":
        return True
    return verdict.get("reason") in EXECUTED_REASONS


def record_challenge(state: dict, claim: str, rec: dict) -> None:
    """每个冻结 claim 独立保存证据/执行结果(latest + history)。"""
    slot = state.setdefault("challenges", {}).setdefault(claim, {"history": []})
    slot["latest"] = rec
    slot.setdefault("history", []).append(rec)
    if rec.get("accepted") is True:
        # 兼容字段: 最近一次被接纳的挑战(旧测试/外层 status 依赖)
        state["challenge"] = rec


def apply_live_verdict(state: dict, claim: str, live: dict) -> str:
    """按 live 执行结果推进判词状态机,返回新状态。"""
    prev = state["verdicts"].get(claim, {}).get("state")
    observed = live["observed"]
    rc = live["receipt"]
    if observed == "fail":
        new_state, reason = "blocked-on-evidence", "challenge-evidence-executed"
    elif prev in ("blocked-on-evidence", "flipped"):
        # 已验证新行为证据: 实时重执行通过,反驳已复现失败
        new_state, reason = "challenge-refuted", "executed-probe-passed"
    else:
        new_state, reason = "approve", "executed-probe-passed"
    rec = {
        "accepted": True,
        "claim": claim,
        "observed": observed,
        "expectation_violated": live["expectation_violated"],
        "receipt": str(live["receipt_path"]),
        "receipt_sha256": sha256_file(live["receipt_path"]),
        "executed": {
            "adapter": rc.get("adapter"),
            "argv": rc.get("argv"),
            "observed": observed,
            "exit_code": rc.get("exit_code"),
            "stdout_sha256": rc.get("stdout_sha256"),
            "stderr_sha256": rc.get("stderr_sha256"),
            "head_before": rc.get("head_before"),
            "head_after": rc.get("head_after"),
            "run_dir": rc.get("cwd"),
            "selector": rc.get("selector"),
            "selector_qualified": rc.get("selector_qualified"),
            "test_target": rc.get("test_target"),
            "manifest_sha256": rc.get("manifest_sha256"),
            "test_target_sha256": rc.get("test_target_sha256"),
            "artifacts": rc.get("artifacts"),
        },
        "executed_unix": int(time.time()),
        "result": "probe-failed" if observed == "fail" else "probe-passed",
    }
    record_challenge(state, claim, rec)
    verdict = {
        "state": new_state,
        "reason": reason,
        "evidence": rec["receipt"],
    }
    if new_state == "challenge-refuted":
        verdict["refuted_by"] = "live-reexecution"
    state["verdicts"][claim] = verdict
    return new_state


# ---------------------------------------------------------------------------
# subcommands
# ---------------------------------------------------------------------------


def cmd_init(args: argparse.Namespace) -> None:
    review_dir = Path(args.review_dir)
    with review_lock(review_dir):
        state = _validated_state_or_fail(load_state(review_dir), review_dir)
        if state:
            raise fail("already-initialized", f"{review_dir} 已初始化")
        # HEAD 真实解析 fail-closed;--head 仅作显式锚定交叉校验。
        head = resolve_head(Path(args.repo))
        if args.head and args.head != head:
            raise fail(
                "head-mismatch",
                f"--head={args.head} 与 repo 真实 HEAD={head} 不一致",
            )
        state = {
            "protocol": PROTOCOL,
            "head": head,
            "base": args.base,
            "runtime": args.runtime,
            "session": args.session,
            "goal": args.goal,
            "repo": str(Path(args.repo).resolve()),
            "reviews": {},
            "frozen": False,
            "challenge": None,
            "challenges": {},
            "cross": [],
            "verdicts": {},
        }
        save_state(review_dir, state)
        emit_json({"ok": True, "state": "initialized", "head": state["head"]})


def _check_report_identity(fm: dict, label: str, state: dict) -> None:
    """初审 frontmatter 身份与调用视角绑定(session/goal 必备;runtime/repo 在则验)。"""
    for key in ("session", "goal"):
        expected = state.get(key)
        v = fm.get(key)
        if not v:
            raise fail(
                "report-identity-missing",
                f"{label} 初审缺 {key} frontmatter(身份缺失不得绕过校验)",
            )
        if expected and v != expected:
            raise fail(
                "report-identity-mismatch",
                f"{label} 初审 {key}={v} != 评审视角 {expected}",
            )
    for key in ("runtime", "repo"):
        expected = state.get(key)
        v = fm.get(key)
        if v and expected:
            if str(Path(v).resolve()) != str(Path(expected).resolve()):
                raise fail(
                    "report-identity-mismatch",
                    f"{label} 初审 {key}={v} != 评审视角 {expected}",
                )


def cmd_freeze(args: argparse.Namespace) -> None:
    review_dir = Path(args.review_dir)
    with review_lock(review_dir):
        state = _validated_state_or_fail(load_state(review_dir), review_dir)
        if not state:
            raise fail("not-initialized", "先 init")
        if state.get("frozen"):
            raise fail("already-frozen", "初审已冻结")

        # --head 显式锚定(不可省略),且必须与 init 基线一致。
        if not args.head:
            raise fail("head-anchor-missing", "freeze 需显式 --head 锚定评审基线")
        if state.get("head") and args.head != state["head"]:
            raise fail(
                "head-mismatch",
                f"--head={args.head} != 评审 HEAD={state['head']}",
            )

        glm_path, k3_path = Path(args.glm_review), Path(args.k3_review)
        for label, p in (("glm", glm_path), ("k3", k3_path)):
            if not p.exists():
                raise fail("first-reviews-incomplete", f"{label} 初审文件缺失: {p}")

        glm_slug, k3_slug = args.glm_slug or "glm", args.k3_slug or "k3"
        if glm_slug == k3_slug or glm_path.resolve() == k3_path.resolve():
            raise fail(
                "duplicate-reviewer",
                "两份初审必须来自不同评审方(同 peer/同文件不得计为两独立报告)",
            )

        glm_fm, k3_fm = parse_frontmatter(glm_path), parse_frontmatter(k3_path)
        native_root = Path(args.native_root) if args.native_root else None
        runtime_evidence = None
        if args.runtime_evidence and Path(args.runtime_evidence).exists():
            runtime_evidence = json.loads(Path(args.runtime_evidence).read_text())

        claims_by_lane: dict[str, list[dict]] = {}
        for label, p, fm, slug in (
            ("glm", glm_path, glm_fm, glm_slug),
            ("k3", k3_path, k3_fm, k3_slug),
        ):
            check_peer_validity(
                fm,
                p,
                (native_root / slug) if native_root else None,
                runtime_evidence,
                slug,
                require_authority=True,
                expected_session=state.get("session"),
                expected_goal=state.get("goal"),
            )
            head = fm.get("HEAD") or fm.get("head")
            if not head:
                raise fail(
                    "report-head-missing",
                    f"{label} 初审缺 HEAD frontmatter(缺失不得绕过校验)",
                )
            if args.head != head:
                raise fail(
                    "head-mismatch",
                    f"{label} 初审 HEAD={head} != --head={args.head}",
                )
            _check_report_identity(fm, label, state)
            claims = parse_claims(p)
            if not claims:
                raise fail(
                    "claims-block-missing",
                    f"{label} 初审缺显式 JSON claims 块(冻结即确立全部初始 claim ID)",
                )
            claims_by_lane[label] = claims

        # 冻结即确立全部初始 claim ID(两 lane 并集,记录 lane 归属)
        verdicts: dict[str, dict] = {}
        claim_lanes: dict[str, list[str]] = {}
        for label, claims in claims_by_lane.items():
            for c in claims:
                cid = c["id"]
                claim_lanes.setdefault(cid, []).append(label)
                if cid not in verdicts:
                    verdicts[cid] = {
                        "state": c.get("verdict", "approve"),
                        "evidence": c.get("evidence", []),
                        "lanes": claim_lanes[cid],
                    }

        # freeze: 记录复合身份并翻转标志 — 同事务一次落盘
        state["reviews"] = {
            "glm": {
                "path": str(glm_path),
                "sha256": glm_fm["__sha256__"],
                "peer": glm_slug,
                "turn": glm_fm.get("turn"),
                "outcome": glm_fm.get("outcome"),
                "native_report": _capture_native_report(native_root, glm_slug, glm_fm.get("turn")),
            },
            "k3": {
                "path": str(k3_path),
                "sha256": k3_fm["__sha256__"],
                "peer": k3_slug,
                "turn": k3_fm.get("turn"),
                "outcome": k3_fm.get("outcome"),
                "native_report": _capture_native_report(native_root, k3_slug, k3_fm.get("turn")),
            },
        }
        state["verdicts"] = verdicts
        state["frozen"] = True
        state["frozen_at_unix"] = int(time.time())
        save_state(review_dir, state)
        emit_json({"ok": True, "state": "frozen", "reviews": state["reviews"],
                   "claims": sorted(verdicts)})


def require_frozen(state: dict) -> None:
    if not state.get("frozen"):
        raise fail("not-frozen", "初审未冻结")


def _record_shape_ok(rec: object, claim: str, where: str, rd: Path) -> None:
    """challenges 记录(latest/history[] 共用)形状门,见 _validated_state_or_fail 注。"""
    if not isinstance(rec, dict):
        raise fail(
            "state-shape-invalid",
            f"state.challenges[{claim}] {where} 非对象: {rd}",
        )
    if "accepted" in rec and not isinstance(rec.get("accepted"), bool):
        raise fail(
            "state-shape-invalid",
            f"state.challenges[{claim}] {where}.accepted 非布尔: {rd}",
        )
    ex = rec.get("executed")
    if ex is not None and not isinstance(ex, dict):
        raise fail(
            "state-shape-invalid",
            f"state.challenges[{claim}] {where}.executed 非对象: {rd}",
        )
    if isinstance(ex, dict):
        art = ex.get("artifacts")
        if art is not None and not isinstance(art, dict):
            raise fail(
                "state-shape-invalid",
                f"state.challenges[{claim}] {where}.executed.artifacts 非对象: {rd}",
            )


def _validated_state_or_fail(state: object, rd: Path) -> dict:
    """共用 state 形状校验(合法 JSON 但结构损坏 → 结构化 JSON 错误)。

    只验实际 schema(state dict/frozen bool/reviews dict+嵌套 path/sha256
    字符串/challenges dict),fail-closed;**不**用笼统 except 吞编程 bug
    —— 形状分支以外的异常照常抛出。classify 的受信上下文加载保留其
    专用错误码(classify-context-*),在 loader 中映射本门的形状错误。
    """
    if not isinstance(state, dict):
        raise fail("state-shape-invalid", f"state 顶层非对象: {rd}")
    if "frozen" in state and not isinstance(state.get("frozen"), bool):
        raise fail("state-shape-invalid", f"state.frozen 非布尔: {rd}")
    if state.get("frozen") is True:
        # 冻结态必有 reviews(freeze 事务写入)与 head;声称冻结却缺是
        # 结构损坏(审计 RED: {"frozen":true} 曾在 status 通过)。
        if "reviews" not in state:
            raise fail(
                "state-shape-invalid",
                f"state.frozen=true 但缺 reviews(结构损坏): {rd}",
            )
        reviews = state.get("reviews")
        if isinstance(reviews, dict):
            for lane in ("glm", "k3"):
                if lane not in reviews:
                    raise fail(
                        "state-shape-invalid",
                        f"state.frozen=true 但 reviews 缺 {lane}(双初审不完整): {rd}",
                    )
        if "head" not in state or not isinstance(state.get("head"), str) or not state["head"]:
            raise fail(
                "state-shape-invalid",
                f"state.frozen=true 但缺合法 head(结构损坏): {rd}",
            )
        # 不变量: freeze 事务恒写 verdicts/challenges/repo;frozen 态
        # 缺任一或类型不符 = 结构损坏,结构化拒绝(不 KeyError)。
        for key in ("verdicts", "challenges"):
            if key not in state or not isinstance(state.get(key), dict):
                raise fail(
                    "state-shape-invalid",
                    f"state.frozen=true 但 {key} 缺失/非对象(结构损坏): {rd}",
                )
        if (
            not isinstance(state.get("repo"), str)
            or not state.get("repo").strip()
        ):
            raise fail(
                "state-shape-invalid",
                f"state.frozen=true 但 repo 缺失/非非空字符串(结构损坏): {rd}",
            )
    if "reviews" in state:
        reviews = state.get("reviews")
        if not isinstance(reviews, dict):
            raise fail("state-shape-invalid", f"state.reviews 非对象: {rd}")
        for label, rec in reviews.items():
            if not isinstance(label, str) or not isinstance(rec, dict):
                raise fail(
                    "state-shape-invalid",
                    f"state.reviews[{label!r}] 形状非法(须对象): {rd}",
                )
            if not isinstance(rec.get("path"), str) or not rec.get("path"):
                raise fail(
                    "state-shape-invalid",
                    f"state.reviews[{label}].path 缺失/非字符串: {rd}",
                )
            if not isinstance(rec.get("sha256"), str) or not rec.get("sha256"):
                raise fail(
                    "state-shape-invalid",
                    f"state.reviews[{label}].sha256 缺失/非字符串: {rd}",
                )
    if "challenges" in state:
        challenges = state.get("challenges")
        if not isinstance(challenges, dict):
            raise fail("state-shape-invalid", f"state.challenges 非对象: {rd}")
        for claim, entry in challenges.items():
            if not isinstance(claim, str) or not claim or not isinstance(entry, dict):
                raise fail(
                    "state-shape-invalid",
                    f"state.challenges[{claim!r}] 形状非法(须对象): {rd}",
                )
            latest = entry.get("latest")
            history = entry.get("history")
            if history is not None and not isinstance(history, list):
                raise fail(
                    "state-shape-invalid",
                    f"state.challenges[{claim}].history 非数组: {rd}",
                )
            # 不变量: latest 与 history 记录同门(_record_shape_ok):
            # accepted 在场必须 bool(显式 null 非法);executed 在场必须
            # dict、其 artifacts 在场必须 dict。所有 accepted 读点统一
            # is True,truthiness 不再作为判真依据。
            for rec, where in ([(latest, "latest")] if latest is not None else []) + [
                (rec, "history[]") for rec in (history or [])
            ]:
                _record_shape_ok(rec, claim, where, rd)
    if "verdicts" in state:
        verdicts = state.get("verdicts")
        if not isinstance(verdicts, dict):
            raise fail("state-shape-invalid", f"state.verdicts 非对象: {rd}")
        for cid, v in verdicts.items():
            if not isinstance(cid, str) or not cid or not isinstance(v, dict):
                raise fail(
                    "state-shape-invalid",
                    f"state.verdicts[{cid!r}] 形状非法(须对象): {rd}",
                )
    if "cross" in state:
        cross = state.get("cross")
        if not isinstance(cross, list):
            raise fail("state-shape-invalid", f"state.cross 非数组: {rd}")
        for c in cross:
            if not isinstance(c, dict):
                raise fail(
                    "state-shape-invalid",
                    f"state.cross 元素非对象: {rd}",
                )
            for field in ("slug", "sha256"):
                if field in c and (not isinstance(c[field], str) or not c[field].strip()):
                    raise fail("state-shape-invalid", f"state.cross.{field} 非非空字符串: {rd}")
            if "turn" in c and _positive_report_turn(c["turn"]) is None:
                raise fail("state-shape-invalid", f"state.cross.turn 非正轮次: {rd}")
    return state


def verify_no_tamper(state: dict) -> None:
    reviews = state.get("reviews") or {}
    for label in REVIEWER_LANES:
        rec = reviews.get(label)
        if not rec:
            continue
        p = Path(rec["path"])
        if not p.exists():
            # fail-closed: 冻结后初审文件被删除与被改写同罪
            raise fail(
                "first-review-tampered",
                f"{label} 初审在冻结后被删除(文件缺失),不予采信",
            )
        if sha256_file(p) != rec["sha256"]:
            raise fail(
                "first-review-tampered",
                f"{label} 初审在冻结后被修改(SHA256 不匹配),不予采信",
            )
    # accepted live 记录全生命周期校验(审计尾项): receipt 本体与输出
    # 工件改写/删除/缺声明/缺 hash → 拒;legacy 快照(缺 executed 内部
    # 字段)由 hash 验证过的 receipt 原件**补齐后再验**(不做"缺字段即
    # 跳过工件校验")—— 事实源是完整 receipt,快照缺项不降低校验强度。
    _verify_live_receipts_untampered(state)


def _verify_live_receipts_untampered(state: dict) -> None:
    """accepted=True history/latest 记录无条件 fail-closed(ROOT-2 裁决)。

    apply_live_verdict 是 accepted=True 的唯一 writer(恒带真实 cargo
    receipt 路径+sha256;imported 记录 accepted=False;operator-refute 改
    verdict/cross 不动 accepted history)。因此:
    - 每条 accepted=True 记录必须 receipt 路径+receipt_sha256 均在且为
      str(删除任一键/字段 → 拒);文件删除/改写 → live-receipt-tampered。
    - 事实源 = hash 验证过的 receipt 原件;快照 executed 缺失字段从
      receipt 重建(legacy 兼容仅限快照内部缺字段),两侧都在且不一致
      → 拒;类型敏感比较(False != 0)。
    - 工件(stdout/stderr): receipt 声明的路径+hash 一律校验;删除或
      改写工件/hash → 拒;两侧(快照与 receipt)声明都在而不一致 → 拒;
      两侧都无工件声明 → 拒(真实 cargo receipt 恒带)。
    - latest 与 history 逐条同验。
    """
    challenges = state.get("challenges") or {}
    if not isinstance(challenges, dict):
        return

    def _check_accepted_rec(claim: str, rec: object, where: str) -> None:
        if not isinstance(rec, dict) or rec.get("accepted") is not True:
            return
        rp_s = rec.get("receipt")
        sha = rec.get("receipt_sha256")
        if not isinstance(rp_s, str) or not rp_s:
            raise fail(
                "live-receipt-tampered",
                f"{claim} {where} accepted 记录缺 receipt 路径(fail-closed)",
            )
        if not isinstance(sha, str) or not sha:
            raise fail(
                "live-receipt-tampered",
                f"{claim} {where} accepted 记录缺 receipt_sha256(fail-closed)",
            )
        rp = Path(rp_s)
        if not rp.is_file():
            raise fail(
                "live-receipt-tampered",
                f"{claim} {where} accepted receipt 被删除(文件缺失): {rp_s}",
            )
        if sha256_file(rp) != sha:
            raise fail(
                "live-receipt-tampered",
                f"{claim} {where} accepted receipt 被改写(SHA256 不匹配): {rp_s}",
            )
        try:
            rc_obj = json.loads(rp.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            raise fail(
                "live-receipt-tampered",
                f"{claim} {where} accepted receipt 不可解析: {rp_s}",
            )
        if not isinstance(rc_obj, dict):
            raise fail(
                "live-receipt-tampered",
                f"{claim} {where} accepted receipt 顶层非对象: {rp_s}",
            )
        if rc_obj.get("receipt_kind") != "cargo-test-execution":
            raise fail(
                "live-receipt-tampered",
                f"{claim} {where} receipt_kind 非 cargo-test-execution: {rp_s}",
            )
        # executed 快照: 在场必须 dict(形状门已拒);整段缺失 = legacy
        # 形态,由 hash 验证过的 receipt 原件单侧重建核心字段校验。
        ex = rec.get("executed") if isinstance(rec.get("executed"), dict) else {}
        # 核心字段类型敏感一致(快照 vs receipt;False != 0)
        for f in (
            "observed", "exit_code", "selector", "selector_qualified",
            "test_target_sha256", "stdout_sha256", "stderr_sha256",
            "head_before", "head_after",
        ):
            rc_v = rc_obj.get(f)
            ex_v = ex.get(f)
            if rc_v is not None and ex_v is not None:
                # 类型敏感比较: bool 是 int 子类,False 不得冒充 0。
                same_type = (
                    type(rc_v) is type(ex_v)
                    or (isinstance(rc_v, (int, float)) and isinstance(ex_v, (int, float))
                        and not isinstance(rc_v, bool) and not isinstance(ex_v, bool))
                )
                if not same_type or rc_v != ex_v:
                    raise fail(
                        "live-receipt-tampered",
                        f"{claim} {where} 核心字段 {f} 快照与 receipt 不一致"
                        f"(类型/值不匹配: {ex_v!r} vs {rc_v!r})",
                    )
            if rc_v is None and ex_v is None and f in (
                "observed", "exit_code", "head_before", "head_after",
            ):
                raise fail(
                    "live-receipt-tampered",
                    f"{claim} {where} 核心字段 {f} 双侧缺失(fail-closed)",
                )
        # 工件: receipt 声明一律校验(真实 cargo receipt 恒带)
        rc_artifacts = rc_obj.get("artifacts")
        ex_artifacts = ex.get("artifacts")
        rc_art = rc_artifacts if isinstance(rc_artifacts, dict) else {}
        ex_art = ex_artifacts if isinstance(ex_artifacts, dict) else {}
        for key in ("stdout", "stderr"):
            art_rc = rc_art.get(key)
            art_ex = ex_art.get(key)
            if (
                isinstance(art_rc, str) and art_rc
                and isinstance(art_ex, str) and art_ex
                and art_rc != art_ex
            ):
                raise fail(
                    "live-receipt-tampered",
                    f"{claim} {where} 执行工件 {key} 快照与 receipt 不一致",
                )
            art = art_rc if isinstance(art_rc, str) and art_rc else art_ex
            if not (isinstance(art, str) and art):
                raise fail(
                    "live-receipt-tampered",
                    f"{claim} {where} receipt 缺执行工件 {key}(fail-closed)",
                )
            declared = rc_obj.get(f"{key}_sha256") or ex.get(f"{key}_sha256")
            if not (isinstance(declared, str) and declared):
                raise fail(
                    "live-receipt-tampered",
                    f"{claim} {where} 缺执行工件 {key} hash(fail-closed)",
                )
            ap = Path(art)
            if not ap.is_file():
                raise fail(
                    "live-receipt-tampered",
                    f"{claim} {where} 执行工件 {key} 被删除(文件缺失): {art}",
                )
            if sha256_file(ap) != declared:
                raise fail(
                    "live-receipt-tampered",
                    f"{claim} {where} 执行工件 {key} 被改写(SHA256 不匹配): {art}",
                )

    for claim, entry in challenges.items():
        if not isinstance(entry, dict):
            continue
        latest = entry.get("latest")
        if latest is not None:
            _check_accepted_rec(claim, latest, "latest")
        history = entry.get("history")
        if isinstance(history, list):
            for i, rec in enumerate(history):
                _check_accepted_rec(claim, rec, f"history[{i}]")


def cmd_challenge(args: argparse.Namespace) -> None:
    review_dir = Path(args.review_dir)
    with review_lock(review_dir):
        state = _validated_state_or_fail(load_state(review_dir), review_dir)
        if not state:
            raise fail("not-initialized", "先 init")
        require_frozen(state)
        verify_no_tamper(state)

        claim = args.claim
        if not claim:
            raise fail("claim-required", "challenge 需 --claim 判词 id")
        # 冻结即确立全部 claim ID — 禁止挑战未冻结/新增 claim
        if claim not in state.get("verdicts", {}):
            raise fail(
                "claim-not-frozen",
                f"claim {claim} 不在冻结 claims 集合中(禁止新增未冻结 claim)",
            )
        if args.live_cargo and args.imported:
            raise fail("args-conflict", "--live-cargo 与 --imported 互斥")

        # imported 分层(先于一切内容/JSON 判定): 外部导入的 receipt/log
        # 一律 not-replayed,--imported 不能旁路执行门。
        if args.imported:
            if not args.evidence:
                raise fail("evidence-missing", "--imported 需 --evidence 导入文件")
            ev_path = Path(args.evidence)
            if not ev_path.exists():
                raise fail("evidence-missing", f"证据文件不存在: {ev_path}")
            state["verdicts"][claim] = {
                "state": "not-replayed",
                "reason": "imported-evidence-not-replayed",
                "evidence": str(ev_path),
            }
            record_challenge(
                state,
                claim,
                {
                    "accepted": False,
                    "imported": True,
                    "claim": claim,
                    "evidence": str(ev_path),
                    "executed_unix": int(time.time()),
                },
            )
            save_state(review_dir, state)
            emit_json(
                {
                    "ok": True,
                    "state": "not-replayed",
                    "claim": claim,
                    "note": "imported 证据未经本入口独立执行,不自动 accepted",
                }
            )
            return

        # 唯一 live 入口: 本入口实时执行固定的生产 Cargo adapter
        if args.live_cargo:
            live = run_live_adapter(review_dir, state, args, claim)
            new_state = apply_live_verdict(state, claim, live)
            save_state(review_dir, state)
            emit_json(
                {
                    "ok": True,
                    "state": new_state,
                    "claim": claim,
                    "observed": live["observed"],
                    "executed": {
                        "exit_code": live["receipt"].get("exit_code"),
                        "receipt": str(live["receipt_path"]),
                    },
                }
            )
            return

        # 非 live 路径: 只承担负向判定 — 任何现成文件都不是执行证明
        if not args.evidence:
            raise fail(
                "evidence-missing",
                "challenge 需 --live-cargo 实时执行,或 --evidence 配合 --imported 分层",
            )
        ev_path = Path(args.evidence)
        if not ev_path.exists():
            raise fail("evidence-missing", f"证据文件不存在: {ev_path}")
        body = ev_path.read_text(encoding="utf-8", errors="replace")

        # 外部 JSON 执行回执: receipt_kind 字符串不是执行证明(外层反例 4/5)
        if ev_path.suffix == ".json":
            try:
                probe = json.loads(body)
            except json.JSONDecodeError:
                probe = None
            if isinstance(probe, dict) and probe.get("receipt_kind"):
                raise fail(
                    "evidence-not-executed",
                    "外部执行回执(receipt_kind)不作执行证明: 行为证据只认本入口 "
                    "--live-cargo 实时执行;外层导入请用 --imported(判 not-replayed)",
                )

        # Gate 1 (cheap, negative): doc-only content 永不被采信
        doc_like = (
            _ANCHOR_PANIC.search(body) is None
            and _ANCHOR_SUMMARY.search(body) is None
            and _PASS_SUMMARY.search(body) is None
        )
        if doc_like:
            state["verdicts"][claim] = {
                "state": "unverified",
                "reason": "evidence-not-behavioral",
            }
            save_state(review_dir, state)
            raise fail(
                "evidence-not-behavioral",
                "证据为文档性内容(字符串常量断言/纯 prose),无论 kind 声明如何均拒绝",
            )

        # Gate 2: structural anchors (necessary, NOT sufficient)
        missing = structural_anchors(ev_path, args.test_name or "")
        if missing:
            state["verdicts"][claim] = {
                "state": "unverified",
                "reason": "evidence-not-behavioral",
            }
            save_state(review_dir, state)
            raise fail("evidence-not-behavioral", "; ".join(missing))

        harness_roots = [Path(p) for p in (args.harness_root or []) if p]
        if args.test_name and harness_roots:
            if not test_name_resolvable(args.test_name, harness_roots):
                raise fail(
                    "evidence-not-behavioral",
                    f"测试名 {args.test_name} 无法在 harness 源解析到定义",
                )

        # Gate 3: --exec-argv 任意执行体一律拒绝(python 打印 cargo 样式文本、
        # /usr/bin/false 等均不作执行证明)。唯一 live 入口是 --live-cargo。
        if args.exec_argv:
            executor = Path(args.exec_argv.split()[0]).name if args.exec_argv.split() else ""
            raise fail(
                "executor-not-trusted",
                f"执行体 {executor or args.exec_argv} 不可信: --exec-argv 不作行为证据;"
                " 唯一 live 入口为 challenge --live-cargo(本入口实时执行固定的"
                " 生产 adapter scripts/olp-review-evidence-cargo.py)",
            )

        # Gate 4: 结构形似不构成执行证明
        raise fail(
            "evidence-not-executed",
            "结构锚齐全但不构成执行证明: 需 --live-cargo 由本入口实时执行生产 adapter",
        )


def cmd_cross(args: argparse.Namespace) -> None:
    review_dir = Path(args.review_dir)
    with review_lock(review_dir):
        state = _validated_state_or_fail(load_state(review_dir), review_dir)
        if not state:
            raise fail("not-initialized", "先 init")
        require_frozen(state)
        verify_no_tamper(state)
        chs = state.get("challenges") or {}
        any_accepted = any(
            (slot.get("latest") or {}).get("accepted") is True
            for slot in chs.values()
        )
        if not any_accepted:
            raise fail("challenge-pending", "挑战证据尚未接纳,不得收录 cross")

        report = Path(args.cross_report)
        if not report.exists():
            raise fail("cross-missing", f"cross 报告不存在: {report}")
        fm = parse_frontmatter(report)
        native_root = Path(args.native_root) if args.native_root else None
        slug = args.cross_slug or ""
        runtime_evidence = None
        if args.runtime_evidence and Path(args.runtime_evidence).exists():
            runtime_evidence = json.loads(Path(args.runtime_evidence).read_text())

        # cross 必须来自冻结的原 reviewer(各自新的 native completed 轮次)
        reviewers = {
            r.get("peer") for r in state.get("reviews", {}).values() if r.get("peer")
        }
        if slug not in reviewers:
            raise fail(
                "cross-reviewer-unknown",
                f"cross slug={slug or '(missing)'} 不是冻结的原 reviewer {sorted(reviewers)}",
            )

        # HEAD 绑定(与初审同一基线,缺失不得绕过)
        head = fm.get("HEAD") or fm.get("head")
        if not head:
            raise fail(
                "report-head-missing",
                "cross 报告缺 HEAD frontmatter(缺失不得绕过校验)",
            )
        if state.get("head") and head != state["head"]:
            raise fail(
                "head-mismatch",
                f"cross 报告 HEAD={head} != 评审 HEAD={state['head']}",
            )

        # 冻结轮下限: 旧 turn1 初审不得冒充 cross turn2;报告 turn 恰等
        # native 最新编号且 originator/goal 身份绑定(与 freeze 同一严格度)
        frozen_turns = [
            int(r["turn"])
            for r in state["reviews"].values()
            if str(r.get("turn") or "").isdigit()
        ]
        min_turn_exclusive = max(frozen_turns, default=0)
        check_peer_validity(
            fm,
            report,
            (native_root / slug) if native_root else None,
            runtime_evidence,
            slug,
            require_authority=True,
            min_turn_exclusive=min_turn_exclusive,
            expected_session=state.get("session"),
            expected_goal=state.get("goal"),
        )

        # 结构化逐 claim 覆盖: 冻结确立的全部 claim ID 必须逐项出现于
        # cross_claims 块(子串包含不算覆盖);--expect-claims 只能扩大校验
        entries = parse_cross_claims(report) or []
        frozen_claims = set(state["verdicts"].keys())
        required = set(frozen_claims)
        if args.expect_claims:
            required |= {c for c in args.expect_claims.split(",") if c}
        covered = {c["id"] for c in entries}
        missing = sorted(required - covered)
        if missing:
            raise fail(
                "missing-claim-coverage",
                f"cross 报告未在结构化 cross_claims 块覆盖判词: {missing}",
            )
        extra = sorted(covered - required)
        if extra:
            raise fail(
                "claim-not-frozen",
                f"cross 引入未冻结 claim: {extra}(冻结即确立全部 claim ID)",
            )

        # refute 回边: 只能基于已验证新行为证据(本入口 live 执行记录)或
        # 显式可审计 operator 决定(--allow-operator-refute);缺有效依据
        # 保持 pending/原失败,不得凭字符串标记升格。
        refutations = []
        for c in entries:
            cid = c["id"]
            if cid not in state["verdicts"] or c["verdict"] != "refute":
                continue
            cur = state["verdicts"][cid].get("state")
            if cur not in ("flipped", "blocked-on-evidence", "challenge-refuted"):
                raise fail(
                    "refutation-without-challenge",
                    f"claim {cid} 当前状态 {cur},无已复现失败可反驳",
                )
            ref = c.get("reference") or {}
            kind = ref.get("kind")
            substantiated = False
            if kind == "executed-evidence":
                want = ref.get("ref") or ""
                history = (state.get("challenges", {}).get(cid) or {}).get("history", [])
                substantiated = any(
                    h.get("accepted") is True
                    and h.get("observed") == "pass"
                    and h.get("receipt_sha256") == want
                    for h in history
                )
            elif kind == "operator-decision":
                substantiated = bool(
                    args.allow_operator_refute and ref.get("operator") and ref.get("note")
                )
            if not substantiated:
                raise fail(
                    "refutation-unsubstantiated",
                    f"claim {cid} refute 缺有效依据: 需已验证新行为证据"
                    "(本入口 live PASS 执行记录 SHA)或显式 --allow-operator-refute"
                    " 的 operator 决定(operator+note 齐全)",
                )
            state["verdicts"][cid] = {
                **state["verdicts"][cid],
                "state": "challenge-refuted",
                "refuted_by": slug,
                "refutation_reference": ref,
            }
            refutations.append(cid)

        # Optional early rejection at collection; final acceptance always
        # requires both initial/cross pairs to retain verifiable model anchors.
        model_evidence = None
        native_report = None
        if args.require_model_evidence:
            native_report = _capture_native_report(native_root, slug, fm.get("turn"))
            model_evidence = _model_pair(state, slug, fm.get("turn"), native_report)
        state.setdefault("cross", []).append(
            {
                "slug": slug,
                "sha256": fm["__sha256__"],
                "turn": fm.get("turn"),
                "refuted": refutations,
                **({"model_evidence": model_evidence} if model_evidence else {}),
                **({"native_report": native_report} if native_report else {}),
            }
        )
        save_state(review_dir, state)
        warnings = []
        if model_evidence is None:
            warning = {
                "code": "model-evidence-not-verified",
                "message": "本次 cross 仅收录用于审计，未核验实际模型，"
                           "最终 review_accepted=false；请带 --require-model-evidence 重新提交",
            }
            warnings.append(warning)
            print(f"warning[{warning['code']}]: {warning['message']}", file=sys.stderr)
        emit_json({"ok": True, "state": "cross-recorded", "refuted": refutations,
                   "warnings": warnings})


def cross_completed_slugs(state: dict) -> set[str]:
    cross = state.get("cross", [])
    if (not isinstance(cross, list)
        or any(not isinstance(c, dict) or not isinstance(c.get("slug"), str)
               or not c["slug"] for c in cross)):
        return set()
    return {c["slug"] for c in cross}


def any_challenge_accepted(state: dict) -> bool:
    chs = state.get("challenges") or {}
    return any(
        (slot.get("latest") or {}).get("accepted") is True
        for slot in chs.values()
    )


def behavior_accepted(state: dict) -> bool:
    """收口条件: 冻结 + ≥1 已接纳挑战 + 两个原 reviewer 各自新 native
    completed cross + 全部冻结 claim 有可核对证据(真实执行或有效反驳)。"""
    if not (state.get("frozen") and any_challenge_accepted(state)):
        return False
    recorded = cross_completed_slugs(state)
    reviewers = {
        r.get("peer") for r in state.get("reviews", {}).values() if r.get("peer")
    }
    if not (len(reviewers) >= 2 and reviewers <= recorded):
        return False
    verdicts = state.get("verdicts", {})
    return all(claim_evidence_backed(v) for v in verdicts.values())


# ---------------------------------------------------------------------------
# PR-level aggregation (classify) — spec L55-73/L439-455
# 分类意图来自外层(MANIFEST outer_recommendation);本工具只做通用聚合派生:
#   * per-selector 产品裁决: observed(pass/fail) + cargo_exit;
#     harness 状态(adapter_exit)与产品失败分列,永不相混。
#   * introduced_vs_existing: 需 BASE/HEAD 同 probe 双执行对照
#     (双 FAIL 形态同构 → existing;HEAD FAIL + BASE PASS → introduced);
#     缺双执行证据 → "unassessed",该 PR 聚合保守降级,禁标 clean。
#   * PR 级: clean(全 pass 且无 unassessed) / residual(失败全 existing) /
#     blocked(反例成立且无双执行对照;或 harness 收据缺失/hash 不符)。
# 禁止按 PR 编号硬编码: 全部遍历 MANIFEST 条目驱动。
# ---------------------------------------------------------------------------

PR_CLASS_STATES = {"clean", "residual", "blocked"}
INTRO_STATES = {"introduced", "existing", "unassessed"}

# cargo test 结果锚(与 adapter scripts/olp-review-evidence-cargo.py 终态判定
# 语义一致):pass 需该 qualified test 的 `... ok` 行 + `test result: ok.` 汇总
# + exit 0;fail 需 `... FAILED` 行 + `test result: FAILED.` 汇总 + exit != 0。
# 汇总文件自报(observed/exit_code)永不足以确立终态,必须核 stdout 实际内容。
_TEST_LINE_RE = re.compile(r"^test (\S+) \.\.\. (ok|FAILED|ignored)", re.M)
_PASS_SUMMARY_RE = re.compile(r"^test result: ok\.", re.M)
_FAIL_SUMMARY_RE = re.compile(r"^test result: FAILED\.", re.M)


def _test_statuses(text: str, qualified: str) -> set[str]:
    """stdout 中该 qualified 测试名的实际逐行结果集合(ok/FAILED/ignored)。

    libtest 可能把状态折到下一行(`test <name> ... \n<panic>\nFAILED`),
    故先取锚行状态;锚行无状态时,若该测试随后 panic 且文本含独立
    `FAILED` 状态行则记 FAILED(adapter 终态判定同语义)。"""
    statuses: set[str] = set()
    anchored = False
    for n, s in _TEST_LINE_RE.findall(text):
        if n == qualified:
            statuses.add(s)
            anchored = True
    if not anchored and f"test {qualified} ..." in text:
        if "panicked at" in text and re.search(r"^FAILED$", text, re.M):
            statuses.add("FAILED")
    return statuses


def _load_json_file(path: Path, err_code: str) -> dict:
    try:
        obj = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as e:
        raise fail(err_code, f"无法读取 {path}: {e}")
    if not isinstance(obj, dict):
        raise fail(err_code, f"{path} 顶层不是对象")
    return obj


def _verify_slot_evidence(
    slot: dict,
    log_dir: Path,
    manifest_heads: dict[str, str],
    manifest_bases: dict[str, str] | None = None,
    qualified_selector: str | None = None,
    probe_sha256: str | None = None,
    trusted: dict | None = None,
) -> dict | None:
    """BASE/HEAD 双执行对照核验(sha256 逐字节),不通过 → None(unassessed)。

    slot 必须含 base_exit/head_exit/classification/base_log_sha256/
    head_log_sha256,且 base/head 日志文件在场、hash 相符;**pr_head 必须
    与 MANIFEST 声明的该 PR head 精确一致**(外来 head 或**另一个真实
    MANIFEST PR 的 head** 均不得作对照);**slot.base 若声明必须精确等于
    该 PR 的 MANIFEST base**(外来 → None);**probe 源文件必须存在且其
    sha256 与 slot.probe_sha256 相符**,且 probe_sha256 == probe_sha256
    参数(= 本 PR receipt 的 test_target_sha256,同 probe 绑定);任何
    缺失/不符 → None(缺双执行证据,fail-closed,不抛异常)。
    manifest_heads/manifest_bases: {pr_key: head/base}(由调用方逐 PR
    传入本 PR 锚;**禁**"等于任一 MANIFEST head"的跨 PR 推广)。
    qualified_selector: 该失败 selector 的 qualified 测试名;hash-bound
    日志必须含该 qualified test 的 `... FAILED` 行 + `test result: FAILED.`
    汇总锚(probe 文件名 stem 模糊匹配不算 same-probe 失败证明)。"""
    try:
        need_ok = (
            isinstance(slot.get("base_exit"), int)
            and isinstance(slot.get("head_exit"), int)
            and isinstance(slot.get("base_log_sha256"), str)
            and isinstance(slot.get("head_log_sha256"), str)
        )
        if not need_ok:
            return None
        pr_head = slot.get("pr_head")
        if not isinstance(pr_head, str) or not pr_head:
            return None
        # 逐 PR 精确绑定: pr_head 必须等于**本 PR** 的 MANIFEST head
        if pr_head not in set(manifest_heads.values()):
            return None
        # slot.base 若声明必须等于本 PR 的 MANIFEST base(外来 → None)
        slot_base = slot.get("base")
        if (
            manifest_bases
            and slot_base is not None
            and slot_base not in set(manifest_bases.values())
        ):
            return None
        # probe 源文件在场 + probe_sha256 绑定(禁文件名回退)
        probe = slot.get("probe")
        if not isinstance(probe, str) or not probe:
            return None
        probe_p = Path(probe)
        if not probe_p.is_file():
            return None
        probe_sha = slot.get("probe_sha256")
        if not isinstance(probe_sha, str) or sha256_file(probe_p) != probe_sha:
            return None
        # receipt.test_target_sha256 == slot.probe_sha256(同 probe 绑定)
        if probe_sha256 is not None and probe_sha != probe_sha256:
            return None
        # 双日志按 sha256 定位(hash-bound),目录内逐文件匹配
        if log_dir is None or not log_dir.is_dir():
            return None
        base_log = head_log = None
        for cand in sorted(log_dir.glob("*.log")):
            digest = sha256_file(cand)
            # 两个独立匹配: 两次真实独立执行完全可能输出相同字节
            # (BASE/HEAD 日志 SHA 相等是合法形态,不得因 elif 只赋
            # base_log 而把 head_log 永远留 None)。任何一门都不放宽。
            if digest == slot["base_log_sha256"]:
                base_log = cand
            if digest == slot["head_log_sha256"]:
                head_log = cand
        if base_log is None or head_log is None:
            return None
        # hash-bound 日志必须真实包含该 qualified 测试的失败锚(非编译失败):
        # `test <qualified> ... FAILED` 行 + `test result: FAILED.` 汇总才
        # 证明"该 probe 在该侧真实执行失败"。禁止 probe 文件名 stem 模糊匹配,
        # 禁止日志内任一 FAILED 匹配——必须锚到本 selector 的 qualified 名。
        try:
            base_text = base_log.read_text(encoding="utf-8", errors="replace")
            head_text = head_log.read_text(encoding="utf-8", errors="replace")
        except OSError:
            return None
        if not isinstance(qualified_selector, str) or not qualified_selector:
            return None
        for text, exit_code in ((base_text, slot["base_exit"]), (head_text, slot["head_exit"])):
            statuses = _test_statuses(text, qualified_selector)
            if (
                "FAILED" not in statuses
                or "ok" in statuses
                or not _FAIL_SUMMARY_RE.search(text)
            ):
                return None
            if exit_code == 0:
                return None
        # Blocker1(ROOT design-review #3): BASE/HEAD 双执行证据两侧都必须
        # 绑定到受信上下文注册的真实执行 —— slot 携带 base_receipt/
        # head_receipt 路径,各自须: (a) provenance 命中注册(路径 canonical
        # 相等); (b) executed.head_before == slot 对应 commit(manifest
        # base/head 已由上方逐 PR 门锚定); (c) 同 qualified selector +
        # test_target_sha256 == probe_sha256; (d) observed=fail 且 exit 真
        # int 非零; (e) 其 stdout_sha256 == slot 对应 log_sha(防注册 BASE
        # PASS/异 selector + 手写 FAIL 日志冒充)。任一不满足 → None。
        trusted = trusted or {}
        slot_base = slot.get("base")
        bound: dict[str, dict] = {}
        for side, commit, log_sha in (
            ("base_receipt", slot_base, slot["base_log_sha256"]),
            ("head_receipt", pr_head, slot["head_log_sha256"]),
        ):
            trec, _tres = _receipt_provenance_ok(slot.get(side), trusted)
            if trec is None:
                return None
            ex = trec["executed"]
            if ex.get("head_before") != commit:
                return None
            if qualified_selector is not None and ex.get("selector_qualified") != qualified_selector:
                return None
            if probe_sha256 is not None and ex.get("test_target_sha256") != probe_sha256:
                return None
            if ex.get("observed") != "fail":
                return None
            ex_exit = ex.get("exit_code")
            if isinstance(ex_exit, bool) or not isinstance(ex_exit, int) or ex_exit == 0:
                return None
            if ex.get("stdout_sha256") != log_sha:
                return None
            bound[side] = {
                "receipt": trec["receipt"],
                "receipt_sha256": trec["receipt_sha256"],
                "review_dir": trec["review_dir"],
            }
        return {
            "base_exit": slot["base_exit"],
            "head_exit": slot["head_exit"],
            "pr_head": pr_head,
            "probe": probe,
            "probe_sha256": probe_sha,
            "classification": str(slot.get("classification", "")),
            "base_log": str(base_log),
            "head_log": str(head_log),
            "base_receipt": bound.get("base_receipt"),
            "head_receipt": bound.get("head_receipt"),
        }
    except (OSError, ValueError, AttributeError, TypeError):
        # 任何证据读取异常都按缺证据处理(fail-closed),绝不崩溃
        return None


def _verify_product_receipt(
    r: dict, manifest_head: str | None
) -> tuple[dict | None, str | None]:
    """逐条校验 per-selector 产品裁决记录: receipt 在场且与 summary 一致。

    返回 (verified, harness_reason): verified 为 dict 时该记录可信
    (observed=pass → pass 记录;observed=fail → 产品失败记录)。
    - receipt_kind 必须**精确** `cargo-test-execution`(外来/伪造收据拒);
    - receipt.selector == summary.selector;matched_tests 的末段必须含
      selector(qualified selector 绑定实际 probe);
    - receipt.head_before == summary.head == head_after == MANIFEST 该 PR
      head(三方 HEAD 绑定);
    - observed 一致(fail/pass);fail 时 exit_code == summary.cargo_exit ≠ 0;
    - stdout/stderr 工件路径在场且 sha256 与 receipt 声明相符;
    - 任一不符 → harness 证据错误(missing/foreign/mismatched 一律
      blocked/unassessed,不得继承 existing/residual;不静默弱化 pass)。
    """
    sel = r.get("selector")
    head = r.get("head")
    cargo_exit = r.get("cargo_exit")
    observed = r.get("observed")
    receipt_path = r.get("receipt")
    if not isinstance(sel, str) or not sel:
        return None, "summary-selector-missing"
    if not isinstance(receipt_path, str) or not receipt_path:
        return None, f"{sel}:summary-receipt-path-missing"
    rp = Path(receipt_path)
    if not rp.is_file():
        return None, f"{sel}:receipt-file-missing"
    try:
        rc = json.loads(rp.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None, f"{sel}:receipt-unreadable"
    if not isinstance(rc, dict):
        return None, f"{sel}:receipt-not-object"
    if rc.get("receipt_kind") != "cargo-test-execution":
        return None, f"{sel}:receipt-kind-not-cargo-test-execution({rc.get('receipt_kind')!r})"
    if rc.get("selector") != sel:
        return None, f"{sel}:receipt-selector-mismatch({rc.get('selector')!r})"
    matched = rc.get("matched_tests")
    if not isinstance(matched, list) or not matched:
        return None, f"{sel}:receipt-matched-tests-missing"
    if not any(
        isinstance(mt, str) and (mt == sel or mt.rsplit("::", 1)[-1] == sel)
        for mt in matched
    ):
        return None, f"{sel}:receipt-matched-tests-not-selector"
    if observed not in ("pass", "fail"):
        return None, f"{sel}:summary-observed-invalid({observed!r})"
    if rc.get("observed") != observed:
        return None, f"{sel}:receipt-observed-mismatch({rc.get('observed')!r})"
    if not isinstance(head, str) or not head:
        return None, f"{sel}:summary-head-missing"
    if rc.get("head_before") != head or rc.get("head_after") != head:
        return None, f"{sel}:receipt-head-mismatch"
    if manifest_head is not None and head != manifest_head:
        return None, f"{sel}:summary-head-not-manifest({head[:8]}!={manifest_head[:8]})"
    if observed == "fail":
        if isinstance(cargo_exit, bool) or not isinstance(cargo_exit, int) or cargo_exit == 0:
            return None, f"{sel}:summary-fail-with-zero-exit"
        if rc.get("exit_code") != cargo_exit:
            return None, f"{sel}:receipt-exit-mismatch"
    else:
        if rc.get("exit_code") != 0:
            return None, f"{sel}:receipt-pass-with-nonzero-exit"
        # PR#632 P2-B: pass 的 summary cargo_exit 必须为真 int 0(bool
        # False 冒充 0 一并拒绝)且与 receipt 一致 —— 非零/null/布尔
        # (矛盾)不得当可信 pass(归 harness 证据错误)。
        if isinstance(cargo_exit, bool) or cargo_exit != 0:
            return None, f"{sel}:summary-pass-with-cargo-exit({cargo_exit!r})"
    # stdout/stderr 工件在场 + hash 绑定
    artifacts = rc.get("artifacts")
    if not isinstance(artifacts, dict):
        return None, f"{sel}:receipt-artifacts-missing"
    for key in ("stdout", "stderr"):
        art = artifacts.get(key)
        if not isinstance(art, str) or not art:
            return None, f"{sel}:receipt-artifact-{key}-missing"
        ap = Path(art)
        if not ap.is_file():
            return None, f"{sel}:receipt-artifact-{key}-file-missing"
        declared = rc.get(f"{key}_sha256")
        if not isinstance(declared, str) or sha256_file(ap) != declared:
            return None, f"{sel}:receipt-artifact-{key}-hash-mismatch"
    # 产品终态不信 summary/receipt 自报: 核 stdout 实际内容,复用 adapter
    # 终态判定语义(olp-review-evidence-cargo.py)。pass 需该 qualified test
    # 的 `... ok` 行 + `test result: ok.` 汇总 + exit 0;fail 需 `... FAILED`
    # 行 + `test result: FAILED.` 汇总 + exit != 0。真 FAILED stdout 改标
    # pass/exit0 → harness 证据错误(blocked/unassessed),不得当 pass。
    qualified = rc.get("selector_qualified")
    if not isinstance(qualified, str) or not qualified:
        matched_q = [mt for mt in matched if isinstance(mt, str)]
        qualified = matched_q[0] if len(matched_q) == 1 else None
    if not qualified:
        return None, f"{sel}:receipt-qualified-selector-missing"
    try:
        stdout_text = Path(artifacts["stdout"]).read_text(
            encoding="utf-8", errors="replace"
        )
    except OSError:
        return None, f"{sel}:receipt-artifact-stdout-unreadable"
    statuses = _test_statuses(stdout_text, qualified)
    if observed == "pass":
        if not (
            "ok" in statuses
            and "FAILED" not in statuses
            and _PASS_SUMMARY_RE.search(stdout_text)
        ):
            return None, f"{sel}:receipt-pass-claim-contradicted-by-stdout"
    else:
        if not (
            "FAILED" in statuses
            and "ok" not in statuses
            and _FAIL_SUMMARY_RE.search(stdout_text)
        ):
            return None, f"{sel}:receipt-fail-claim-contradicted-by-stdout"
    verified = {
        "selector": sel,
        "observed": observed,
        "adapter_exit": r.get("adapter_exit"),
        "cargo_exit": cargo_exit if observed == "fail" else 0,
        "receipt": receipt_path,
        "receipt_selector_qualified": qualified,
        "receipt_matched_tests": matched,
        "receipt_test_target_sha256": rc.get("test_target_sha256"),
    }
    return verified, None


def _slot_probes_selector(slot: dict, slot_log_dir: Path | None) -> list[str]:
    """解析 slot probe 源文件实际包含的测试函数名(same probe 判定锚)。

    probe 文件(.rs)内 `fn <name>(` 形态的测试名列表。**probe 文件必须
    在场**(slot 核验已做 probe_sha256 绑定;此处只读该文件)——文件
    缺失时返回空(无文件名回退,same probe 无法证明 → unassessed)。"""
    probe = slot.get("probe")
    if not isinstance(probe, str) or not probe:
        return []
    p = Path(probe)
    if not p.is_file():
        # 无路径名回退: probe 源不在场 = 无法解析 same-probe 函数名
        return []
    try:
        text = p.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return []
    names = re.findall(r"\bfn\s+([A-Za-z0-9_]+)\s*\(", text)
    return names


def _qualified_selector_from(
    slot: dict, selector: str, receipt_selector_qualified: str | None
) -> str | None:
    """same-probe 判定锚: receipt.selector_qualified 精确 qualified 测试名。

    优先使用 receipt 的 selector_qualified(qualified selector 绑定实际
    probe);仅当其末段确实等于本 selector 时采用。缺失时回退到唯一
    fn 名 == selector 的 probe 源声明(probe 已经 sha256 绑定);仍无法
    确定 → None(same probe 无法证明 → unassessed,不用文件名回退)。"""
    if isinstance(receipt_selector_qualified, str) and receipt_selector_qualified:
        if receipt_selector_qualified.rsplit("::", 1)[-1] == selector:
            return receipt_selector_qualified
        return None
    fns = _slot_probes_selector(slot, None)
    if selector in fns:
        return selector
    return None


def _slot_proves_existing_for_selector(
    slot: dict,
    slot_log_dir: Path | None,
    selector: str,
    receipt_matched_tests: list | None,
) -> bool:
    """slot 双执行证据是否**精确匹配该 selector**(same probe)。

    禁止按 exit_code 相等或"任一 head 匹配"把 slot 的 existing 证据推广
    到整 PR: 只有 slot probe 源文件声明的测试函数(fn 名)与该失败
    selector 同名(或 receipt matched_tests 含之)时才可归 existing。
    """
    if not selector:
        return False
    probe_fns = set(_slot_probes_selector(slot, slot_log_dir))
    if selector in probe_fns:
        return True
    if isinstance(receipt_matched_tests, list):
        for mt in receipt_matched_tests:
            if isinstance(mt, str) and (
                mt == selector or mt.rsplit("::", 1)[-1] == selector
            ) and mt.rsplit("::", 1)[-1] in probe_fns:
                return True
    return False


def aggregate_pr_classification(
    manifest_path: Path,
    replay_summary_path: Path,
    slot_path: Path | None,
    slot_log_dir: Path | None,
    trusted: dict | None = None,
) -> dict:
    """通用 PR 级聚合。遍历 MANIFEST prs,消费 replay per-selector 裁决与
    (如在场)BASE/HEAD 双执行 slot;不出现任何 PR 编号条件分支。"""
    trusted = trusted or {}
    manifest = _load_json_file(manifest_path, "classify-manifest-invalid")
    prs = manifest.get("prs")
    if not isinstance(prs, dict) or not prs:
        raise fail("classify-manifest-invalid", "MANIFEST 缺 prs 对象")
    replay = _load_json_file(replay_summary_path, "classify-replay-invalid")
    results = replay.get("results")
    if not isinstance(results, list) or not results:
        raise fail("classify-replay-invalid", "replay summary 缺 results 数组")

    # MANIFEST 每条 PR 的 head/base 声明(receipt 三方 HEAD 绑定 + slot
    # pr_head/base 逐 PR 精确绑定的锚)
    manifest_heads: dict[str, str] = {}
    manifest_bases: dict[str, str] = {}
    for pr_key, meta in prs.items():
        if isinstance(meta, dict) and isinstance(meta.get("head"), str) and meta["head"]:
            manifest_heads[pr_key] = meta["head"]
        if isinstance(meta, dict) and isinstance(meta.get("base"), str) and meta["base"]:
            manifest_bases[pr_key] = meta["base"]

    # slot 原始 JSON 只在文件在场时读取一次;**核验逐 PR 逐失败 selector
    # 进行**(pr_head 必须等于本 PR 的 MANIFEST head,而非任一 MANIFEST
    # head),不在循环外做"任一 head 匹配"的预核验。
    slot_raw: dict | None = None
    if slot_path is not None and slot_path.is_file():
        slot_raw = _load_json_file(slot_path, "classify-slot-invalid")

    # per-selector 索引: pr 字符串键 → [产品裁决记录]
    by_pr: dict[str, list[dict]] = {}
    for r in results:
        if not isinstance(r, dict):
            continue
        pr_key = str(r.get("pr", ""))
        if not pr_key:
            continue
        by_pr.setdefault(pr_key, []).append(r)

    pr_classes: dict[str, dict] = {}
    for pr_key, meta in prs.items():
        if not isinstance(meta, dict):
            continue
        selectors = by_pr.get(pr_key, [])
        product_fails: list[dict] = []
        harness_faults: list[dict] = []
        all_pass = bool(selectors)
        for r in selectors:
            observed = r.get("observed")
            adapter_exit = r.get("adapter_exit")
            cargo_exit = r.get("cargo_exit")
            verified, harness_reason = _verify_product_receipt(
                r, manifest_heads.get(pr_key)
            )
            if verified is not None:
                # Blocker1: receipt 来源必须命中受信上下文注册
                trec, treason = _receipt_provenance_ok(r.get("receipt"), trusted)
                if trec is None:
                    verified = None
                    harness_reason = treason or "receipt-untrusted"
            # Python bool 是 int 子类: adapter_exit/cargo_exit 必须是真
            # int —— False 冒充 0(True 冒充 1)与字段一致性相悖,拒绝。
            adapter_exit_int = (
                isinstance(adapter_exit, int) and not isinstance(adapter_exit, bool)
            )
            cargo_exit_int = (
                isinstance(cargo_exit, int) and not isinstance(cargo_exit, bool)
            )
            if (
                verified is not None
                and observed == "fail"
                and adapter_exit_int
                and adapter_exit == 0
                and cargo_exit_int
                and cargo_exit != 0
            ):
                product_fails.append(verified)
            elif (
                observed == "pass"
                and verified is not None
                and adapter_exit_int
                and adapter_exit == 0
                and cargo_exit_int
                and cargo_exit == 0
            ):
                # PR#632 P2-B: pass 记录同样要求 adapter 成功 + cargo exit 0
                # —— adapter 非零(harness 故障)或 summary cargo_exit 非 0/
                # 缺失(与 pass 声明矛盾)均不得当可信 pass,归 harness
                # 证据错误;混合 existing fail + 坏 pass → blocked/unassessed。
                continue
            else:
                # harness 侧故障(adapter 未跑完/收据缺失/不一致/外来伪装)
                # ≠ 产品失败;missing/foreign/mismatched 归 harness 证据错误,
                # 该 PR 保守 blocked/unassessed。
                harness_faults.append({
                    "selector": r.get("selector"),
                    "observed": observed,
                    "adapter_exit": adapter_exit,
                    "cargo_exit": cargo_exit,
                    "reason": harness_reason or "summary-fields-invalid",
                })
                all_pass = False
        if not selectors or harness_faults:
            all_pass = False

        intro = "unassessed"
        slot_bound = None
        # 双执行对照只证明 slot 声明的 same probe,且**逐失败 selector 逐 PR
        # 精确绑定**重新核验:slot.pr_head 必须等于**本 PR** 的 MANIFEST head
        # (不是任一 MANIFEST head),slot.base 若声明须等于本 PR MANIFEST
        # base(外来 → None),receipt.test_target_sha256 == slot.probe_sha256,
        # 且 hash-bound base/head 日志含该 selector 的 qualified 测试确切
        # FAILED 锚。禁止按 exit_code 相等或任一 head 匹配把 existing
        # 推广到整 PR 其他 selector。PR 归 existing/residual 当且仅当全部
        # 失败 selector 都被同一 slot 的 same-probe 双执行覆盖。
        if product_fails and slot_raw is not None:
            covered_slots: list[dict | None] = []
            for f in product_fails:
                qualified = _qualified_selector_from(
                    slot_raw,
                    f["selector"],
                    f.get("receipt_selector_qualified"),
                )
                if not qualified:
                    covered_slots.append(None)
                    continue
                verified_slot = _verify_slot_evidence(
                    slot_raw,
                    slot_log_dir or (slot_path.parent if slot_path else None),
                    {pr_key: manifest_heads.get(pr_key)},
                    {pr_key: manifest_bases.get(pr_key)},
                    qualified,
                    f.get("receipt_test_target_sha256"),
                    trusted=trusted,
                )
                if (
                    verified_slot is not None
                    and verified_slot["base_exit"] != 0
                    and verified_slot["head_exit"] != 0
                    and "existing" in verified_slot["classification"]
                ):
                    covered_slots.append(verified_slot)
                else:
                    covered_slots.append(None)
            covered = [s is not None for s in covered_slots]
            if all(covered):
                slot0 = covered_slots[0]
                intro = "existing"
                slot_bound = {
                    "pr_head": slot0["pr_head"],
                    "probe": slot0["probe"],
                    "base_exit": slot0["base_exit"],
                    "head_exit": slot0["head_exit"],
                    "classification": slot0["classification"],
                    "base_log": slot0["base_log"],
                    "head_log": slot0["head_log"],
                    "covered_selectors": [
                        f["selector"] for f, c in zip(product_fails, covered) if c
                    ],
                }
            elif any(covered):
                # 部分覆盖: 已覆盖 selector 可记 existing 证据,但 PR 整体
                # 不可归 residual(存在无 same-probe 证据的失败)→ blocked。
                slot0 = next(s for s in covered_slots if s is not None)
                slot_bound = {
                    "pr_head": slot0["pr_head"],
                    "probe": slot0["probe"],
                    "base_exit": slot0["base_exit"],
                    "head_exit": slot0["head_exit"],
                    "classification": slot0["classification"],
                    "base_log": slot0["base_log"],
                    "head_log": slot0["head_log"],
                    "covered_selectors": [
                        f["selector"] for f, c in zip(product_fails, covered) if c
                    ],
                    "partial": True,
                }

        if product_fails or harness_faults:
            # 反例/证据故障成立: 只有(全部失败 selector 均被 slot same-probe
            # 双执行覆盖且非 partial)且无 harness 证据错误才 residual;
            # 否则 blocked(缺失/foreign/mismatched 收据不得继承 existing)。
            if (
                slot_bound is not None
                and not slot_bound.get("partial")
                and intro == "existing"
                and not harness_faults
            ):
                pr_state = "residual"
            else:
                pr_state = "blocked"
                if harness_faults and intro == "existing":
                    # 有 harness 证据错误时 existing 不可采信 → 保守降级
                    intro = "unassessed"
        elif all_pass:
            pr_state = "clean"
        else:
            pr_state = "blocked"

        # introduced_vs_existing 只能从真实 same-probe 双执行对照派生。
        # 当前生产仅支持"双 FAIL 形态同构 → existing"这一种对照模式;
        # "BASE PASS + HEAD FAIL → introduced"是冻结词表中的保留语义,
        # **尚未有对应的双执行证据模式支撑,不会产出**——缺证据一律
        # unassessed。frozen spec L70-72: unassessed 一律不得 clean
        # (无例外): 全 pass 但缺必需对照证据同样 blocked,直到真实
        # same-probe BASE/HEAD 对照允许更强分类;不发明诊断。
        if intro == "unassessed" and pr_state == "clean":
            pr_state = "blocked"

        pr_classes[pr_key] = {
            "classification": pr_state,
            "introduced_vs_existing": intro,
            "outer_recommendation": meta.get("outer_recommendation"),
            "product_fail_selectors": [f["selector"] for f in product_fails],
            "harness_fault_selectors": [h["selector"] for h in harness_faults],
            "dual_execution_evidence": slot_bound,
            "recommendation_aligned": (
                (pr_state == "clean")
                == (meta.get("outer_recommendation") == "approve")
                if isinstance(meta.get("outer_recommendation"), str) and meta["outer_recommendation"]
                else None
            ),
        }

    return {
        "protocol": PROTOCOL,
        "aggregation": "pr-classification",
        "manifest": str(manifest_path),
        "replay_summary": str(replay_summary_path),
        "slot": (str(slot_path) if slot_path else None),
        "prs": pr_classes,
    }


# ---------------------------------------------------------------------------
# Blocker1(PR#632 human HOLD): classify 可信来源绑定(provenance)。
# 一致性 ≠ 来源: 自洽的伪造 MANIFEST/summary/receipt/slot/日志不再可信。
# 信任来源 = 调用方显式给出的受信评审上下文(--review-dir,可重复)中,
# 经 challenge --live-cargo 真实落盘并 accepted 的记录(challenges[*].
# history);不建第二注册表、不从 summary/manifest 发现目录。
# ---------------------------------------------------------------------------


def _load_trusted_live_records(review_dirs: list[Path]) -> dict:
    """从显式受信评审上下文收集 accepted live 记录,按 receipt sha256 索引。

    每个上下文必须: review-state.json 可解析为对象、frozen、reviews 必填
    且嵌套形状合法、verify_no_tamper 通过、真实 repo HEAD 与 state.head
    一致。读取在 review_lock(与 freeze/challenge/cross 同一 flock)临界
    区内进行,防止与并发写状态竞态。记录必须 accepted==true 且 executed
    形状完整;receipt 路径 canonical 相等且位于该上下文 live-evidence/
    内、文件 sha256 与记录一致。任何畸形 → 结构化 JSON 错误
    fail-closed(不扩旧 parser 范围)。
    """
    index: dict[str, dict] = {}
    for rd in review_dirs:
        if not rd.is_dir():
            raise fail("classify-context-missing", f"受信上下文不存在: {rd}")
        raw = (rd / STATE_FILENAME).read_text(encoding="utf-8") if (rd / STATE_FILENAME).exists() else None
        if raw is None:
            raise fail(
                "classify-context-missing",
                f"受信上下文缺 review-state.json: {rd}",
            )
        try:
            state = json.loads(raw)
        except json.JSONDecodeError as e:
            raise fail(
                "classify-context-invalid",
                f"受信上下文 state 非合法 JSON: {e}: {rd}",
            )
        try:
            state = _validated_state_or_fail(state, rd)
        except ReviewError as e:
            raise fail(
                "classify-context-invalid",
                f"受信上下文 state 形状非法: {e.message}",
            )
        if state.get("frozen") is not True:
            raise fail(
                "classify-context-missing",
                f"受信上下文未冻结(不可作信任来源): {rd}",
            )
        # 形状守卫(fail-closed,结构化 JSON): verify_no_tamper 对
        # reviews 嵌套形状有假设,畸形输入须在此结构化拒绝而非 traceback。
        reviews = state.get("reviews")
        if not isinstance(reviews, dict) or not reviews:
            raise fail(
                "classify-context-invalid",
                f"受信上下文 state.reviews 缺失/形状非法(须非空对象): {rd}",
            )
        for label, rec in reviews.items():
            if not isinstance(label, str) or not isinstance(rec, dict):
                raise fail(
                    "classify-context-invalid",
                    f"受信上下文 state.reviews[{label!r}] 形状非法(须对象): {rd}",
                )
            if not isinstance(rec.get("path"), str) or not rec.get("path"):
                raise fail(
                    "classify-context-invalid",
                    f"受信上下文 state.reviews[{label}].path 缺失/非法: {rd}",
                )
            if not isinstance(rec.get("sha256"), str) or not rec.get("sha256"):
                raise fail(
                    "classify-context-invalid",
                    f"受信上下文 state.reviews[{label}].sha256 缺失/非法: {rd}",
                )
        # 真实 repo HEAD 与 state.head 一致(bounded helper,锚定归属)
        repo_s = state.get("repo")
        state_head = state.get("head")
        if not isinstance(repo_s, str) or not repo_s or not isinstance(state_head, str) or not state_head:
            raise fail(
                "classify-context-invalid",
                f"受信上下文 state.repo/head 缺失/非法: {rd}",
            )
        try:
            real_head = resolve_head(Path(repo_s))
        except ReviewError:
            raise fail(
                "classify-context-invalid",
                f"受信上下文 repo HEAD 不可解析: {rd}",
            )
        if real_head != state_head:
            raise fail(
                "classify-context-invalid",
                f"受信上下文 repo 真实 HEAD 与 state.head 不符: {rd}",
            )
        # 注: 调用方(cmd_classify)已按稳定顺序持有全部上下文的
        # review_lock(ExitStack 覆盖 load+aggregate);此处不再内嵌加锁,
        # 防同一排他 flock 经不同 fd 二次获取导致自死锁。
        try:
            verify_no_tamper(state)
        except ReviewError:
            raise fail(
                "classify-context-missing",
                f"受信上下文 state 校验失败(tamper): {rd}",
            )
        chs = state.get("challenges") or {}
        if not isinstance(chs, dict):
            raise fail(
                "classify-context-invalid",
                f"受信上下文 challenges 形状非法: {rd}",
            )
        live_dir = (rd / "live-evidence").resolve()
        for claim, entry in chs.items():
            if not isinstance(claim, str) or not isinstance(entry, dict):
                raise fail(
                    "classify-context-invalid",
                    f"受信上下文 challenges 条目形状非法: {rd}",
                )
            for rec in entry.get("history") or []:
                if not isinstance(rec, dict) or rec.get("accepted") is not True:
                    continue
                ex = rec.get("executed")
                receipt_s = rec.get("receipt")
                sha = rec.get("receipt_sha256")
                if (
                    not isinstance(ex, dict)
                    or not isinstance(receipt_s, str)
                    or not receipt_s
                    or not isinstance(sha, str)
                    or not sha
                ):
                    raise fail(
                        "classify-context-invalid",
                        f"accepted live 记录形状非法: {rd}",
                    )
                rp = Path(receipt_s)
                try:
                    rp_resolved = rp.resolve()
                except OSError:
                    raise fail(
                        "classify-context-invalid",
                        f"receipt 路径不可解析: {receipt_s}",
                    )
                # 路径同一性: canonical 相等 + live-evidence 目录包含
                try:
                    rp_resolved.relative_to(live_dir)
                except ValueError:
                    raise fail(
                        "classify-context-invalid",
                        f"accepted receipt 不在受信 live-evidence 内: {receipt_s}",
                    )
                if not rp_resolved.is_file():
                    raise fail(
                        "classify-context-invalid",
                        f"accepted receipt 文件缺失: {receipt_s}",
                    )
                if sha256_file(rp_resolved) != sha:
                    raise fail(
                        "classify-context-invalid",
                        f"accepted receipt 与注册 sha256 不符(篡改): {receipt_s}",
                    )
                # 注册 receipt 的实际 head(双侧)必须等于所属上下文
                # state.head —— 防跨上下文/跨 HEAD 注册件混入。
                try:
                    rc_head_obj = json.loads(rp_resolved.read_text(encoding="utf-8"))
                except (OSError, json.JSONDecodeError):
                    rc_head_obj = {}
                if not isinstance(rc_head_obj, dict):
                    rc_head_obj = {}
                if (
                    rc_head_obj.get("head_before") != state.get("head")
                    or rc_head_obj.get("head_after") != state.get("head")
                ):
                    raise fail(
                        "classify-context-invalid",
                        f"accepted receipt head 与所属上下文 state.head 不符: {receipt_s}",
                    )
                # 受信注册件 = sha 锚定的 receipt 原件: state 快照缺字段时
                # 以 receipt 本身为准(observed/exit/selector 等,均被
                # sha256 完整性覆盖;防旧快照无 observed 导致注册失真)。
                try:
                    rc_obj = json.loads(rp_resolved.read_text(encoding="utf-8"))
                except (OSError, json.JSONDecodeError):
                    rc_obj = {}
                if not isinstance(rc_obj, dict):
                    rc_obj = {}
                # receipt(hash 锚定原件)为基底;state 快照 ex 与 receipt 的
                # **核心执行字段**两侧都在且不一致 → 结构化拒绝(不静默以
                # 任一侧覆盖);legacy 记录缺字段时回退 receipt 实测值。
                _CORE_FIELDS = (
                    "observed", "exit_code", "selector",
                    "selector_qualified", "test_target_sha256",
                    "stdout_sha256", "head_before", "head_after",
                )
                for _f in _CORE_FIELDS:
                    if (
                        _f in rc_obj
                        and rc_obj[_f] is not None
                        and _f in ex
                        and ex[_f] is not None
                        and rc_obj[_f] != ex[_f]
                    ):
                        raise fail(
                            "classify-context-invalid",
                            f"accepted 记录与 receipt 核心字段 {_f} 不一致: {receipt_s}",
                        )
                merged_ex = {k: v for k, v in rc_obj.items()}
                merged_ex.update({k: v for k, v in ex.items() if v is not None})
                index[sha] = {
                    "review_dir": str(rd),
                    "receipt": str(rp_resolved),  # canonical(resolved)
                    "receipt_sha256": sha,
                    "state_head": state.get("head"),
                    "executed": merged_ex,
                }
    return index


def _receipt_provenance_ok(
    receipt_path: str | None, trusted: dict
) -> tuple[dict | None, str | None]:
    """summary 记录的 receipt 是否来自受信上下文注册。

    返回 (trusted_record, reason): sha256 必须命中注册索引(防外部/未注册
    receipt),且注册路径 canonical 相等(防外部同内容副本绕过
    live-evidence 边界)。
    """
    if not isinstance(receipt_path, str) or not receipt_path:
        return None, "summary-receipt-path-missing"
    rp = Path(receipt_path)
    if not rp.is_file():
        return None, "receipt-file-missing"
    sha = sha256_file(rp)
    rec = trusted.get(sha)
    if rec is None:
        return None, "receipt-not-registered-in-trusted-context"
    try:
        resolved = rp.resolve()
    except OSError:
        return None, "receipt-path-unresolvable"
    if str(resolved) != rec["receipt"]:
        return None, "receipt-path-not-registered-copy"
    return rec, None


def cmd_classify(args: argparse.Namespace) -> None:
    manifest_path = Path(args.manifest)
    if not manifest_path.is_file():
        raise fail("classify-manifest-invalid", f"MANIFEST 不存在: {manifest_path}")
    replay_path = Path(args.replay_summary)
    if not replay_path.is_file():
        raise fail("classify-replay-invalid", f"replay summary 不存在: {replay_path}")
    slot_path = Path(args.slot) if getattr(args, "slot", None) else None
    slot_log_dir = Path(args.slot_log_dir) if getattr(args, "slot_log_dir", None) else None
    # 受信上下文: canonical 去重 + 稳定排序(锁序确定,防交叉死锁);
    # 存在性先验 —— 缺失须在 review_lock(review_lock 会 mkdir)之前失败。
    raw_dirs = [Path(p) for p in (getattr(args, "review_dir", None) or [])]
    canonical: list[Path] = []
    for p in raw_dirs:
        try:
            rp = p.resolve()
        except OSError:
            raise fail("classify-context-missing", f"受信上下文路径不可解析: {p}")
        if rp not in canonical:
            canonical.append(rp)
    canonical.sort(key=lambda x: str(x))
    for rp in canonical:
        if not rp.is_dir():
            raise fail("classify-context-missing", f"受信上下文不存在: {rp}")
    # ExitStack 按稳定顺序持有全部上下文的 review_lock,临界区覆盖
    # load(_load_trusted_live_records)与 aggregate(receipt/artifact 再读)
    # —— 与 freeze/challenge/cross 共享同一 flock 协议;loader 内部不再
    # 加锁(防同 flock 二次排他获取自死锁)。
    with contextlib.ExitStack() as stack:
        for rp in canonical:
            stack.enter_context(review_lock(rp))
        trusted = _load_trusted_live_records(canonical)
        emit_json(
            aggregate_pr_classification(
                manifest_path, replay_path, slot_path, slot_log_dir, trusted=trusted
            )
        )


def review_accepted(state: dict) -> bool:
    return behavior_accepted(state) and models_verified(state)


def build_status(state: dict) -> dict:
    verdicts = {k: dict(v) for k, v in (state.get("verdicts") or {}).items()}
    # 两模型一致不能替代行为实验: 无本入口执行依据的 approve 逐 claim
    # 降级为暂态(仅 challenge 一次不代表所有 claim)
    for c, v in verdicts.items():
        if v.get("state") == "approve" and v.get("reason") != "executed-probe-passed":
            verdicts[c] = {**v, "state": "pending-behavioral-evidence"}
    model_verified = models_verified(state)
    behavior_verified = behavior_accepted(state)
    return {
        "protocol": PROTOCOL,
        "frozen": state.get("frozen", False),
        "challenge_accepted": any_challenge_accepted(state),
        "review_accepted": behavior_verified and model_verified,
        "behavior_accepted": behavior_verified,
        "model_verified": model_verified,
        "verdicts": verdicts,
        "cross": state.get("cross", []),
        "identity": {
            k: state.get(k)
            for k in ("head", "base", "runtime", "session", "goal", "repo")
        },
    }


def cmd_status(args: argparse.Namespace) -> None:
    review_dir = Path(args.review_dir)
    if not review_dir.is_dir() or not (review_dir / STATE_FILENAME).exists():
        raise fail("not-initialized", "先 init")
    # 与 freeze/challenge/cross 同一 flock 读取临界区(单次持锁;不与
    # classify 的 ExitStack 外层重复嵌套同一锁),保证 verify 与渲染基于
    # 一致快照。
    with review_lock(review_dir, read_only=True):
        state = _validated_state_or_fail(load_state(review_dir), review_dir)
        if not state:
            raise fail("not-initialized", "先 init")
        verify_no_tamper(state)
        emit_json(build_status(state))


# ---------------------------------------------------------------------------
# human rendering (--format human)
# ---------------------------------------------------------------------------


def render_human(obj: dict) -> str:
    if "error" in obj:
        e = obj["error"]
        return f"错误 [{e.get('code')}]: {e.get('message')}"
    lines = []
    ident = obj.get("identity") or {}
    lines.append("== olp-review-evidence 状态 ==")
    if ident:
        lines.append(
            f"identity: head={ident.get('head')} base={ident.get('base')} "
            f"runtime={ident.get('runtime')} session={ident.get('session')} "
            f"goal={ident.get('goal')}"
        )
    lines.append(f"lifecycle: frozen={obj.get('frozen')} "
                 f"challenge_accepted={obj.get('challenge_accepted')} "
                 f"review_accepted={obj.get('review_accepted')} "
                 f"model_verified={obj.get('model_verified')}")
    verdicts = obj.get("verdicts") or {}
    if verdicts:
        lines.append("verdicts:")
        for cid, v in sorted(verdicts.items()):
            lines.append(f"  - {cid}: {v.get('state')} ({v.get('reason', '')})")
    cross = obj.get("cross") or []
    if cross:
        lines.append("cross: " + ", ".join(c.get("slug", "?") for c in cross))
    if not ident and not verdicts and obj.get("state"):
        lines.append(f"state: {obj.get('state')}")
    if obj.get("observed"):
        lines.append(f"observed: {obj.get('observed')}")
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="olp-review-evidence")
    p.add_argument("--format", choices=["json", "human"], default="json")
    sub = p.add_subparsers(dest="cmd", required=True)

    sp = sub.add_parser("init")
    sp.add_argument("review_dir")
    sp.add_argument("--repo", default=".")
    sp.add_argument("--head")
    sp.add_argument("--base", required=True)
    sp.add_argument("--runtime", required=True)
    sp.add_argument("--session", required=True)
    sp.add_argument("--goal", required=True)
    sp.set_defaults(func=cmd_init)

    sp = sub.add_parser("freeze")
    sp.add_argument("review_dir")
    sp.add_argument("--glm-review", required=True)
    sp.add_argument("--k3-review", required=True)
    sp.add_argument("--glm-slug")
    sp.add_argument("--k3-slug")
    sp.add_argument("--head")
    sp.add_argument("--native-root")
    sp.add_argument("--runtime-evidence")
    sp.set_defaults(func=cmd_freeze)

    common = lambda sp: (
        sp.add_argument("review_dir"),
        sp.add_argument("--glm-review"),
        sp.add_argument("--k3-review"),
    )

    sp = sub.add_parser("challenge")
    common(sp)
    sp.add_argument("--evidence", help="证据文件(非 live 路径仅作负向判定/--imported 分层)")
    sp.add_argument("--claim", required=True)
    sp.add_argument("--kind")  # hint only — never a gate
    sp.add_argument("--test-name")
    sp.add_argument("--harness-root", action="append")
    sp.add_argument("--exec-argv", help="已移除: 任意执行体一律 executor-not-trusted")
    sp.add_argument("--run-dir", help="兼容保留,不再使用")
    sp.add_argument("--timeout", type=int, default=600, help="live 执行超时(秒)")
    sp.add_argument("--imported", action="store_true",
                    help="外部导入证据: 一律 not-replayed,不能旁路执行门")
    # 唯一 live 入口: 本入口实时执行固定的生产 Cargo adapter
    sp.add_argument("--live-cargo", action="store_true",
                    help="由本入口实时执行固定的生产 adapter(唯一 live 入口)")
    sp.add_argument("--selector", help="live: 精确测试名(裸名或全限定)")
    sp.add_argument("--expect", choices=["pass", "fail"], help="live: 显式预期结果")
    sp.add_argument("--exec-repo", help="live: 执行 repo(真实 HEAD 须等于评审 HEAD;默认评审 repo)")
    sp.add_argument("--lib", action="store_true", help="live: --lib 单元测试入口")
    sp.add_argument("--test-target", help="live: 集成测试 target")
    sp.add_argument("--manifest", help="live: Cargo.toml 路径(须归属执行 repo)")
    sp.add_argument("--cargo-target-dir", help="live: 转发 adapter --target-dir")
    sp.set_defaults(func=cmd_challenge)

    sp = sub.add_parser("cross")
    common(sp)
    sp.add_argument("--cross-report", required=True)
    sp.add_argument("--cross-slug")
    sp.add_argument("--native-root")
    sp.add_argument("--runtime-evidence")
    sp.add_argument("--expect-claims")
    sp.add_argument("--allow-operator-refute", action="store_true",
                    help="显式允许 operator-decision 型 refute(可审计人工裁决)")
    sp.add_argument("--require-model-evidence", action="store_true",
                    help="cross 收录前经 runtime ledger token_cost_update 核验 reviewer 实际模型(fail-closed)")
    sp.set_defaults(func=cmd_cross)

    sp = sub.add_parser("status")
    common(sp)
    sp.set_defaults(func=cmd_status)

    cp = sub.add_parser("classify")
    cp.add_argument("--manifest", required=True,
                    help="外层 MANIFEST.json(分类意图 outer_recommendation)")
    cp.add_argument("--replay-summary", required=True,
                    help="生产 adapter 重放 summary(per-selector observed/exit)")
    cp.add_argument("--review-dir", dest="review_dir", action="append", default=[],
                    help="受信评审上下文(可重复): 其 accepted live receipt 才是"
                         "分类的执行来源;至少一个,否则全部 unassessed")
    cp.add_argument("--slot", default=None,
                    help="BASE/HEAD 双执行 slot 收据(存在才可能 existing/residual)")
    cp.add_argument("--slot-log-dir", dest="slot_log_dir", default=None,
                    help="slot 双日志目录(sha256 逐字节核验)")
    cp.set_defaults(func=cmd_classify)
    return p


def main(argv: list[str] | None = None) -> int:
    global CLI_HUMAN_FORMAT
    parser = build_parser()
    args = parser.parse_args(argv)
    CLI_HUMAN_FORMAT = getattr(args, "format", "json") == "human"
    human = CLI_HUMAN_FORMAT
    try:
        args.func(args)
        return 0
    except ReviewError as e:
        if human:
            sys.stdout.write(render_human({"error": {"code": e.code, "message": e.message}}) + "\n")
        else:
            emit_json({"error": {"code": e.code, "message": e.message}})
        return 1
    except BrokenPipeError:
        return 1


if __name__ == "__main__":
    sys.exit(main())
