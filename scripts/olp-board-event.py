#!/usr/bin/env python3
"""Record and replay the opt-in olp-board/v1 structured board extension."""

import argparse
import bisect
from datetime import datetime
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import re
import secrets
import sys


SCHEMA = "olp-board/v1"
PREFIX = b"> OLP-EVENT "
HEX32 = re.compile(r"[0-9a-f]{32}\Z")
HEX_COMMIT = re.compile(r"(?:[0-9a-f]{40}|[0-9a-f]{64})\Z")
HEX64 = re.compile(r"[0-9a-f]{64}\Z")
STAMP = re.compile(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z\Z", re.ASCII)
BASE_FIELDS = {"schema", "id", "prev", "type", "actor", "ts", "source"}
TYPE_FIELDS = {
    "item": {"number", "title", "to", "recovery"},
    "receive": {"item"},
    "ack": {"item", "outcome", "commit", "r2", "recovery"},
    "review": {"ack", "decision"},
    "withdraw": {"item"},
    "resolve": {"review", "next"},
    "void": {"target"},
}
# The number ends at the first ". " (writers refuse numbers containing one), so
# a title may contain ". " and a rendered heading always parses back.
ITEM_LINE = re.compile(rb"### ((?:(?!\. )[^\r\n])+)\. ([^\r\n]+)\r?\n\Z")
# The v1 ACK grammar (tests/olp_contract.rs) trims leading whitespace, and the
# harvest scanner also accepts a full-width colon.
ACK_LINE = re.compile(rb"[ \t]*ACK\((done|wontdo|blocked)\)(?::|\xef\xbc\x9a)[^\r\n]*\r?\n\Z")
# Hand-written variants that lanes produce in practice: any `ACK(` (quoted,
# bulleted, emphasised or inline), an `ACK` heading, and the retired bare
# `ACK:` form. Prose that merely starts with the word ACK is not flagged.
SUSPECT_ACK = re.compile(rb"ACK\(|\A[ \t>*+-]*#{1,6}[ \t]*\**ACK\b|"
                         rb"\A[ \t>*+-]*\**ACK\**[ \t]*(?::|\xef\xbc\x9a)")
SUSPECT_OUTCOME = re.compile(rb"ACK\((done|wontdo|blocked)\)")
OPEN_FENCE = re.compile(rb"^ {0,3}(`{3,}|~{3,})([^\r\n]*)\Z")
AUTO_PREV = object()
RECORD_ERRORS = (ValueError, KeyError, TypeError, OverflowError, RecursionError)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"),
                      allow_nan=False)


def strict_loads(data):
    def pairs(values):
        result = {}
        for key, value in values:
            if key in result:
                # Escaped: a key may be a lone surrogate that no stream can encode.
                raise ValueError("Duplicate JSON key: " + json.dumps(key))
            result[key] = value
        return result

    def reject_constant(value):
        raise ValueError("Non-finite JSON number: " + value)

    try:
        return json.loads(data, object_pairs_hook=pairs, parse_constant=reject_constant)
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise ValueError("Invalid JSON: " + str(error)) from error


def uint(value, label, positive=False):
    require(type(value) is int and value >= (1 if positive else 0), "Invalid " + label)
    return value


def nonempty(value, label):
    require(isinstance(value, str) and bool(value.strip()) and
            "\n" not in value and "\r" not in value, "Invalid " + label)
    return value


def board_module():
    path = Path(__file__).with_name("olp-board-append.py")
    spec = importlib.util.spec_from_file_location("olp_board_append", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def source_ref(offset, body):
    return {"offset": offset, "length": len(body), "sha256": digest(body)}


def validate_source(source):
    require(isinstance(source, dict) and set(source) == {"offset", "length", "sha256"},
            "Source requires offset, length and sha256")
    uint(source["offset"], "source offset")
    uint(source["length"], "source length", positive=True)
    require(isinstance(source["sha256"], str) and HEX64.fullmatch(source["sha256"]),
            "Invalid source digest")


def validate_optional_source(source, label):
    if source is not None:
        try:
            validate_source(source)
        except ValueError as error:
            raise ValueError("Invalid " + label + ": " + str(error)) from error


def split_lines(data):
    """Yield (offset, line) split on LF only; a bare CR is ordinary content."""
    offset = 0
    size = len(data)
    while offset < size:
        end = data.find(b"\n", offset)
        end = size if end < 0 else end + 1
        yield offset, data[offset:end]
        offset = end


def content(line):
    return line[:-1] if line.endswith(b"\n") else line


def visible(line):
    """The line without NUL bytes: the conservative view state and recovery judge.

    It matches bash 4+ `read`, which drops NUL. bash 3.2 cuts the line at the
    NUL instead, so its legacy scanner may see only the prefix; the harvest
    merge reconciles identities by line number either way.
    """
    return line.replace(b"\0", b"")


def normative_kind(line):
    if line.startswith(PREFIX):
        return None
    # Judge the line without NUL bytes, so a NUL splitting a keyword cannot
    # hide an ACK: state blocks dispatch whatever the legacy scanner's bash
    # makes of the line (bash 3.2 may not card it before its recovery).
    line = visible(line)
    if ITEM_LINE.fullmatch(line):
        return "item"
    if ACK_LINE.fullmatch(line):
        return "ACK"
    if SUSPECT_ACK.search(content(line)):
        return "suspected ACK"
    return None


class Ranges:
    """Disjoint byte ranges kept sorted for O(log n) overlap queries."""

    def __init__(self):
        self.starts = []
        self.ends = []

    def add(self, start, length):
        index = bisect.bisect_left(self.starts, start)
        self.starts.insert(index, start)
        self.ends.insert(index, start + length)

    def overlaps(self, start, length):
        index = bisect.bisect_left(self.starts, start + length) - 1
        return index >= 0 and self.ends[index] > start


def render_item(number, title, body):
    nonempty(number, "item number")
    nonempty(title, "item title")
    require(body and body.endswith(b"\n"), "Item body must be nonempty and end with LF")
    body.decode("utf-8", errors="strict")
    return f"### {number}. {title}\n".encode("utf-8") + body


def one_line(path, label):
    body = Path(path).resolve(strict=True).read_bytes()
    text = body.decode("utf-8", errors="strict").strip()
    require(text and "\n" not in text and "\r" not in text, label + " must be one nonempty line")
    return text


def render_receive(actor, item):
    return f"RECEIVE(item={item}, actor={actor})\n".encode("utf-8")


def render_ack(event, explanation):
    commit = event["commit"] if event["commit"] is not None else "none"
    return (f"ACK({event['outcome']}): {explanation}; item {event['item']}; "
            f"commit {commit}; R2 {event['r2']}\n").encode("utf-8")


def render_review(event, explanation):
    return (f"REVIEW({event['decision']}): {explanation}; ack {event['ack']}\n").encode("utf-8")


def render_withdraw(event, explanation):
    return f"WITHDRAW(item={event['item']}): {explanation}\n".encode("utf-8")


def render_resolve(event, explanation):
    following = event["next"] if event["next"] is not None else "none"
    return (f"RESOLVE(review={event['review']}, next={following}): "
            f"{explanation}\n").encode("utf-8")


def render_void(event, explanation, terminate_partial_line):
    target = event["target"]
    prose = f"VOID(offset={target['offset']}, length={target['length']}): {explanation}\n"
    return (b"\n" if terminate_partial_line else b"") + prose.encode("utf-8")


def source_matches(event, body, data):
    text = body.decode("utf-8", errors="strict")
    kind = event["type"]
    explanation = r"([^\r\n]+)"
    if kind == "item":
        prefix = f"### {event['number']}. {event['title']}\n"
        require(text.startswith(prefix) and len(text) > len(prefix) and text.endswith("\n"),
                "Item source does not match its number and title")
        return
    if kind == "receive":
        require(body == render_receive(event["actor"], event["item"]),
                "Receive source does not match the event")
        return
    if kind == "ack":
        commit = event["commit"] if event["commit"] is not None else "none"
        pattern = (r"ACK\(" + re.escape(event["outcome"]) + r"\): " + explanation +
                   r"; item " + re.escape(event["item"]) + r"; commit " + re.escape(commit) +
                   r"; R2 " + re.escape(event["r2"]) + r"\n\Z")
    elif kind == "review":
        pattern = (r"REVIEW\(" + re.escape(event["decision"]) + r"\): " + explanation +
                   r"; ack " + re.escape(event["ack"]) + r"\n\Z")
    elif kind == "withdraw":
        pattern = r"WITHDRAW\(item=" + re.escape(event["item"]) + r"\): " + explanation + r"\n\Z"
    elif kind == "resolve":
        following = event["next"] if event["next"] is not None else "none"
        pattern = (r"RESOLVE\(review=" + re.escape(event["review"]) + r", next=" +
                   re.escape(following) + r"\): " + explanation + r"\n\Z")
    else:
        offset = event["source"]["offset"]
        partial = offset > 0 and data[offset - 1:offset] != b"\n"
        target = event["target"]
        pattern = ((r"\n" if partial else "") + r"VOID\(offset=" + str(target["offset"]) +
                   r", length=" + str(target["length"]) + r"\): " + explanation + r"\n\Z")
    require(re.fullmatch(pattern, text) is not None,
            kind.capitalize() + " source does not match the event")


def fence_line(line):
    value = content(line)
    return value[:-1] if value.endswith(b"\r") else value


def opening_fence(line):
    match = OPEN_FENCE.fullmatch(fence_line(line))
    if match is None:
        return None
    marker, info = match.groups()
    if marker.startswith(b"`") and b"`" in info:
        return None
    return marker[:1], len(marker)


def closes_fence(line, marker, minimum_length):
    match = re.fullmatch(rb" {0,3}([`~]+)[ \t]*", fence_line(line))
    if match is None:
        return False
    candidate = match.group(1)
    return candidate[:1] == marker and len(candidate) >= minimum_length and len(set(candidate)) == 1


def scan_lines(data):
    records = []
    ranges = []
    active = None
    for offset, line in split_lines(data):
        if active is None:
            opened = opening_fence(line)
            if opened is None:
                records.append((offset, line, False))
            else:
                marker, minimum_length = opened
                active = {
                    "offset": offset,
                    "marker": marker,
                    "minimum_length": minimum_length,
                    "opener": source_ref(offset, line),
                }
                records.append((offset, line, True))
        else:
            records.append((offset, line, True))
            if closes_fence(line, active["marker"], active["minimum_length"]):
                end = offset + len(line)
                raw = data[active["offset"]:end]
                ranges.append({"offset": active["offset"], "length": len(raw),
                               "sha256": digest(raw), "closed": True,
                               "opener": active["opener"]})
                active = None
    if active is not None:
        raw = data[active["offset"]:]
        ranges.append({"offset": active["offset"], "length": len(raw), "sha256": digest(raw),
                       "closed": False, "opener": active["opener"]})
    return records, ranges


def line_records(data):
    records, unused_ranges = scan_lines(data)
    return records


class State:
    def __init__(self, data=b""):
        self.data = data
        self.records, self.fences = scan_lines(data)
        self.line_index = {offset: index for index, (offset, _, _) in enumerate(self.records)}
        self.head = None
        self.events = {}
        self.order = {}
        self.items = {}
        self.receives = {}
        self.acks = {}
        self.reviews = {}
        self.first_event_offset = None
        self.drift = []
        self.malformed = {}
        self.paired = Ranges()
        self.claimed = Ranges()
        self.recoveries = {}
        self.voids = {}
        self.fenced_ranges = [{key: value for key, value in fence.items() if key != "opener"}
                              for fence in self.fences]

    def record_at(self, offset):
        index = self.line_index.get(offset)
        return None if index is None else self.records[index]

    def unterminated_tail(self, before):
        """Return the partial line that ends `data[:before]`, if any."""
        if before == 0 or self.data[before - 1:before] == b"\n":
            return None
        start = self.data.rfind(b"\n", 0, before) + 1
        return start, self.data[start:before]

    def claim_target(self, target, before, label):
        """Validate an exact drift reference used by recovery or void."""
        validate_source(target)
        offset, length = target["offset"], target["length"]
        require(offset + length <= before, label + " must reference text before the " +
                ("recovering" if label == "Recovery" else "voiding") + " event")
        raw = self.data[offset:offset + length]
        require(len(raw) == length and digest(raw) == target["sha256"],
                label + " source bytes changed")
        require(not self.paired.overlaps(offset, length),
                label + " source is already paired with an event")
        require(not self.claimed.overlaps(offset, length), label + " source was already consumed")
        return offset, length, raw

    def normative_target(self, target, before, label):
        offset, length, raw = self.claim_target(target, before, label)
        require(self.first_event_offset is not None and offset >= self.first_event_offset,
                label + " must reference normative text after structured opt-in")
        record = self.record_at(offset)
        require(record is not None and len(record[1]) == length,
                label + " must cover exactly one whole line")
        kind = normative_kind(raw)
        require(not record[2] and kind is not None,
                label + " source is not an unresolved normative line")
        return raw, kind

    def validate_recovery(self, event):
        recovery = event.get("recovery")
        if recovery is None:
            return
        raw, kind = self.normative_target(recovery, event["source"]["offset"], "Recovery")
        raw = visible(raw)
        if event["type"] == "item":
            match = ITEM_LINE.fullmatch(raw)
            require(kind == "item" and match is not None,
                    "Item recovery is not a normative item heading")
            number, title = (part.decode("utf-8", errors="strict") for part in match.groups())
            require(number == event["number"] and title == event["title"],
                    "Item recovery number or title does not match")
            return
        require(kind in ("ACK", "suspected ACK"), "ACK recovery is not a normative ACK line")
        match = (ACK_LINE.fullmatch(raw) if kind == "ACK" else SUSPECT_OUTCOME.search(raw))
        require(match is not None,
                "Suspected ACK has no recoverable outcome; void it and record a new ACK")
        require(match.group(1).decode("ascii") == event["outcome"],
                "ACK recovery outcome does not match")

    def validate_void(self, event):
        target = event["target"]
        before = event["source"]["offset"]
        offset, length, raw = self.claim_target(target, before, "Void")
        tail = self.unterminated_tail(before)
        if tail is not None:
            require(tail == (offset, raw),
                    "The board ends with a partial line; void that line first")
            return
        if offset in self.malformed:
            entry = self.malformed[offset]
            require(entry["length"] == length, "Void target is not an unresolved drift record")
            return
        self.normative_target(target, before, "Void")

    def check(self, event):
        """Validate one event against the ledger without mutating it."""
        require(isinstance(event, dict), "Expected an event object")
        kind = event.get("type")
        require(kind in TYPE_FIELDS, "Unknown event type")
        require(set(event) == BASE_FIELDS | TYPE_FIELDS[kind], "Missing or unexpected event fields")
        require(event["schema"] == SCHEMA, "Unsupported board schema")
        require(isinstance(event["id"], str) and HEX32.fullmatch(event["id"]), "Invalid event ID")
        require(event["id"] not in self.events, "Duplicate event ID")
        require(event["prev"] == self.head, "Broken prev chain")
        nonempty(event["actor"], "actor")
        require(isinstance(event["ts"], str) and STAMP.fullmatch(event["ts"]),
                "Invalid event timestamp")
        datetime.strptime(event["ts"], "%Y-%m-%dT%H:%M:%SZ")
        validate_source(event["source"])
        actor = event["actor"]
        if kind == "item":
            nonempty(event["number"], "item number")
            nonempty(event["title"], "item title")
            nonempty(event["to"], "item recipient")
            validate_optional_source(event["recovery"], "item recovery")
            self.validate_recovery(event)
        elif kind == "receive":
            item = self.items.get(event["item"])
            require(item is not None, "Receive references an unknown item")
            require(item["withdraw"] is None, "Item was withdrawn")
            require(actor == item["event"]["to"], "Only the item recipient may receive")
            require(item["receive"] is None, "Item was already received")
        elif kind == "ack":
            item = self.items.get(event["item"])
            require(item is not None and item["receive"] is not None, "ACK requires a received item")
            require(actor == item["event"]["to"], "Only the item recipient may ACK")
            require(item["ack"] is None, "Item already has a terminal ACK")
            require(event["outcome"] in ("done", "wontdo", "blocked"), "Invalid ACK outcome")
            require(event["r2"] in ("verified", "partially-verified", "unverified"), "Invalid R2")
            require(event["commit"] is None or
                    (isinstance(event["commit"], str) and HEX_COMMIT.fullmatch(event["commit"])),
                    "Invalid commit")
            validate_optional_source(event["recovery"], "ACK recovery")
            self.validate_recovery(event)
        elif kind == "review":
            ack = self.acks.get(event["ack"])
            require(ack is not None, "Review references an unknown ACK")
            item = self.items[ack["item"]]
            require(actor == item["event"]["actor"], "Only the item author may review")
            require(item["review"] is None, "ACK already has a review")
            require(event["decision"] in ("accept", "return", "escalate"), "Invalid review decision")
            require(not (ack["outcome"] == "wontdo" and event["decision"] == "return"),
                    "wontdo cannot be returned")
        elif kind == "withdraw":
            item = self.items.get(event["item"])
            require(item is not None, "Withdraw references an unknown item")
            require(actor == item["event"]["actor"], "Only the item author may withdraw")
            require(item["withdraw"] is None, "Item was already withdrawn")
            require(item["receive"] is None,
                    "A received item cannot be withdrawn; the recipient must ACK it")
        elif kind == "resolve":
            review = self.reviews.get(event["review"])
            require(review is not None, "Resolve references an unknown review")
            item = self.items[self.acks[review["ack"]]["item"]]
            require(review["decision"] == "escalate", "Only an escalated review can be resolved")
            require(actor == item["event"]["actor"], "Only the item author may resolve")
            require(item["resolve"] is None, "Escalation was already resolved")
            following = event["next"]
            require(following is None or (isinstance(following, str) and following in self.items
                                           and following != item["event"]["id"]),
                    "Resolve must point to a different existing item or none")
        else:
            self.validate_void(event)

    def commit(self, event, event_offset):
        kind = event["type"]
        if kind == "item":
            self.items[event["id"]] = {"event": event, "receive": None, "ack": None,
                                       "review": None, "withdraw": None, "resolve": None}
        elif kind == "receive":
            self.items[event["item"]]["receive"] = event
            self.receives[event["id"]] = event
        elif kind == "ack":
            self.items[event["item"]]["ack"] = event
            self.acks[event["id"]] = event
        elif kind == "review":
            self.items[self.acks[event["ack"]]["item"]]["review"] = event
            self.reviews[event["id"]] = event
        elif kind == "withdraw":
            self.items[event["item"]]["withdraw"] = event
        elif kind == "resolve":
            review = self.reviews[event["review"]]
            self.items[self.acks[review["ack"]]["item"]]["resolve"] = event
        self.order[event["id"]] = len(self.events)
        self.events[event["id"]] = event
        self.head = event["id"]
        source = event["source"]
        self.paired.add(source["offset"], source["length"])
        for field, ledger in (("recovery", self.recoveries), ("target", self.voids)):
            evidence = event.get(field)
            if evidence is not None:
                self.claimed.add(evidence["offset"], evidence["length"])
                ledger[(evidence["offset"], evidence["length"])] = {
                    "source": evidence, "event": event["id"], "type": kind}
        if self.first_event_offset is None:
            self.first_event_offset = event_offset

    def apply(self, event, event_offset):
        self.check(event)
        self.commit(event, event_offset)

    def add_malformed(self, offset, line, error):
        value = content(line)
        # Every consumer prints the reason as UTF-8 JSON, so it must stay
        # encodable whatever the damaged line held.
        reason = str(error).encode("utf-8", "backslashreplace").decode("utf-8")
        self.malformed[offset] = {"offset": offset, "length": len(value),
                                  "sha256": digest(value),
                                  "reason": "malformed event: " + reason}

    def compute_drift(self):
        drift = []
        for entry in self.malformed.values():
            if not self.claimed.overlaps(entry["offset"], entry["length"]):
                drift.append({**entry, "kind": "malformed_event"})
        if self.first_event_offset is not None:
            for offset, line, fenced in self.records:
                if offset < self.first_event_offset or fenced:
                    continue
                kind = normative_kind(line)
                if kind is None or not line.endswith(b"\n"):
                    continue
                if self.paired.overlaps(offset, len(line)) or self.claimed.overlaps(offset, len(line)):
                    continue
                if kind == "suspected ACK":
                    drift.append({**source_ref(offset, line), "kind": "suspected_ack",
                                  "reason": "suspected hand-written ACK requires recovery or void"})
                else:
                    drift.append({**source_ref(offset, line), "kind": "unpaired_" + kind.lower(),
                                  "reason": "unpaired normative " + kind +
                                            " requires manual recovery"})
            for fence in self.fences:
                if not fence["closed"] and fence["offset"] >= self.first_event_offset:
                    drift.append({**fence["opener"], "kind": "unclosed_fence",
                                  "reason": "unclosed fence after structured opt-in"})
        drift.sort(key=lambda entry: entry["offset"])
        return drift

    def partial_tail(self):
        """Evidence for the trailing line without LF, whatever it contains.

        A non-event tail may be a writer that ignores the lock and is still
        appending, so it is not DRIFT on its own; writers refuse to append until
        the tail is voided (or terminated by a fence-closing plain append).
        """
        tail = self.unterminated_tail(len(self.data))
        if tail is None or self.claimed.overlaps(tail[0], len(tail[1])):
            return None
        return source_ref(tail[0], tail[1])

    def trigger(self, entry):
        """Return the event whose arrival made a projected entry actionable."""
        if entry["type"] == "item":
            item = self.items[entry["id"]]
            return item["receive"]["id"] if item["receive"] is not None else entry["id"]
        return entry["id"]

    def projection(self):
        unreceived = []
        received_pending = []
        unreviewed_ack = []
        escalated = []
        for item in self.items.values():
            if item["withdraw"] is not None:
                continue
            if item["receive"] is None:
                unreceived.append(item["event"])
            elif item["ack"] is None:
                received_pending.append(item["event"])
            elif item["review"] is None:
                unreviewed_ack.append(item["ack"])
            elif item["review"]["decision"] == "escalate" and item["resolve"] is None:
                escalated.append(item["review"])
        # Items arrive before their ACKs/reviews, which can arrive in any order
        # when recipients work concurrently. Each queue follows its own events.
        unreviewed_ack.sort(key=lambda event: self.order[event["id"]])
        escalated.sort(key=lambda event: self.order[event["id"]])
        mode = "mixed" if self.drift else ("structured" if self.events else "legacy")
        return {
            "schema": SCHEMA,
            "mode": mode,
            "head": self.head,
            "unreceived": unreceived,
            "received_pending": received_pending,
            "unreviewed_ack": unreviewed_ack,
            "escalated": escalated,
            "drift": self.drift,
            "fenced_ranges": self.fenced_ranges,
            "recovery_evidence": list(self.recoveries.values()),
            "quarantine_evidence": list(self.voids.values()),
            "dispatch_blocked": bool(self.drift),
            "partial_tail": self.partial_tail(),
            "events": list(self.events.values()),
        }


def parse_event_record(state, index, offset, line):
    """Parse and structurally verify the event line at records[index]."""
    require(line.endswith(b"\n"), "Structured event line must end with LF")
    event = strict_loads(line[len(PREFIX):-1])
    require(isinstance(event, dict), "Structured event must be a JSON object")
    validate_source(event.get("source"))
    source = event["source"]
    require(source["offset"] + source["length"] == offset,
            "Event source must be immediately adjacent")
    start = source["offset"]
    # A source is whole lines of fresh text. Only a void that closes a partial
    # line starts with the LF it supplies to that line.
    require(start == 0 or state.data[start - 1:start] == b"\n" or
            (event.get("type") == "void" and state.data[start:start + 1] == b"\n"),
            "Event source must start at a line boundary")
    # Paired and consumed ranges stay disjoint, so no byte is paired twice or
    # both paired and voided, and the sorted-range queries stay exact.
    require(not state.paired.overlaps(start, source["length"]) and
            not state.claimed.overlaps(start, source["length"]),
            "Event source overlaps text already paired or consumed")
    body = state.data[source["offset"]:offset]
    require(len(body) == source["length"] and digest(body) == source["sha256"],
            "Event source bytes changed")
    require(isinstance(event.get("type"), str) and event["type"] in TYPE_FIELDS,
            "Unknown event type")
    require(set(event) == BASE_FIELDS | TYPE_FIELDS[event["type"]],
            "Missing or unexpected event fields")
    if event["type"] == "void":
        validate_source(event["target"])
    source_matches(event, body, state.data)
    require(line == PREFIX + canonical(event).encode("utf-8") + b"\n",
            "Event JSON is not canonical")
    records = state.records
    require(index + 1 < len(records) and records[index + 1][1] ==
            ("ts=" + str(event["ts"]) + "\n").encode("utf-8"), "Event timestamp boundary mismatch")
    return event


def replay(data):
    """Project the ledger; malformed event records become DRIFT, never fatal."""
    state = State(data)
    for index, (offset, line, fenced) in enumerate(state.records):
        if fenced or not line.startswith(PREFIX):
            continue
        try:
            event = parse_event_record(state, index, offset, line)
            state.check(event)
        except RECORD_ERRORS as error:
            state.add_malformed(offset, line, error)
            continue
        state.commit(event, offset)
    state.drift = state.compute_drift()
    return state


def read_state(board, lock_timeout=10.0):
    path, data = board_module().read_snapshot(board, lock_timeout)
    return path, replay(data)


def build_event(state, kind, actor, fields, source, offset, ts, event_id=None):
    require(kind in TYPE_FIELDS, "Unknown event type")
    require(set(fields) == TYPE_FIELDS[kind], "Missing or unexpected event fields")
    require(not (kind == "item" and isinstance(fields["number"], str) and ". " in fields["number"]),
            "Item number must not contain '. ': the heading '### <number>. <title>' splits "
            "at the first '. '")
    event = {"schema": SCHEMA, "id": event_id or secrets.token_hex(16),
             "prev": state.head, "type": kind,
             "actor": actor, "ts": ts, "source": source_ref(offset, source), **fields}
    source_matches(event, source, state.data)
    state.check(event)
    return event


def write_event(args, kind, actor, fields, prose_builder, explicit_prev=AUTO_PREV,
                explicit_id=None, progress=None):
    module = board_module()
    if progress is None:
        progress = {"may_have_appended": False}
    result_event = {}

    def builder(data, offset, ts):
        state = replay(data)
        if explicit_prev is not AUTO_PREV:
            require(explicit_prev == state.head, "Stale ledger head; reload before recording")
        provisional = {"type": kind, "actor": actor, **fields}
        prose = prose_builder(provisional, data)
        event = build_event(state, kind, actor, fields, prose, offset, ts, event_id=explicit_id)
        result_event.update(event)
        payload = prose + PREFIX + canonical(event).encode("utf-8") + b"\n"
        candidate = replay(data + payload + ("ts=" + ts + "\n").encode("ascii"))
        if candidate.head != event["id"]:
            unclosed = any(not fence["closed"] for fence in candidate.fences)
            raise ValueError("Rendered event was hidden by prose structure" +
                             ("; close the open code fence first (olp-board-append.py "
                              "append, with --terminate-partial-line if the board ends "
                              "with a partial line)" if unclosed else ""))
        require(not [entry for entry in candidate.drift if entry["offset"] >= offset],
                "Rendered payload would create new drift")
        return payload

    receipt = module.append_generated(args.board, args.lock_timeout, builder, progress,
                                      may_terminate_partial_line=(kind == "void"))
    return {**receipt, "event": result_event["id"], "source": result_event["source"]}


def strict_file(path):
    return strict_loads(Path(path).resolve(strict=True).read_bytes())


def evidence_file(path):
    if path is None:
        return None
    evidence = strict_file(path)
    validate_source(evidence)
    return evidence


def record_command(args, progress):
    spec = strict_file(args.event_file)
    require(isinstance(spec, dict), "Event file must contain an object")
    kind = spec.get("type")
    require(kind in TYPE_FIELDS, "Unknown event type")
    expected = {"prev", "type", "actor"} | TYPE_FIELDS[kind]
    require(set(spec) == expected, "Record spec has missing or unexpected fields")
    body = Path(args.body_file).resolve(strict=True).read_bytes()
    fields = {key: spec[key] for key in TYPE_FIELDS[kind]}
    return write_event(args, kind, spec["actor"], fields, lambda unused, data: body,
                       explicit_prev=spec["prev"], progress=progress)


def verify_receipt(args):
    receipt = strict_file(args.receipt_file)
    required = {"board", "offset", "length", "body_sha256", "payload_sha256", "ts", "verified"}
    require(isinstance(receipt, dict) and required <= set(receipt), "Invalid receipt")
    require(receipt["verified"] is True, "Receipt is not verified")
    uint(receipt["offset"], "receipt offset")
    uint(receipt["length"], "receipt length", positive=True)
    board, data = board_module().read_snapshot(receipt["board"], args.lock_timeout)
    payload = data[receipt["offset"]:receipt["offset"] + receipt["length"]]
    require(len(payload) == receipt["length"] and digest(payload) == receipt["payload_sha256"],
            "Receipt payload bytes changed")
    suffix = ("ts=" + receipt["ts"] + "\n").encode("ascii")
    require(payload.endswith(suffix) and digest(payload[:-len(suffix)]) == receipt["body_sha256"],
            "Receipt body or timestamp changed")
    if "event" in receipt:
        require(receipt["event"] in replay(data).events,
                "Receipt event is not a valid ledger event")
    return {"verified": True, "board": str(board), "offset": receipt["offset"],
            "length": receipt["length"]}


def finite_nonnegative(value):
    number = float(value)
    if not math.isfinite(number) or number < 0:
        raise argparse.ArgumentTypeError("Expected a finite nonnegative number")
    return number


class MachineArgumentParser(argparse.ArgumentParser):
    """Report an argument error like any other write-entry failure: one machine JSON."""

    def error(self, message):
        print(json.dumps({"verified": False, "error": self.prog + ": " + message,
                          "may_have_appended": False, "execution_authorized": False},
                         ensure_ascii=False), file=sys.stderr, flush=True)
        raise SystemExit(2)


def add_writer_options(parser):
    parser.add_argument("--board", required=True)
    parser.add_argument("--lock-timeout", type=finite_nonnegative, default=10.0)


def main(argv=None):
    # Subcommand parsers inherit the parser class, so they report the same way.
    parser = MachineArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    item = sub.add_parser("item")
    add_writer_options(item)
    item.add_argument("--actor", required=True)
    item.add_argument("--number", required=True)
    item.add_argument("--title", required=True)
    item.add_argument("--to", default="runtime")
    item.add_argument("--body-file", required=True)
    item.add_argument("--recovery-file")

    receive = sub.add_parser("receive")
    add_writer_options(receive)
    receive.add_argument("--actor", required=True)
    receive.add_argument("--item", required=True)

    ack = sub.add_parser("ack")
    add_writer_options(ack)
    ack.add_argument("--actor", required=True)
    ack.add_argument("--item", required=True)
    ack.add_argument("--outcome", choices=("done", "wontdo", "blocked"), required=True)
    ack.add_argument("--r2", choices=("verified", "partially-verified", "unverified"), required=True)
    ack.add_argument("--commit")
    ack.add_argument("--body-file", required=True)
    ack.add_argument("--recovery-file")

    review = sub.add_parser("review")
    add_writer_options(review)
    review.add_argument("--actor", required=True)
    review.add_argument("--ack", required=True)
    review.add_argument("--decision", choices=("accept", "return", "escalate"), required=True)
    review.add_argument("--body-file", required=True)

    withdraw = sub.add_parser("withdraw")
    add_writer_options(withdraw)
    withdraw.add_argument("--actor", required=True)
    withdraw.add_argument("--item", required=True)
    withdraw.add_argument("--body-file", required=True)

    resolve = sub.add_parser("resolve")
    add_writer_options(resolve)
    resolve.add_argument("--actor", required=True)
    resolve.add_argument("--review", required=True)
    resolve.add_argument("--next")
    resolve.add_argument("--body-file", required=True)

    void = sub.add_parser("void")
    add_writer_options(void)
    void.add_argument("--actor", required=True)
    void.add_argument("--target-file", required=True)
    void.add_argument("--body-file", required=True)

    record = sub.add_parser("record")
    add_writer_options(record)
    record.add_argument("--event-file", required=True)
    record.add_argument("--body-file", required=True)

    state = sub.add_parser("state")
    state.add_argument("--board", required=True)
    state.add_argument("--lock-timeout", type=finite_nonnegative, default=10.0)

    snapshot = sub.add_parser(
        "snapshot", help="copy the board once under its shared lock and project that copy")
    snapshot.add_argument("--board", required=True)
    snapshot.add_argument("--out", required=True, help="new file that receives the board bytes")
    snapshot.add_argument("--lock-timeout", type=finite_nonnegative, default=10.0)

    verify = sub.add_parser("verify")
    verify.add_argument("--receipt-file", required=True)
    verify.add_argument("--lock-timeout", type=finite_nonnegative, default=10.0)

    args = parser.parse_args(argv)
    progress = {"may_have_appended": False}
    try:
        if args.command == "item":
            raw = Path(args.body_file).resolve(strict=True).read_bytes()
            result = write_event(args, "item", args.actor,
                                 {"number": args.number, "title": args.title, "to": args.to,
                                  "recovery": evidence_file(args.recovery_file)},
                                 lambda unused, data: render_item(args.number, args.title, raw),
                                 progress=progress)
        elif args.command == "receive":
            result = write_event(args, "receive", args.actor, {"item": args.item},
                                 lambda unused, data: render_receive(args.actor, args.item),
                                 progress=progress)
        elif args.command == "ack":
            explanation = one_line(args.body_file, "ACK explanation")
            fields = {"item": args.item, "outcome": args.outcome,
                      "commit": args.commit, "r2": args.r2,
                      "recovery": evidence_file(args.recovery_file)}
            result = write_event(args, "ack", args.actor, fields,
                                 lambda event, data: render_ack(event, explanation),
                                 progress=progress)
        elif args.command == "review":
            explanation = one_line(args.body_file, "Review explanation")
            fields = {"ack": args.ack, "decision": args.decision}
            result = write_event(args, "review", args.actor, fields,
                                 lambda event, data: render_review(event, explanation),
                                 progress=progress)
        elif args.command == "withdraw":
            explanation = one_line(args.body_file, "Withdraw explanation")
            result = write_event(args, "withdraw", args.actor, {"item": args.item},
                                 lambda event, data: render_withdraw(event, explanation),
                                 progress=progress)
        elif args.command == "resolve":
            explanation = one_line(args.body_file, "Resolve explanation")
            fields = {"review": args.review, "next": args.next}
            result = write_event(args, "resolve", args.actor, fields,
                                 lambda event, data: render_resolve(event, explanation),
                                 progress=progress)
        elif args.command == "void":
            explanation = one_line(args.body_file, "Void explanation")
            fields = {"target": evidence_file(args.target_file)}
            result = write_event(
                args, "void", args.actor, fields,
                lambda event, data: render_void(event, explanation,
                                                bool(data) and not data.endswith(b"\n")),
                progress=progress)
        elif args.command == "record":
            result = record_command(args, progress)
        elif args.command == "verify":
            result = verify_receipt(args)
        elif args.command == "snapshot":
            # One read serves every consumer of this state (the harvest reads the
            # copy, not the live board), so text appended meanwhile waits whole
            # for the next reader.
            _, data = board_module().read_snapshot(args.board, args.lock_timeout)
            with open(args.out, "xb") as stream:
                stream.write(data)
            result = {**replay(data).projection(),
                      "snapshot": {"path": str(Path(args.out).resolve()), "length": len(data),
                                   "sha256": digest(data)}}
        else:
            _, ledger = read_state(args.board, args.lock_timeout)
            result = ledger.projection()
        print(json.dumps(result, ensure_ascii=False, allow_nan=False), flush=True)
        return 0
    # RecursionError: deeply nested JSON input is an input error like any other.
    except (OSError, ValueError, KeyError, TypeError, OverflowError, RecursionError,
            KeyboardInterrupt) as error:
        print(json.dumps({"verified": False, "error": str(error),
                          "may_have_appended": progress["may_have_appended"],
                          "execution_authorized": False}, ensure_ascii=False),
              file=sys.stderr, flush=True)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
