"""Strict admission contracts; all defaults are explicit in frozen manifests."""
import re
from urllib.parse import urlsplit

from .safe import Refusal, fields, hash_id, identifier, integer, text

GATES = {"engineering", "authority", "compatibility", "recovery", "mechanism"}


def model(value):
    fields(value, {"id", "endpoint", "api_key_env", "context_tokens", "output_tokens",
                   "input_rate", "output_rate", "temperature", "reasoning_effort"})
    text(value["id"], "model ID", 128)
    parsed = urlsplit(text(value["endpoint"], "endpoint", 512))
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise Refusal("provider endpoint must be credential-free HTTPS")
    if not parsed.path.endswith("/chat/completions"):
        raise Refusal("v1 requires an explicit chat-completions endpoint")
    if not re.fullmatch(r"[A-Z][A-Z0-9_]{0,127}", value["api_key_env"]):
        raise Refusal("invalid API-key environment variable name")
    integer(value["context_tokens"], 1024, 1_048_576, "context ceiling")
    integer(value["output_tokens"], 1, 131_072, "output ceiling")
    if value["output_tokens"] >= value["context_tokens"]:
        raise Refusal("output ceiling must be smaller than context ceiling")
    for field in ("input_rate", "output_rate"):
        integer(value[field], 0, 10**12, field)
    if value["temperature"] != 0 or type(value["temperature"]) is bool:
        raise Refusal("v1 fixes temperature at zero")
    if value["reasoning_effort"] not in (None, "low", "medium", "high"):
        raise Refusal("unsupported explicit reasoning setting")
    return value


def manifest(value):
    fields(value, {"schema_version", "id", "model", "limits", "image", "baseline_binary",
                   "baseline_binary_sha256", "source", "source_commit", "configuration", "memory",
                   "gates", "build", "development_protocol", "confirmation_protocol", "kind"}, {"containment"})
    identifier(value["id"])
    if value["kind"] not in ("research", "fixture"):
        raise Refusal("unknown campaign kind")
    value.setdefault("containment", {"memory_mb": 2048, "scratch_bytes": 1073741824})
    fields(value["containment"], {"memory_mb", "scratch_bytes"})
    integer(value["containment"]["memory_mb"], 512, 16384, "container memory MiB")
    integer(value["containment"]["scratch_bytes"], 67108864, 17179869184, "container scratch bytes")
    model(value["model"])
    fields(value["limits"], {"cost_microusd", "requests", "wall_seconds", "cycles", "repetitions"})
    for name, bounds in {"cost_microusd": (1, 10**12), "requests": (1, 10000),
                         "wall_seconds": (1, 604800), "cycles": (1, 10), "repetitions": (3, 10)}.items():
        integer(value["limits"][name], *bounds, name)
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", value["image"]):
        raise Refusal("local OCI image must be pinned by image ID")
    hash_id(value["baseline_binary_sha256"])
    if not re.fullmatch(r"[0-9a-f]{40}", value["source_commit"]):
        raise Refusal("source requires a full pinned Git commit")
    if not isinstance(value["configuration"], dict) or not isinstance(value["memory"], list):
        raise Refusal("configuration and advisory memory must be explicit snapshots")
    fields(value["configuration"], {"agent_mode"})
    if value["configuration"]["agent_mode"] not in ("single", "text"):
        raise Refusal("v1 supports only explicit single/text candidate strategies")
    if len(value["memory"]) > 100:
        raise Refusal("too many memory entries")
    for entry in value["memory"]:
        text(entry, "memory", 4096)
    if not isinstance(value["gates"], dict) or set(value["gates"]) != GATES:
        raise Refusal("all five frozen gate families are required")
    for gate in [value["build"], *value["gates"].values()]:
        fields(gate, {"argv", "inputs", "timeout_seconds"})
        if not isinstance(gate["argv"], list) or not gate["argv"] or len(gate["argv"]) > 32:
            raise Refusal("gate command must be a bounded argv")
        for arg in gate["argv"]:
            text(arg, "gate argument", 4096)
        if not isinstance(gate["inputs"], dict) or not gate["inputs"]:
            raise Refusal("gate source inputs must be frozen")
        for key, expected in gate["inputs"].items():
            text(key, "gate input", 4096)
            hash_id(expected)
        integer(gate["timeout_seconds"], 1, 3600, "gate timeout")
    return value


def protocol(value, repetitions):
    fields(value, {"schema_version", "id", "seed", "repetitions", "limits", "tasks"})
    identifier(value["id"])
    integer(value["seed"], 0, 2**32 - 1, "protocol seed")
    if value["repetitions"] != repetitions:
        raise Refusal("protocol repetitions differ from campaign")
    fields(value["limits"], {"model_turns", "wall_seconds", "output_tokens", "context_tokens", "model_tokens"})
    for name, high in {"model_turns": 200, "wall_seconds": 3600, "output_tokens": 131072,
                       "context_tokens": 1048576, "model_tokens": 1000000000}.items():
        integer(value["limits"][name], 1, high, name)
    tasks = value["tasks"]
    if not isinstance(tasks, list) or not 1 <= len(tasks) <= 50:
        raise Refusal("protocol requires 1–50 tasks")
    if len({t.get("id") for t in tasks}) != len(tasks):
        raise Refusal("duplicate task identity")
    for task in tasks:
        fields(task, {"id", "repository", "commit", "gold_commit", "goal", "targeted", "regression", "image"})
        identifier(task["id"])
        text(task["goal"], "goal")
        for name in ("commit", "gold_commit"):
            if not re.fullmatch(r"[0-9a-f]{40}", task[name]):
                raise Refusal("task requires full base/gold commit")
        if task["commit"] == task["gold_commit"]:
            raise Refusal("gold cannot equal known bad state")
        if not re.fullmatch(r"sha256:[0-9a-f]{64}", task["image"]):
            raise Refusal("grader image must be locally pinned")
        for name in ("targeted", "regression"):
            gate = task[name]
            fields(gate, {"argv", "inputs", "timeout_seconds"})
            if not isinstance(gate["argv"], list) or not 1 <= len(gate["argv"]) <= 32:
                raise Refusal("invalid grader argv")
            for arg in gate["argv"]: text(arg, "grader argument", 4096)
            if not isinstance(gate["inputs"], dict) or not gate["inputs"]:
                raise Refusal("external grader inputs are required")
            for path, expected in gate["inputs"].items():
                text(path, "grader input", 4096)
                hash_id(expected)
            integer(gate["timeout_seconds"], 1, 3600, "grader timeout")
    return value


def proposal(value):
    fields(value, {"schema_version", "id", "parent", "weaknesses", "hypothesis", "mechanism",
                   "predicted_gain", "risks", "primary_metric"})
    identifier(value["id"])
    hash_id(value["parent"])
    if not isinstance(value["weaknesses"], list) or not 1 <= len(value["weaknesses"]) <= 16:
        raise Refusal("proposal requires bounded observed weakness references")
    for item in value["weaknesses"]:
        hash_id(item)
    for field in ("hypothesis", "mechanism", "risks"):
        text(value[field], field, 8000)
    if value["primary_metric"] != "strict_completion":
        raise Refusal("v1 primary outcome is preregistered strict completion")
    if type(value["predicted_gain"]) not in (float, int) or not 0.05 <= value["predicted_gain"] <= 1:
        raise Refusal("predicted gain must be finite and at least five percentage points")
    return value
