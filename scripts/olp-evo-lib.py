"""Shared evolution-loop library (#43c-1, SDD contract v2
specs/task-req-olp-evo-p2.spec.md).

No CLI. Imported by scripts/olp-evo-retro.sh (and, from 43c-2 on, the
metrics script) so parsing / grouping / normalization / anchoring / layer
hints live in exactly one place.
"""

from __future__ import annotations

import sys

# 43-r2: never leave __pycache__ behind, whoever imports this module.
sys.dont_write_bytecode = True

import json
import re

# --- trigger classes ----------------------------------------------------
BOARD_TRIGGERS = ("ack_blocked", "ack_wontdo", "override", "r2_record")
EVENTS_TRIGGERS = (
    "escalation",
    "turn_error",
    "goal_blocked",
    "goal_budget_limited",
    "fallback_switch",
    "malformed_exhausted",
)

ACK_PREFIXES = ("ACK(blocked):", "ACK(wontdo):")

LAYER_TABLE = {
    "ack_blocked": "Lifecycle",
    "ack_wontdo": "Lifecycle",
    "goal_blocked": "Lifecycle",
    "goal_budget_limited": "Lifecycle",
    "escalation": "Lifecycle",
    "report_blocked": "Tooling",
    "ask_outer_timeout": "Tooling",
    "malformed_exhausted": "Tooling",
    "turn_error": "Execution",
    "fallback_switch": "Execution",
    "override": "Governance",
    "r2_record": "Verification",
}

_RE_SIGNED_PREFIX = re.compile(r"^外环\([^)]*\)·(?:改判|R2 记档)\([^)]*\)[:：]\s*")
_RE_PATH = re.compile(r"(?<![\w])/[^\s]+")
_RE_HEX = re.compile(r"(?<![A-Za-z\d])[0-9a-f]{8,}(?![A-Za-z\d])")
_RE_NUM = re.compile(r"(?<![A-Za-z_\d])\d+(?![A-Za-z\d])")
_RE_FAILOVER = re.compile(r"router failover: (\S+) -> (\S+)")


# --- parsing ------------------------------------------------------------
def parse_cards(text: str) -> list[dict]:
    """Parse evolution-board cards. Fields default to None; the caller
    decides what is missing (malformed)."""
    cards: list[dict] = []
    cur: dict | None = None
    for line in text.split("\n"):
        if line.startswith("### EVO-"):
            if cur is not None:
                cards.append(cur)
            num = line[len("### EVO-") :].split("（")[0]
            cur = {
                "id": f"EVO-{num}",
                "num": int(num) if num.isdigit() else 0,
                "trigger": None,
                "identity": None,
                "symptom": None,
                "source": "-",
                "envelope": "-",
                "raw": line,
            }
            continue
        if cur is None:
            continue
        for field in ("trigger:", "identity:", "symptom:", "source:", "envelope:"):
            if line.startswith(field):
                cur[field[:-1]] = line[len(field) :].strip()
    if cur is not None:
        cards.append(cur)
    return cards


# --- grouping text ------------------------------------------------------
def group_text(card: dict) -> str:
    trigger = card.get("trigger") or ""
    symptom = card.get("symptom") or ""
    if trigger in BOARD_TRIGGERS:
        t = symptom
        while True:
            if t.startswith("> "):
                t = t[2:]
            elif t.startswith("**"):
                t = t[2:]
            else:
                break
        t = t.lstrip()
        for p in ACK_PREFIXES:
            if t.startswith(p):
                t = t[len(p) :]
                break
        m = _RE_SIGNED_PREFIX.match(t)
        if m:
            t = t[m.end() :]
        return t
    if trigger in EVENTS_TRIGGERS:
        try:
            d = json.loads(symptom)
            if isinstance(d, dict):
                det = d.get("data", {}).get("detail") or d.get("detail")
                if det is not None:
                    det = str(det)
                    if trigger == "fallback_switch":
                        # 43-r2: same session + different lanes group
                        # together — normalize the lane names and drop
                        # the latency tail.
                        det = re.sub(r"(\S+) -> (\S+)", r"<lane> -> <lane>", det)
                        det = re.sub(r",\s*\d+ms", "", det)
                    return det
        except Exception:
            pass
        return symptom
    # MCP class: report_blocked / ask_outer_timeout
    idx = symptom.find("reason=")
    if idx >= 0:
        return symptom[idx + len("reason=") :]
    return symptom


# --- normalization ------------------------------------------------------
def normalize(text: str) -> str:
    t = text.lower()
    t = _RE_PATH.sub("<path>", t)
    t = _RE_HEX.sub("<hex>", t)
    t = _RE_NUM.sub("<num>", t)
    t = re.sub(r"\s+", " ", t).strip()
    return t[:80]


# --- anchor -------------------------------------------------------------
def anchor(card: dict) -> str:
    trigger = card.get("trigger") or ""
    identity = card.get("identity") or ""
    fallback = card.get("id") or "EVO-0000"

    if trigger == "fallback_switch":
        # lanes come from the RAW detail — group_text normalizes lanes for
        # grouping, but the anchor must distinguish a->b from b->c.
        raw_detail = ""
        try:
            d = json.loads(card.get("symptom") or "")
            if isinstance(d, dict):
                raw_detail = str(d.get("data", {}).get("detail") or d.get("detail") or "")
        except Exception:
            pass
        if not raw_detail:
            raw_detail = card.get("symptom") or ""
        m = _RE_FAILOVER.search(raw_detail)
        session = _events_session(identity, card)
        if session is None:
            return fallback
        if m:
            return f"{session}|{m.group(1)}->{m.group(2)}"
        return session

    body = identity
    kind = None
    for pfx in ("board:", "events:", "mcp:"):
        if body.startswith(pfx):
            kind = pfx[:-1]
            body = body[len(pfx) :]
            break
    parts = body.rsplit("#")
    if kind == "board":
        seg = parts[-3] if len(parts) >= 3 else "-"
    else:
        seg = parts[-2] if len(parts) >= 2 else "-"
    if seg in ("-", "", "-1"):
        # "-1" = harvest's no-heading sentinel; contract: 无标题则回退 EVO id
        return fallback
    return seg


def _events_session(identity: str, card: dict) -> str | None:
    # 43-r2: the JSON symptom's `session` is the AUTHORITATIVE full text
    # (it may contain `#`); the identity segment is only the fallback.
    try:
        d = json.loads(card.get("symptom") or "")
        if isinstance(d, dict):
            sess = d.get("session")
            if sess:
                return str(sess)
    except Exception:
        pass
    # 43-r3: POSITIONAL parse — the ref segment spans from after the 3rd
    # `#` up to (not including) the LAST `#`, so a ref may itself contain
    # `#` (e.g. tenant-a:local:tui#coding). rsplit()[-2] would truncate
    # at an inner `#`.
    body = identity[len("events:") :] if identity.startswith("events:") else identity
    c3 = -1
    for _ in range(3):
        c3 = body.find("#", c3 + 1)
        if c3 < 0:
            break
    last = body.rfind("#")
    if c3 >= 0 and last > c3 + 1:
        seg = body[c3 + 1 : last]
        if seg not in ("-", ""):
            return seg
    return None


# --- layer --------------------------------------------------------------
def layer(trigger: str) -> str:
    return LAYER_TABLE.get(trigger, "Observability")


# --- grouping -----------------------------------------------------------
def group(cards: list[dict]) -> list[dict]:
    """Group cards into candidates. Returns a list of dicts with key,
    trigger, layer, anchors (ordered unique), recurrence_hint, cards."""
    order: list[str] = []
    groups: dict[str, dict] = {}
    for c in cards:
        gt = group_text(c)
        key = (c.get("trigger") or "") + "|" + normalize(gt)
        if key not in groups:
            groups[key] = {
                "key": key,
                "trigger": c.get("trigger") or "",
                "layer": layer(c.get("trigger") or ""),
                "anchors": [],
                "cards": [],
            }
            order.append(key)
        g = groups[key]
        g["cards"].append(c)
        a = anchor(c)
        if a not in g["anchors"]:
            g["anchors"].append(a)
    for k in order:
        groups[k]["recurrence_hint"] = len(groups[k]["anchors"])
    return [groups[k] for k in order]


# --- FLAW record parsing (phase 3, 44a) ---------------------------------
def parse_flaw(text: str) -> dict:
    """Parse a FLAW-*.md record: frontmatter (simple key: value lines),
    sections by `## <name>` headers, and code-path anchors.

    Returns {"frontmatter": {k: v}, "sections": {name: body}, "paths": [...]}.
    Paths come from the 责任步 (responsibility) section plus the 锚点
    (anchors) section: backtick spans containing `/`, with trailing
    line-number annotations (` L123–L456`, ` L12`) stripped.
    """
    frontmatter: dict[str, str] = {}
    sections: dict[str, str] = {}
    order: list[str] = []

    lines = text.split("\n")
    i = 0
    # frontmatter between leading --- fences
    if lines and lines[0].strip() == "---":
        i = 1
        while i < len(lines) and lines[i].strip() != "---":
            line = lines[i]
            if ":" in line:
                k, _, v = line.partition(":")
                frontmatter[k.strip()] = v.strip()
            i += 1
        i += 1  # past the closing ---

    cur_name = ""
    buf: list[str] = []
    while i < len(lines):
        line = lines[i]
        if line.startswith("## "):
            if cur_name:
                sections[cur_name] = "\n".join(buf).strip()
                order.append(cur_name)
            cur_name = line[3:].strip()
            buf = []
        elif cur_name:
            buf.append(line)
        i += 1
    if cur_name:
        sections[cur_name] = "\n".join(buf).strip()
        order.append(cur_name)

    # paths: backtick spans with '/' from 责任步 + 锚点 sections
    paths: list[str] = []
    seen: set[str] = set()
    _re_span = re.compile(r"`([^`]*?/[^`]*?)`")
    _re_lineno = re.compile(r"(?:\s+L\d+(?:[–-]L?\d+)?|:\d+)$")
    for sec in ("责任步", "锚点"):
        body = sections.get(sec, "")
        for m in _re_span.finditer(body):
            p = _re_lineno.sub("", m.group(1)).strip()
            if p and p not in seen:
                seen.add(p)
                paths.append(p)

    return {"frontmatter": frontmatter, "sections": sections, "paths": paths}
