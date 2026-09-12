"""Model gate unit fixtures. These are synthetic ledgers, not review receipts."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import subprocess
import sys
import hashlib

SPEC = importlib.util.spec_from_file_location(
    "review", Path(__file__).resolve().parents[1] / "scripts/olp-review-evidence.py"
)
review = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(review)
SESSION = "fixture:local:tui#master"


def ledger(root, slug, models=("glm-5.3", "glm-5.3")):
    session = f"fixture:local:tui#peer-{slug}"
    wire = session + "\0~cwd-123abc"
    path = root / "ui-protocol" / wire.encode().hex() / "ledger-0001.log"
    path.parent.mkdir(parents=True, exist_ok=True)
    rows = []
    for n, model in enumerate(models, 1):
        for kind in ("turn_started", "model", "turn_completed"):
            event = {"record_kind": "notification", "kind": kind,
                     "session_id": session, "turn_id": f"{slug}-{n}"}
            if kind == "model":
                event = {**event, "record_kind": "progress",
                         "metadata": {"kind": "token_cost_update",
                                      "token_cost": {"model": model}}}
            rows.append({"seq": len(rows) + 1, "event": event})
    write_rows(path, rows)
    return path


def native_report(root, slug, number, tid=None, outcome="completed"):
    path = root / "native" / slug / f"result-{number}.md"
    path.parent.mkdir(parents=True, exist_ok=True)
    tid = tid if tid is not None else f"{slug}-{number}"
    path.write_text(f"---\nslug: {slug}\noutcome: {outcome}\nturn: {number}\nturn_id: {tid}\n---\nreport\n")
    return {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def write_rows(path, rows):
    path.write_text("".join(json.dumps(r) + "\n" for r in rows))


class ModelGate(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="olp-model-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.paths = {"glm": ledger(self.root, "glm"),
                      "k3": ledger(self.root, "k3", ("k3-256k", "k3-256k"))}
        self.native = {lane: {n: native_report(self.root, lane, n) for n in (1, 2)}
                       for lane in ("glm", "k3")}
        # Deliberately isolate model acceptance from the behavior gate here.
        # The Rust live-Cargo test covers the composed CLI acceptance path.
        self.state = {
            "runtime": str(self.root), "session": SESSION, "frozen": True,
            "reviews": {lane: {"peer": lane, "turn": "1", "native_report": self.native[lane][1]}
                        for lane in ("glm", "k3")},
            "challenges": {"X": {"latest": {"accepted": True}}},
            "verdicts": {"X": {"state": "approve", "reason": "executed-probe-passed"}},
            "cross": [],
        }
        for lane in ("glm", "k3"):
            self.state["cross"].append({"slug": lane, "turn": "2",
                "native_report": self.native[lane][2],
                "model_evidence": self.pair(lane)})

    def pair(self, lane, turn="2"):
        return review._model_pair(self.state, lane, turn, self.native[lane][int(turn)])

    def assert_blocked(self):
        status = review.build_status(self.state)
        self.assertTrue(status["behavior_accepted"])
        self.assertFalse(status["model_verified"])
        self.assertFalse(status["review_accepted"])

    def test_completed_write_loss_cannot_borrow_an_earlier_model(self):
        # A complete GLM turn lost its native result/index writes. The
        # current cross is result-2, but belongs to ledger turn3 using K3.
        ledger(self.root, "glm", ("glm-5.3", "glm-5.3", "k3-256k"))
        native = self.root / "native" / "glm"
        native.mkdir(parents=True, exist_ok=True)
        for number, tid in ((1, "glm-1"), (2, "glm-3")):
            path = native / f"result-{number}.md"
            path.write_text(f"---\nslug: glm\noutcome: completed\nturn: {number}\nturn_id: {tid}\n---\nreport\n")
            binding = {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
            if number == 1:
                self.state["reviews"]["glm"]["native_report"] = binding
            else:
                self.state["cross"][0]["native_report"] = binding
        self.assertFalse(review.models_verified(self.state),
                         "result-2 must not borrow the model of ledger turn2")

    def test_malformed_cross_entries_fail_closed(self):
        for entry in (None, "bad", [], ["not", "a", "dict"], {"slug": []}, {"slug": {}}):
            with self.subTest(entry=entry):
                state = copy.deepcopy(self.state)
                state["cross"] = [entry]
                self.assertFalse(review.models_verified(state))
                self.assertFalse(review.build_status(state)["review_accepted"])

    def test_initial_state_turn_cannot_redirect_frozen_native_identity(self):
        # PR637 review: keep frozen report bytes/hashes unchanged, but edit
        # the copied initial ordinal to skip an initial foreign-model turn.
        ledger(self.root, "glm", ("some-foreign-model", "glm-5.3", "glm-5.3"))
        self.native["glm"][3] = native_report(self.root, "glm", 3)
        self.state["reviews"]["glm"]["turn"] = "2"
        self.state["cross"][0].update(turn="3", native_report=self.native["glm"][3])
        with self.assertRaises(review.ReviewError) as caught:
            self.pair("glm", "3")
        self.assertIn("编号不匹配", str(caught.exception))
        self.assert_blocked()

    def test_shifted_correct_model_uses_actual_turn_identity(self):
        ledger(self.root, "glm", ("glm-5.3", "glm-5.3", "glm-5.3"))
        binding = native_report(self.root, "glm", 2, "glm-3")
        cross = self.state["cross"][0]
        cross["native_report"] = binding
        cross["model_evidence"] = review._model_pair(self.state, "glm", "2", binding)
        self.assertEqual(cross["model_evidence"]["cross"]["turn_id"], "glm-3")
        self.assertEqual(cross["model_evidence"]["cross"]["turn_no"], 2)
        self.assertEqual(cross["model_evidence"]["cross"]["ledger_turn_no"], 3)
        self.assertTrue(review.models_verified(self.state))
        # Matching file/terminal counts would still be the wrong join if
        # an earlier turn never wrote a terminal. Exact IDs remain sound.
        rows = [json.loads(line) for line in self.paths["glm"].read_text().splitlines()]
        rows[1]["event"] = {"record_kind": "notification", "kind": "envelope"}
        rows[2]["event"] = {"record_kind": "notification", "kind": "envelope"}
        write_rows(self.paths["glm"], rows)
        self.state["reviews"]["glm"]["native_report"] = native_report(self.root, "glm", 1, "glm-2")
        cross["model_evidence"] = review._model_pair(self.state, "glm", "2", binding)
        self.assertTrue(review.models_verified(self.state))

    def test_native_report_identity_rejects_missing_unknown_reused_and_reversed_ids(self):
        for initial, later in (("", "glm-2"), ("glm-1", ""), ("glm-1", "unknown"),
                               ("glm-1", "k3-2"), ("glm-1", "glm-1"), ("glm-2", "glm-1")):
            with self.subTest(initial=initial, cross=later):
                self.state["reviews"]["glm"]["native_report"] = native_report(self.root, "glm", 1, initial)
                binding = native_report(self.root, "glm", 2, later)
                with self.assertRaises(review.ReviewError) as error:
                    review._model_pair(self.state, "glm", "2", binding)
                self.assertEqual(error.exception.code, "peer-model-unverified")

    def test_legacy_and_changed_native_reports_cannot_verify(self):
        original = copy.deepcopy(self.state)
        for phase in ("initial", "cross"):
            self.state = copy.deepcopy(original)
            rec = self.state["reviews"]["glm"] if phase == "initial" else self.state["cross"][0]
            del rec["native_report"]
            self.assertFalse(review.models_verified(self.state))
        self.state = original
        for number in (1, 2):
            path = Path(self.native["glm"][number]["path"])
            before = path.read_bytes()
            path.write_bytes(before + b"changed report\n")
            self.assertFalse(review.models_verified(self.state))
            path.write_bytes(before)
            self.assertTrue(review.models_verified(self.state))
            path.unlink()
            self.assertFalse(review.models_verified(self.state))
            path.write_bytes(before)

    def test_duplicate_native_identity_keys_rejected(self):
        binding = self.native["glm"][2]
        path = Path(binding["path"])
        path.write_text(path.read_text().replace("turn_id: glm-2", "turn_id: glm-1\nturn_id: glm-2"))
        binding["sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
        with self.assertRaises(review.ReviewError):
            self.pair("glm")

    def test_status_rejects_malformed_cross_fields_as_structured_errors(self):
        (self.root / review.LOCK_FILENAME).touch()
        for entry in ({"slug": []}, {"slug": {}}, {"slug": "glm", "turn": []},
                      {"slug": "glm", "turn": "²"}, {"slug": "glm", "turn": True}):
            with self.subTest(entry=entry):
                (self.root / "review-state.json").write_text(json.dumps({"cross": [entry]}))
                proc = subprocess.run([sys.executable, "-B", str(Path(review.__file__)),
                                       "status", str(self.root)], capture_output=True, text=True)
                self.assertNotEqual(proc.returncode, 0, proc.stdout)
                self.assertNotIn("Traceback", proc.stderr)
                self.assertEqual(json.loads(proc.stdout)["error"]["code"], "state-shape-invalid")

    def test_dual_completed_and_initial_reviews_required(self):
        self.assertTrue(review.build_status(self.state)["review_accepted"])
        ledger(self.root, "k3", ("glm-5.3", "k3-256k"))
        with self.assertRaises(review.ReviewError) as error:
            self.pair("k3")
        self.assertEqual(error.exception.code, "peer-model-mismatch")
        self.assert_blocked()

    def test_missing_peer_or_deleted_anchor_blocks(self):
        original = copy.deepcopy(self.state)
        self.state["cross"].pop()
        self.assertFalse(review.build_status(self.state)["model_verified"])
        self.assertFalse(review.build_status(self.state)["review_accepted"])
        self.state = original
        del self.state["cross"][0]["model_evidence"]
        self.assert_blocked()

    def test_old_turn_cannot_replace_cross(self):
        rec = self.state["cross"][0]
        rec["model_evidence"]["cross"] = rec["model_evidence"]["initial"]
        self.assert_blocked()
        rec["turn"] = "1"
        self.assert_blocked()

    def test_runtime_disappearance_and_raw_model_drift(self):
        ledger(self.root, "k3", ("k3-256k", "k3-new"))
        self.assert_blocked()  # even same-family changes invalidate stored anchors
        self.paths["k3"].unlink()
        self.assert_blocked()

    def test_failed_interrupted_and_output_chunks_are_not_completion(self):
        path = self.paths["k3"]
        original = [json.loads(line) for line in path.read_text().splitlines()]
        for kind in ("turn_error", "turn_interrupted", "stream_end", "response"):
            with self.subTest(kind=kind):
                rows = copy.deepcopy(original)
                rows[-1]["event"]["kind"] = kind
                write_rows(path, rows)
                self.assert_blocked()

    def test_foreign_profile_rows_and_ambiguous_cwd_rejected(self):
        path = self.paths["k3"]
        rows = [json.loads(line) for line in path.read_text().splitlines()]
        rows[-2]["event"]["session_id"] = "foreign:local:tui#peer-k3"
        write_rows(path, rows)
        self.assert_blocked()
        ledger(self.root, "k3", ("k3-256k", "k3-256k"))
        other = self.root / "ui-protocol" / b"fixture:local:tui#peer-k3\0~cwd-ffffff".hex()
        other.mkdir()
        (other / "ledger-1.log").write_bytes(path.read_bytes())
        self.assert_blocked()

    def test_malformed_and_missing_sequences_fail_closed(self):
        path = self.paths["k3"]
        original = path.read_text()
        for value in (None, True, 0, "5"):
            with self.subTest(seq=value):
                rows = [json.loads(line) for line in original.splitlines()]
                rows[-2]["seq"] = value
                write_rows(path, rows)
                self.assert_blocked()
        path.write_text(original + "{broken\n")
        self.assert_blocked()
        path.write_text("\n".join(original.splitlines()[1:]) + "\n")
        self.assert_blocked()

    def test_missing_start_or_conflicting_terminal_rejected(self):
        path = self.paths["k3"]
        rows = [json.loads(line) for line in path.read_text().splitlines()]
        rows[-3]["event"]["kind"] = "envelope"
        write_rows(path, rows)
        self.assert_blocked()
        path = ledger(self.root, "k3", ("k3-256k", "k3-256k"))
        rows = [json.loads(line) for line in path.read_text().splitlines()]
        rows.append({"seq": 7, "event": {**rows[-1]["event"], "kind": "turn_error"}})
        write_rows(path, rows)
        self.assert_blocked()

    def test_rotated_complete_tail_is_not_renumbered(self):
        path = self.paths["k3"]
        rows = [json.loads(line) for line in path.read_text().splitlines()]
        # Rotation itself is safe while all original segments remain.
        tail = path.with_name("ledger-0002.log")
        write_rows(path, rows[:3])
        write_rows(tail, rows[3:])
        self.assertTrue(review.build_status(self.state)["review_accepted"])
        path.unlink()  # retention deleted the first complete turn
        self.assert_blocked()
        with self.assertRaises(review.ReviewError) as error:
            review._verify_peer_model_from_ledger(self.root, SESSION, "k3", "k3", 1,
                                                  native_turn_id="k3-1")
        self.assertEqual(error.exception.code, "peer-model-unverified")
        self.assertIn("新建短 reviewer peer", str(error.exception))
        self.assertIn("初审", str(error.exception))

    def test_model_name_is_not_a_freeform_prefix(self):
        ledger(self.root, "k3", ("k3-256k", "k3pretending"))
        with self.assertRaises(review.ReviewError) as error:
            self.pair("k3")
        self.assertEqual(error.exception.code, "peer-model-mismatch")

    def test_legacy_history_can_be_superseded_but_latest_unverified_blocks(self):
        self.state["cross"].insert(0, {"slug": "glm", "turn": "2"})
        self.assertTrue(review.build_status(self.state)["review_accepted"])
        self.state["cross"].append({"slug": "glm", "turn": "2"})
        self.assert_blocked()

    def test_lifecycle_and_model_rows_without_turn_id_rejected(self):
        path = self.paths["k3"]
        original = [json.loads(line) for line in path.read_text().splitlines()]
        for index in range(len(original)):
            with self.subTest(row=index):
                rows = copy.deepcopy(original)
                del rows[index]["event"]["turn_id"]
                write_rows(path, rows)
                self.assert_blocked()

    def test_runtime_identity_shapes_fail_closed(self):
        original = copy.deepcopy(self.state)
        for key in ("runtime", "session"):
            for value in (None, "", 42, {"unexpected": True}, True):
                with self.subTest(key=key, value=value):
                    self.state = copy.deepcopy(original)
                    self.state[key] = value
                    self.assert_blocked()
                    with self.assertRaises(review.ReviewError):
                        self.pair("glm")

    def test_behavior_failure_still_blocks_valid_models(self):
        self.state["verdicts"]["X"] = {"state": "blocked-on-evidence"}
        status = review.build_status(self.state)
        self.assertTrue(status["model_verified"])
        self.assertFalse(status["review_accepted"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
