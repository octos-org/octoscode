#!/usr/bin/env python3
# ARCHIVED (OUTER_LOOP_REVIEW #31): superseded by the Rust implementation —
# `octoscode olp-mcp-serve` (src/olp_mcp.rs + src/cmd/olp_mcp.rs), tested by
# tests/olp_mcp_contract.rs. Kept as the campaign-night reference only; the
# live profile mount points at the octoscode binary, not this script.
#!/usr/bin/env python3
"""OLP-MCP outer-loop server (#29 S1, specs/task-req-olp-mcp.spec.md).

The inner loop's fifth channel: a synchronous ask-the-outer-loop tool
surface mounted via `octos mcp` (MCP stdio, newline-delimited JSON-RPC —
the frame format rmcp's TokioChildProcess speaks, verified against
octos/crates/octos-agent/src/mcp.rs connect_stdio).

Design decisions (pinned in the spec; do not reopen):
- exactly two tools: ask_outer(question, context, tried),
  report_blocked(reason, needs). No request_review.
- mailbox: questions -> ~/.octos/outer/mcp/questions/<ts>-<id>.json,
  answers polled from answers/<id>.json; 90s timeout returns degraded
  guidance, never blocks a turn indefinitely.
- audit: every Q/A (ask, answer, timeout, refusal) goes through
  ~/.octos/outer/board_append.sh signed MCP(ask_outer).
- anti-outsourcing: max 3 ask_outer calls per slice (per server
  process); `tried` is mandatory; over-quota is refused BEFORE the
  mailbox write.
- pure Python 3 stdlib, no network, no LLM credentials.
"""

import json
import logging
import os
import subprocess
import sys
import time
import uuid
from pathlib import Path

PROTOCOL_VERSION = "2024-11-05"
SERVER_NAME = "olp-mcp"
SERVER_VERSION = "0.1.0"
SIGNATURE = "MCP(ask_outer)"
ASK_TIMEOUT_SECS = 90.0
ASK_POLL_INTERVAL_SECS = 0.5
ASK_QUOTA_PER_SLICE = 3
BOARD_RELATIVE = "OUTER_LOOP_MCP.md"

DEGRADED_GUIDANCE = (
    "外环 90s 未应答(超时降级):按黑板既有指导继续推进;若确实无法推进,"
    "以 ACK(blocked) 收场并注明本次问询 id,外环会在线信箱补答。"
)
QUOTA_REFUSAL = (
    "拒绝:本切片 ask_outer 问询已达 3 次上限(防思考外包)。请按黑板既有"
    "指导自行推进,或以 report_blocked 直通落板请求外环介入。"
)
TRIED_REFUSAL = (
    "拒绝:tried 字段为空。防思考外包纪律要求先自行尝试——把已试过的"
    "路径写进 tried 再问。"
)

log = logging.getLogger("olp-mcp")


def outer_root() -> Path:
    """Mailbox root — overridable for tests; runtime default ~/.octos/outer."""
    override = os.environ.get("OLP_MCP_OUTER_ROOT")
    if override:
        return Path(override)
    return Path.home() / ".octos" / "outer"


def mailbox_dirs(root: Path) -> tuple:
    questions = root / "mcp" / "questions"
    answers = root / "mcp" / "answers"
    consumed = root / "mcp" / "consumed"
    questions.mkdir(parents=True, exist_ok=True)
    answers.mkdir(parents=True, exist_ok=True)
    consumed.mkdir(parents=True, exist_ok=True)
    return questions, answers, consumed


def board_append(root: Path, text: str) -> bool:
    """Append to the outer-loop board via board_append.sh (stdin body,
    board path argv). Degrades to a local log line when the script is
    absent — audit must never crash the server."""
    script = root / "board_append.sh"
    board = root / BOARD_RELATIVE
    if not script.exists():
        log.warning("board_append.sh missing at %s; entry logged only", script)
        return False
    try:
        subprocess.run(
            ["bash", str(script), str(board)],
            input=text,
            text=True,
            check=True,
            timeout=10,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        return True
    except (subprocess.SubprocessError, OSError) as err:
        log.warning("board_append failed: %s", err)
        return False


def audit_entry(root: Path, kind: str, detail: str) -> None:
    entry = f"\n- {time.strftime('%Y-%m-%d %H:%M:%S')} {SIGNATURE} {kind}: {detail}\n"
    board_append(root, entry)


TOOLS = [
    {
        "name": "ask_outer",
        "description": (
            "Ask the outer loop a question and wait (<=90s) for the answer "
            "via the mailbox. Quota: 3 per slice; `tried` is mandatory."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "question": {"type": "string", "description": "The question for the outer loop"},
                "context": {"type": "string", "description": "Where you are stuck / relevant state"},
                "tried": {"type": "string", "description": "What you already tried (must be non-empty)"},
            },
            "required": ["question", "context", "tried"],
        },
    },
    {
        "name": "report_blocked",
        "description": (
            "Report a blocker straight to the outer-loop board (no mailbox "
            "round-trip)."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "reason": {"type": "string", "description": "Why you are blocked"},
                "needs": {"type": "string", "description": "What you need to unblock"},
            },
            "required": ["reason", "needs"],
        },
    },
]


class OlcpMcpServer:
    def __init__(self, root: Path, timeout_secs: float = ASK_TIMEOUT_SECS,
                 poll_interval: float = ASK_POLL_INTERVAL_SECS):
        self.root = root
        self.questions_dir, self.answers_dir, self.consumed_dir = mailbox_dirs(root)
        self.timeout_secs = timeout_secs
        self.poll_interval = poll_interval
        self.ask_count = 0

    # ---- tool implementations -------------------------------------------

    def ask_outer(self, args: dict) -> str:
        question = (args.get("question") or "").strip()
        context = (args.get("context") or "").strip()
        tried = (args.get("tried") or "").strip()

        if not tried:
            audit_entry(self.root, "refusal", "ask_outer rejected: empty tried")
            return TRIED_REFUSAL
        if self.ask_count >= ASK_QUOTA_PER_SLICE:
            audit_entry(self.root, "refusal",
                        f"ask_outer rejected: quota {ASK_QUOTA_PER_SLICE} exhausted")
            return QUOTA_REFUSAL
        if not question:
            return "拒绝:question 为空。"

        self.ask_count += 1
        ask_id = uuid.uuid4().hex[:12]
        ts = time.strftime("%Y%m%dT%H%M%S")
        payload = {
            "id": ask_id,
            "ts": ts,
            "question": question,
            "context": context,
            "tried": tried,
        }
        question_path = self.questions_dir / f"{ts}-{ask_id}.json"
        question_path.write_text(
            json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
        audit_entry(self.root, "ask",
                    f"id={ask_id} question={question[:200]}")

        answer_path = self.answers_dir / f"{ask_id}.json"
        deadline = time.monotonic() + self.timeout_secs
        while time.monotonic() < deadline:
            if answer_path.exists():
                try:
                    answer = json.loads(
                        answer_path.read_text(encoding="utf-8"))
                    text = answer.get("answer", "")
                    audit_entry(self.root, "answer",
                                f"id={ask_id} answered ({len(text)} chars)")
                    # S3: archive the consumed pair so questions/ holds only
                    # unanswered asks (the outer side's queue view) and the
                    # audit trail keeps the full history.
                    archive = self.consumed_dir / f"{ts}-{ask_id}.json"
                    archive.write_text(json.dumps(
                        {"question": payload, "answer": answer},
                        ensure_ascii=False, indent=2), encoding="utf-8")
                    question_path.unlink(missing_ok=True)
                    answer_path.unlink(missing_ok=True)
                    return text
                except (json.JSONDecodeError, OSError) as err:
                    log.warning("answer unreadable: %s", err)
            time.sleep(self.poll_interval)

        audit_entry(self.root, "timeout", f"id={ask_id} no answer in {self.timeout_secs:.0f}s")
        return f"{DEGRADED_GUIDANCE}\n(问询 id: {ask_id})"

    def report_blocked(self, args: dict) -> str:
        reason = (args.get("reason") or "").strip()
        needs = (args.get("needs") or "").strip()
        if not reason:
            return "拒绝:reason 为空。"
        audit_entry(self.root, "blocked",
                    f"reason={reason[:200]} needs={needs[:200]}")
        return "已直通落板(署名 MCP(ask_outer)),外环会在黑板看到。"

    # ---- JSON-RPC plumbing ------------------------------------------------

    def handle_request(self, request: dict) -> dict:
        method = request.get("method")
        req_id = request.get("id")

        if method == "initialize":
            return {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION},
                },
            }
        if method in ("notifications/initialized", "initialized"):
            return None  # notification, no response
        if method == "tools/list":
            return {"jsonrpc": "2.0", "id": req_id, "result": {"tools": TOOLS}}
        if method == "tools/call":
            params = request.get("params") or {}
            name = params.get("name")
            args = params.get("arguments") or {}
            if name == "ask_outer":
                text = self.ask_outer(args)
            elif name == "report_blocked":
                text = self.report_blocked(args)
            else:
                return {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "error": {"code": -32602,
                              "message": f"unknown tool: {name!r} (surface is exactly two tools)"},
                }
            return {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "content": [{"type": "text", "text": text}],
                    "isError": False,
                },
            }
        if method == "ping":
            return {"jsonrpc": "2.0", "id": req_id, "result": {}}
        if req_id is None:
            return None  # unknown notification
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "error": {"code": -32601, "message": f"method not found: {method}"},
        }

    def serve_stdio(self, stdin, stdout) -> None:
        """Newline-delimited JSON-RPC — the frame format rmcp's
        TokioChildProcess (used by `octos mcp`) speaks."""
        for line in stdin:
            line = line.strip()
            if not line:
                continue
            try:
                request = json.loads(line)
            except json.JSONDecodeError:
                stdout.write(json.dumps({
                    "jsonrpc": "2.0", "id": None,
                    "error": {"code": -32700, "message": "parse error"},
                }) + "\n")
                stdout.flush()
                continue
            response = self.handle_request(request)
            if response is not None:
                stdout.write(json.dumps(response, ensure_ascii=False) + "\n")
                stdout.flush()


# ---- self-test -------------------------------------------------------------


def _rpc(method: str, params=None, req_id=1) -> str:
    frame = {"jsonrpc": "2.0", "id": req_id, "method": method}
    if params is not None:
        frame["params"] = params
    return json.dumps(frame)


def self_test() -> int:
    """Seven pins, one per spec Scenario selector. Uses a temp outer root
    (never the real mailbox) and a compressed clock."""
    import io
    import tempfile

    results = []

    def check(name: str, ok: bool, detail: str = "") -> None:
        results.append((name, ok, detail))
        print(f"{'PASS' if ok else 'FAIL'} {name}{(' — ' + detail) if detail else ''}")

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        # Fake board_append.sh recording invocations.
        board_log = root / "board_calls.log"
        fake_script = root / "board_append.sh"
        fake_script.write_text(
            "#!/usr/bin/env bash\ncat >> \"$(dirname \"$1\")/board_calls.log\"\n",
            encoding="utf-8")
        fake_script.chmod(0o755)

        server = OlcpMcpServer(root, timeout_secs=0.6, poll_interval=0.05)

        def call(method, params=None, req_id=1):
            stdin = io.StringIO(_rpc(method, params, req_id) + "\n")
            stdout = io.StringIO()
            server.serve_stdio(stdin, stdout)
            out = stdout.getvalue().strip()
            return json.loads(out) if out else None

        # 1. initialize handshake
        resp = call("initialize", {"protocolVersion": PROTOCOL_VERSION,
                                   "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}})
        ok = (resp and resp.get("result", {}).get("protocolVersion") == PROTOCOL_VERSION
              and "tools" in resp["result"].get("capabilities", {})
              and resp["result"]["serverInfo"]["name"] == SERVER_NAME)
        check("self_test_initialize_handshake", bool(ok))

        # 2. tools/list exactly two
        resp = call("tools/list")
        tools = (resp or {}).get("result", {}).get("tools", [])
        names = sorted(t["name"] for t in tools)
        ok = (names == ["ask_outer", "report_blocked"]
              and all("inputSchema" in t and t["inputSchema"].get("required") for t in tools))
        check("self_test_tools_list_exactly_two", bool(ok), f"tools={names}")

        # 3. ask_outer roundtrip (fake answer injected by a writer thread)
        import threading

        def answer_writer():
            # wait for the question file, then write the answer
            deadline = time.monotonic() + 2
            while time.monotonic() < deadline:
                qs = list(server.questions_dir.glob("*-*.json"))
                if qs:
                    payload = json.loads(qs[0].read_text(encoding="utf-8"))
                    (server.answers_dir / f"{payload['id']}.json").write_text(
                        json.dumps({"answer": "外环答:走候选 b"}, ensure_ascii=False),
                        encoding="utf-8")
                    return
                time.sleep(0.02)

        writer = threading.Thread(target=answer_writer, daemon=True)
        writer.start()
        resp = call("tools/call", {"name": "ask_outer", "arguments": {
            "question": "驱逐释放选 a 还是 b?", "context": "#20 评审两方向",
            "tried": "读过 model.rs 7408 与 store.rs 错误臂"}})
        writer.join(timeout=2)
        text = resp["result"]["content"][0]["text"] if resp else ""
        # S3: the consumed pair is archived and questions/ is drained.
        questions = list(server.questions_dir.glob("*-*.json"))
        consumed = list(server.consumed_dir.glob("*-*.json"))
        board_text = board_log.read_text(encoding="utf-8") if board_log.exists() else ""
        ok = (text == "外环答:走候选 b" and len(consumed) == 1 and len(questions) == 0
              and json.loads(consumed[0].read_text(encoding="utf-8"))["question"]["tried"]
              and json.loads(consumed[0].read_text(encoding="utf-8"))["answer"]["answer"] == "外环答:走候选 b"
              and SIGNATURE in board_text and "ask" in board_text)
        check("self_test_ask_outer_roundtrip", bool(ok), f"answer={text!r}")

        # 4. timeout degrades (no answer present; compressed clock)
        server2 = OlcpMcpServer(root, timeout_secs=0.4, poll_interval=0.05)
        start = time.monotonic()
        text = server2.ask_outer({"question": "无人作答的问题", "context": "x",
                                  "tried": "已自查"})
        elapsed = time.monotonic() - start
        board_text = board_log.read_text(encoding="utf-8") if board_log.exists() else ""
        ok = ("超时降级" in text and elapsed < 2.0 and "timeout" in board_text
              and "ACK(blocked)" in text)
        check("self_test_ask_outer_timeout_degrades", bool(ok),
              f"elapsed={elapsed:.2f}s")

        # 5. quota refusal (4th ask on one slice/server)
        server3 = OlcpMcpServer(root, timeout_secs=0.1, poll_interval=0.02)
        for i in range(ASK_QUOTA_PER_SLICE):
            server3.ask_outer({"question": f"q{i}", "context": "c", "tried": "t"})
        before = len(list(server3.questions_dir.glob("*-*.json")))
        refusal = server3.ask_outer({"question": "q4", "context": "c", "tried": "t"})
        after = len(list(server3.questions_dir.glob("*-*.json")))
        ok = ("已达 3 次上限" in refusal and before == after)
        check("self_test_ask_outer_quota_refusal", bool(ok))

        # 6. tried mandatory
        server4 = OlcpMcpServer(root, timeout_secs=0.1, poll_interval=0.02)
        before = len(list(server4.questions_dir.glob("*-*.json")))
        refusal = server4.ask_outer({"question": "q", "context": "c", "tried": "  "})
        after = len(list(server4.questions_dir.glob("*-*.json")))
        ok = ("tried" in refusal and "拒绝" in refusal and before == after)
        check("self_test_ask_outer_requires_tried", bool(ok))

        # 7. report_blocked goes straight to the board, no mailbox file
        before_q = len(list(server.questions_dir.glob("*-*.json")))
        resp = call("tools/call", {"name": "report_blocked", "arguments": {
            "reason": "cargo 窗口被占", "needs": "外环代跑验证"}}, req_id=99)
        after_q = len(list(server.questions_dir.glob("*-*.json")))
        board_text = board_log.read_text(encoding="utf-8") if board_log.exists() else ""
        text = resp["result"]["content"][0]["text"] if resp else ""
        ok = ("已直通落板" in text and before_q == after_q
              and "blocked" in board_text and "cargo 窗口被占" in board_text)
        check("self_test_report_blocked_board_only", bool(ok))

    failed = [name for name, ok, _ in results if not ok]
    print(f"\n{len(results) - len(failed)}/{len(results)} self-tests passed")
    return 1 if failed else 0


def main() -> int:
    logging.basicConfig(stream=sys.stderr, level=logging.WARNING,
                        format="olp-mcp: %(levelname)s %(message)s")
    if "--self-test" in sys.argv:
        return self_test()
    server = OlcpMcpServer(outer_root())
    server.serve_stdio(sys.stdin, sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())
