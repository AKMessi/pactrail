import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from pactrail_lab import budget, snapshots
from pactrail_lab.safe import Refusal, decode, integer, relative, tree
from pactrail_lab.store import Store
from unittest.mock import patch


class Foundation(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.store = Store(self.root / "campaign", True)

    def tearDown(self):
        self.store.close()
        self.temp.cleanup()

    def test_strict_hostile_json_and_paths(self):
        for raw in (b'{"x":1,"x":2}', b'{"x":NaN}', b'[' * 2000):
            with self.assertRaises(Refusal): decode(raw)
        for name in ("../x", "/x", "x/../y", "a\\b", "a//b", "a/./b", "C:x"):
            with self.assertRaises(Refusal): relative(name)
        with self.assertRaises(Refusal): integer(True, 0, 1, "number")

    def test_idempotence_leases_and_journal(self):
        def action():
            self.store.set("active", "parent")
            return {"accepted": True}
        head = self.store.head()
        first = self.store.mutate("command", head, {"action": "start"}, action)
        self.assertEqual(first, self.store.mutate("command", head, {"action": "start"}, action))
        with self.assertRaises(Refusal): self.store.mutate("command", head, {"action": "different"}, action)
        with self.assertRaises(Refusal): self.store.mutate("other", head, {}, action)
        self.store.verify()
        self.store.db.execute("UPDATE values_store SET value=? WHERE name='active'", (json.dumps("child"),))
        with self.assertRaisesRegex(Refusal, "projection"): self.store.verify()

    def test_transaction_failure_rolls_back(self):
        head = self.store.head()
        def broken():
            self.store.set("active", "wrong")
            raise OSError("disk failure")
        with self.assertRaises(OSError): self.store.mutate("failed", head, {}, broken)
        self.assertEqual(head, self.store.head())
        self.assertIsNone(self.store.value("active"))

    def test_auto_aborted_sqlite_commit_preserves_storage_error(self):
        database = self.store.db
        class StorageFailure:
            @property
            def in_transaction(self): return database.in_transaction
            def execute(self, sql, *args):
                if sql == "COMMIT":
                    database.execute("ROLLBACK")
                    raise sqlite3.OperationalError("injected disk I/O error")
                return database.execute(sql, *args)
        before = self.store.head()
        with patch.object(self.store, "db", StorageFailure()):
            with self.assertRaisesRegex(sqlite3.OperationalError, "injected disk I/O"):
                with self.store.transaction(): self.store.set("active", "uncommitted-child")
        self.assertEqual(self.store.head(), before)
        self.assertIsNone(self.store.value("active"))
        self.store.verify()

    def test_corruption_unavailable_and_schema(self):
        key = self.store.put(b"evidence")
        self.store.db.execute("UPDATE objects SET body=? WHERE digest=?", (b"forged", key))
        with self.assertRaisesRegex(Refusal, "digest"): self.store.get(key)
        with self.assertRaisesRegex(Refusal, "unavailable"): self.store.get("f" * 64)
        self.store.db.execute("PRAGMA user_version=999")
        with self.assertRaisesRegex(Refusal, "schema"): Store(self.root / "campaign")

    def test_roundtrip_source_unicode_crlf_modes_and_empty_directories(self):
        source = self.root / "source"
        source.mkdir()
        (source / "empty").mkdir()
        p = source / "π.py"
        p.write_bytes(b"a\r\nb\r\n")
        p.chmod(0o755)
        key = snapshots.import_tree(self.store, source)
        snapshots.materialize(self.store, key, self.root / "restored")
        self.assertEqual(tree(source), tree(self.root / "restored"))
        (source / "escape").symlink_to(self.root)
        with self.assertRaises(Refusal): snapshots.import_tree(self.store, source)

    def test_conservative_reservation_no_free_crash_retry(self):
        model = {"context_tokens": 1000, "output_tokens": 100, "input_rate": 1000000, "output_rate": 2000000}
        lease = {"id": "trial", "requests": 2, "deadline_ns": time.time_ns() + 10**9}
        limits = {"requests": 3, "cost_microusd": 2400}
        first = budget.reserve(self.store, "request-one", {"model": "m"}, model, limits, lease)
        self.assertEqual(1200, first["reserved"])
        with self.assertRaisesRegex(Refusal, "replayed"):
            budget.reserve(self.store, "request-one", {"model": "m"}, model, limits, lease)
        budget.settle(self.store, "request-one", self.store.put(b"response"))
        budget.reserve(self.store, "request-two", {"model": "m"}, model, limits, lease)
        with self.assertRaises(Refusal): budget.reserve(self.store, "request-three", {}, model, limits, lease)
        self.assertEqual(2400, sum(r["reserved"] for r in budget.reservations(self.store).values()))

    def test_reopening_does_not_reset_charges(self):
        with self.store.transaction(): self.store.set("charged", 42)
        self.store.close()
        self.store = Store(self.root / "campaign")
        self.assertEqual(42, self.store.value("charged"))

    def test_large_request_history_uses_individual_bounded_records(self):
        with self.store.transaction():
            for i in range(3000):
                request_id = "request-" + str(i).zfill(32)
                row = dict(request_id=request_id, request_hash="a" * 64, reserved=1,
                           lease="previous-" + "x" * 40, state="settled", dispatched=True,
                           usage=dict(input_tokens=1024, output_tokens=128, cached_input_tokens=0), response="b" * 64)
                self.store.set("model-reservation-" + request_id, row)
        rows = budget.reservations(self.store)
        # A monolithic projection would now breach the admitted JSON boundary.
        with self.assertRaises(Refusal): self.store.record(rows)
        model = dict(context_tokens=1024, output_tokens=128, input_rate=0, output_rate=0)
        limits = dict(requests=3001, cost_microusd=3000)
        lease = dict(id="last", requests=1, deadline_ns=time.time_ns() + 30 * 10**9)
        budget.reserve(self.store, "final-request", {}, model, limits, lease)
        self.assertEqual(len(budget.reservations(self.store)), 3001)
        self.store.verify()

    def test_old_reservation_projection_keeps_charge_and_uncertain_dispatch(self):
        old = dict(request_id="old-request", request_hash="a" * 64, reserved=7,
                   lease="old", state="reserved", usage=None)
        with self.store.transaction():
            self.store.set("reservations", {"old-request": old})
            self.store.set("dispatched", {"old-request": True})
        self.assertEqual(budget.reservations(self.store)["old-request"]["reserved"], 7)
        budget.settle(self.store, "old-request", self.store.put(b"old response"))
        self.assertEqual(budget.reservations(self.store)["old-request"]["state"], "settled")
        self.assertEqual(budget.reservations(self.store)["old-request"]["reserved"], 7)
        self.store.verify()


if __name__ == "__main__": unittest.main()
