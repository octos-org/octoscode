"""Shared evolution-loop library (#43c-1, SDD contract v2
specs/task-req-olp-evo-p2.spec.md).

No CLI. Imported by scripts/olp-evo-retro.sh (and, from 43c-2 on, the
metrics script) so parsing / grouping / normalization / anchoring / layer
hints live in exactly one place.
"""

from __future__ import annotations

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
                    return str(det)
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
        detail = group_text(card)
        m = _RE_FAILOVER.search(detail)
        session = _events_session(identity, card)
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
    if seg in ("-", ""):
        return fallback
    return seg


def _events_session(identity: str, card: dict) -> str:
    # session is the events anchor segment (倒数第 2); fall back to the
    # JSON symptom's session field, else "-".
    body = identity[len("events:") :] if identity.startswith("events:") else identity
    parts = body.rsplit("#")
    seg = parts[-2] if len(parts) >= 2 else "-"
    if seg not in ("-", ""):
        return seg
    try:
        d = json.loads(card.get("symptom") or "")
        if isinstance(d, dict):
            s = d.get("session")
            if s:
                return str(s)
    except Exception:
        pass
    return "-"


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
