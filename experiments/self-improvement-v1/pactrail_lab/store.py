"""One transactional journal; projections cannot change an admitted decision."""
import contextlib
import os
from pathlib import Path
import sqlite3
import time

from .safe import MAX_ARTIFACT, MAX_JSON, Refusal, canonical, decode, digest, hash_id, identifier, real_path


class Store:
    def __init__(self, root, create=False):
        self.root = Path(os.path.abspath(root))
        if create:
            self.root.mkdir(mode=0o700, parents=True, exist_ok=False)
        real_path(self.root, True)
        path = self.root / "journal.sqlite3"
        if path.exists():
            real_path(path)
        elif not create:
            raise Refusal("campaign journal is unavailable")
        for suffix in ("-wal", "-shm", "-journal"):
            if Path(str(path) + suffix).is_symlink():
                raise Refusal("redirected SQLite sidecar")
        self.db = sqlite3.connect(path, timeout=10, isolation_level=None)
        self.db.execute("PRAGMA foreign_keys=ON")
        self.db.execute("PRAGMA synchronous=FULL")
        if create:
            self.db.executescript("""
                PRAGMA user_version=1;
                CREATE TABLE objects(digest TEXT PRIMARY KEY, body BLOB NOT NULL);
                CREATE TABLE events(seq INTEGER PRIMARY KEY, previous TEXT NOT NULL,
                    body BLOB NOT NULL, digest TEXT UNIQUE NOT NULL);
                CREATE TABLE commands(id TEXT PRIMARY KEY, request TEXT NOT NULL, result TEXT NOT NULL);
                CREATE TABLE values_store(name TEXT PRIMARY KEY, value TEXT NOT NULL);
            """)
        elif self.db.execute("PRAGMA user_version").fetchone()[0] != 1:
            self.close()
            raise Refusal("unsupported campaign database schema")
        self.verify()

    def close(self):
        self.db.close()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()

    @contextlib.contextmanager
    def transaction(self):
        self.db.execute("BEGIN IMMEDIATE")
        try:
            yield
            self.db.execute("COMMIT")
        except BaseException:
            self.db.execute("ROLLBACK")
            raise

    def put(self, data):
        if not isinstance(data, bytes) or len(data) > MAX_ARTIFACT:
            raise Refusal("oversized or nonbinary artifact")
        key = digest(data)
        self.db.execute("INSERT OR IGNORE INTO objects VALUES (?, ?)", (key, data))
        if self.get(key) != data:
            raise Refusal("artifact identity collision")
        return key

    def get(self, key, limit=MAX_ARTIFACT):
        hash_id(key)
        row = self.db.execute("SELECT length(body) FROM objects WHERE digest=?", (key,)).fetchone()
        if row is None:
            raise Refusal("checkpoint references unavailable artifact")
        if row[0] > limit:
            raise Refusal("artifact exceeds admitted read limit")
        data = self.db.execute("SELECT body FROM objects WHERE digest=?", (key,)).fetchone()[0]
        if digest(data) != key:
            raise Refusal("artifact digest mismatch")
        return data

    def record(self, value):
        data = canonical(value)
        if len(data) > MAX_JSON:
            raise Refusal("record exceeds limit")
        return self.put(data)

    def load(self, key):
        return decode(self.get(key, MAX_JSON))

    def head(self):
        row = self.db.execute("SELECT digest FROM events ORDER BY seq DESC LIMIT 1").fetchone()
        return row[0] if row else "0" * 64

    def value(self, name, default=None):
        row = self.db.execute("SELECT value FROM values_store WHERE name=?", (name,)).fetchone()
        return decode(row[0]) if row else default

    def set(self, name, value):
        key = self.record(value)
        self.append("projection-set", {"name": name, "record": key})
        self.db.execute("INSERT INTO values_store VALUES (?, ?) ON CONFLICT(name) DO UPDATE SET value=excluded.value",
                        (name, canonical(value)))

    def append(self, kind, payload):
        identifier(kind)
        seq = self.db.execute("SELECT COALESCE(MAX(seq), -1)+1 FROM events").fetchone()[0]
        previous = self.head()
        body = canonical({"schema_version": 1, "sequence": seq, "previous": previous,
                          "kind": kind, "payload": payload, "time_ns": time.time_ns()})
        key = digest(body)
        self.db.execute("INSERT INTO events VALUES (?, ?, ?, ?)", (seq, previous, body, key))
        return key

    def events(self):
        return [decode(row[0]) for row in self.db.execute("SELECT body FROM events ORDER BY seq")]

    def verify(self):
        previous, count, projection, commands = "0" * 64, 0, {}, {}
        for seq, before, body, key in self.db.execute("SELECT * FROM events ORDER BY seq"):
            value = decode(body)
            if seq != count or before != previous or digest(body) != key or value.get("sequence") != seq or value.get("previous") != before:
                raise Refusal("campaign journal integrity mismatch")
            previous, count = key, count + 1
            if value.get("kind") == "projection-set":
                payload = value["payload"]
                projection[payload["name"]] = self.load(payload["record"])
            elif value.get("kind") == "command-completed":
                payload = value["payload"]
                self.load(payload["result"])
                commands[payload["id"]] = (payload["request"], payload["result"])
        persisted = {name: decode(value) for name, value in self.db.execute("SELECT * FROM values_store")}
        if persisted != projection:
            raise Refusal("campaign projection differs from authoritative journal")
        persisted_commands = {name: (request, result) for name, request, result in self.db.execute("SELECT * FROM commands")}
        if persisted_commands != commands:
            raise Refusal("command cache differs from authoritative journal")

    def mutate(self, command_id, expected_head, request, action):
        identifier(command_id)
        request_hash = digest(canonical(request))
        with self.transaction():
            self.verify()
            old = self.db.execute("SELECT request,result FROM commands WHERE id=?", (command_id,)).fetchone()
            if old:
                if old[0] != request_hash:
                    raise Refusal("command ID reused with different content")
                return self.load(old[1])
            if expected_head != self.head():
                raise Refusal("stale campaign head; inspect before retry")
            result = action()
            result_key = self.record(result)
            self.append("command-completed", {"id": command_id, "request": request_hash, "result": result_key})
            self.db.execute("INSERT INTO commands VALUES (?, ?, ?)", (command_id, request_hash, result_key))
            return result
