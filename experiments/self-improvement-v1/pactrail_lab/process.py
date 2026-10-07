"""Bounded supervisor operations; candidate execution requires OCI."""
import os
from pathlib import Path
import selectors
import signal
import subprocess
import time
import uuid

from .safe import Refusal, clean_environment, real_path, hash_id, relative


def command(argv, *, timeout=30, limit=1_048_576, env=None, cwd=None):
    if not argv or any(not isinstance(v, str) or "\x00" in v for v in argv):
        raise Refusal("invalid bounded command")
    if os.name != "posix":
        raise Refusal("supervisor execution requires POSIX process groups")
    started = time.monotonic()
    child = subprocess.Popen(argv, cwd=cwd, env=env or clean_environment(),
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
    output = {"stdout": bytearray(), "stderr": bytearray()}
    reason = None
    with selectors.DefaultSelector() as selection:
        for name, stream in (("stdout", child.stdout), ("stderr", child.stderr)):
            os.set_blocking(stream.fileno(), False)
            selection.register(stream, selectors.EVENT_READ, name)
        try:
            while selection.get_map():
                if time.monotonic() - started >= timeout:
                    reason = "timeout"
                    break
                for key, _ in selection.select(min(0.1, max(0, timeout - (time.monotonic() - started)))):
                    block = os.read(key.fd, 65536)
                    if not block:
                        selection.unregister(key.fileobj)
                    elif sum(len(v) for v in output.values()) + len(block) > limit:
                        reason = "output-limit"
                        break
                    else:
                        output[key.data].extend(block)
                if reason:
                    break
        finally:
            # Also remove children when the main process exits but descendants
            # keep pipes open. Completion never grants daemon authority.
            try: os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError: pass
            child.wait()
            child.stdout.close()
            child.stderr.close()
    return {"exit_code": child.returncode, "reason": reason,
            "wall_time_ms": round((time.monotonic() - started) * 1000),
            "stdout": bytes(output["stdout"]), "stderr": bytes(output["stderr"])}


class Oci:
    def __init__(self, store, image, runtime="docker"):
        if not isinstance(image, str) or not image.startswith("sha256:"):
            raise Refusal("OCI image is not pinned")
        hash_id(image[7:])
        if os.name != "posix" or not hasattr(os, "getuid") or os.getuid() == 0:
            raise Refusal("candidate runner requires a non-root Linux operator")
        self.store, self.image, self.runtime = store, image, runtime

    def probe(self):
        import sys
        if sys.platform != "linux":
            raise Refusal("candidate containment currently requires Linux")
        result = command([self.runtime, "image", "inspect", "--format", "{{.Id}}", self.image])
        if result["exit_code"] or result["reason"] or result["stdout"].decode().strip() != self.image:
            raise Refusal("pinned local OCI image unavailable; no pull/native fallback")

    def run(self, argv, mounts, *, timeout=120, memory_mb=2048, output_limit=8_388_608, environment=None):
        self.probe()
        arguments_mounts, destinations = [], set()
        for source, target, writable in mounts:
            if not isinstance(target, str) or not target.startswith("/"):
                raise Refusal("invalid container mount")
            relative(target[1:])
            if target == "/work" or target.startswith("/work/") or target == "/tmp" or target.startswith("/tmp/"):
                raise Refusal("writable storage must use bounded container tmpfs")
            if writable:
                raise Refusal("candidate host bind mounts must be read-only")
            if target in destinations or any(target.startswith(p + "/") or p.startswith(target + "/") for p in destinations):
                raise Refusal("overlapping container mounts")
            path = Path(source)
            if target == "/gateway.sock":
                import stat
                real_path(path.parent, True)
                if path.is_symlink() or not stat.S_ISSOCK(path.lstat().st_mode):
                    raise Refusal("invalid gateway socket mount")
            else:
                real_path(path, path.is_dir())
            if "," in str(path):
                raise Refusal("ambiguous container mount")
            destinations.add(target)
            arguments_mounts += ["--mount", f"type=bind,src={path},dst={target}" + ("" if writable else ",readonly")]
        environment = environment or {}
        if set(environment) - {"PACTRAIL_LAB_LEASE"} or any(not isinstance(v, str) or "\x00" in v or "\n" in v for v in environment.values()):
            raise Refusal("unapproved candidate environment")
        name = "pactrail-lab-" + uuid.uuid4().hex
        with self.store.transaction():
            pending = self.store.value("containers", {})
            pending[name] = {"image": self.image, "state": "reserved"}
            self.store.set("containers", pending)
            self.store.append("container-reserved", {"name": name, "image": self.image})
        arguments = [self.runtime, "create", "--name", name, "--label", "org.pactrail.lab=" + self.store.root.name,
                     "--restart=no", "--read-only", "--network=none", "--cap-drop=ALL",
                     "--security-opt=no-new-privileges", "--pids-limit=128", "--memory", str(memory_mb) + "m",
                     "--cpus=2", "--user", f"{os.getuid()}:{os.getgid()}",
                     "--tmpfs", "/tmp:rw,noexec,nosuid,size=67108864,mode=1777", "--workdir", "/work",
                     "--tmpfs", f"/work:rw,nosuid,size=1073741824,mode=0700,uid={os.getuid()},gid={os.getgid()}",
                     "--env", "HOME=/tmp", "--env", "PYTHONDONTWRITEBYTECODE=1"]
        arguments += arguments_mounts
        for key, value in environment.items():
            arguments += ["--env", key + "=" + value]
        if not argv: raise Refusal("container command is required")
        arguments += ["--entrypoint", argv[0], self.image, *argv[1:]]
        try:
            created = command(arguments)
            with self.store.transaction():
                self.store.append("container-admission", {"name": name, "stdout": self.store.put(created["stdout"]),
                                  "stderr": self.store.put(created["stderr"]), "exit_code": created["exit_code"]})
            if created["exit_code"] or created["reason"]:
                raise Refusal("OCI create failed; consult retained supervisor diagnostics")
            with self.store.transaction():
                self.store.append("container-created", {"name": name})
            return command([self.runtime, "start", "--attach", name], timeout=timeout, limit=output_limit)
        finally:
            removed = command([self.runtime, "rm", "--force", name], timeout=15)
            with self.store.transaction():
                pending = self.store.value("containers", {})
                pending[name]["state"] = "removed" if removed["exit_code"] == 0 else "cleanup-required"
                self.store.set("containers", pending)
                self.store.append("container-cleanup", {"name": name, "exit_code": removed["exit_code"]})

    def recover(self):
        for name, row in self.store.value("containers", {}).items():
            if row["state"] == "removed":
                continue
            label = command([self.runtime, "inspect", "--format", '{{index .Config.Labels "org.pactrail.lab"}}', name])
            if label["reason"] or (label["exit_code"] and (b"No such object" not in label["stderr"] and b"No such container" not in label["stderr"])):
                raise Refusal("cannot establish container state; recovery remains blocked")
            if label["exit_code"] == 0 and label["stdout"].decode().strip() != self.store.root.name:
                raise Refusal("container ownership mismatch during recovery")
            if label["exit_code"] == 0:
                removed = command([self.runtime, "rm", "--force", name], timeout=15)
                if removed["exit_code"]:
                    raise Refusal("named container cleanup failed")
            with self.store.transaction():
                pending = self.store.value("containers", {})
                pending[name]["state"] = "removed"
                self.store.set("containers", pending)
                self.store.append("container-recovered", {"name": name})
