"""Bounded, canonical, non-executable inputs shared by the supervisor."""
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat

MAX_JSON = 1_048_576
MAX_ARTIFACT = 64 * 1024 * 1024
MAX_FILES = 100_000
MAX_TREE = 256 * 1024 * 1024


class Refusal(ValueError):
    """An explicit fail-closed admission or integrity diagnostic."""


def canonical(value):
    try:
        return json.dumps(value, sort_keys=True, separators=(",", ":"),
                          ensure_ascii=False, allow_nan=False).encode("utf-8")
    except (ValueError, TypeError, UnicodeError, RecursionError) as error:
        raise Refusal("value is not bounded canonical JSON") from error


def digest(data):
    return hashlib.sha256(data).hexdigest()


def identifier(value):
    if not isinstance(value, str) or not re.fullmatch(r"[a-z0-9][a-z0-9-]{0,63}", value):
        raise Refusal("identifier must be 1–64 lowercase letters, digits or hyphens")
    return value


def hash_id(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value):
        raise Refusal("SHA-256 identity must be 64 lowercase hex characters")
    return value


def integer(value, low, high, name):
    if type(value) is not int or not low <= value <= high:
        raise Refusal(f"{name} must be an integer in [{low}, {high}]")
    return value


def fields(value, required, optional=()):
    if not isinstance(value, dict) or set(value) - set(required) - set(optional) or set(required) - set(value):
        raise Refusal("record has unknown or missing fields")
    if "schema_version" in required and (type(value["schema_version"]) is not int or value["schema_version"] != 1):
        raise Refusal("unsupported lab schema")
    if len(canonical(value)) > MAX_JSON:
        raise Refusal("oversized record")
    return value


def text(value, name, limit=16000):
    if not isinstance(value, str) or not value.strip() or len(value.encode("utf-8")) > limit or "\x00" in value:
        raise Refusal(f"invalid {name}")
    return value


def decode(data, limit=MAX_JSON):
    if len(data) > limit:
        raise Refusal("oversized JSON")
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise Refusal("duplicate JSON field")
            result[key] = value
        return result
    def constant(_):
        raise Refusal("non-finite JSON number")
    try:
        return json.loads(data, object_pairs_hook=pairs, parse_constant=constant)
    except (ValueError, UnicodeError, RecursionError) as error:
        raise Refusal("malformed JSON") from error


def real_path(path, directory=False):
    path = Path(os.path.abspath(path))
    for part in (path, *path.parents):
        if part.is_symlink():
            raise Refusal("symlink in trusted path")
    mode = path.stat().st_mode
    if not (stat.S_ISDIR(mode) if directory else stat.S_ISREG(mode)):
        raise Refusal("trusted path has incorrect file type")
    return path


def read(path, limit=MAX_ARTIFACT):
    path = real_path(path)
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    with os.fdopen(os.open(path, flags), "rb") as stream:
        if os.fstat(stream.fileno()).st_size > limit:
            raise Refusal("oversized file")
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise Refusal("file grew past its limit")
    return data


def relative(value):
    if not isinstance(value, str) or not value or "\x00" in value or "\\" in value or ":" in value:
        raise Refusal("invalid relative path")
    path = PurePosixPath(value)
    if path.is_absolute() or any(p in ("", ".", "..") for p in value.split("/")):
        raise Refusal("path escape or noncanonical relative path")
    return path


def clean_environment():
    # Configuration/credential discovery is deliberately not inherited.
    return {key: os.environ[key] for key in ("PATH", "LANG", "SYSTEMROOT") if key in os.environ}


def tree(root, exclude=()):
    root = real_path(root, True)
    result, size = [], 0
    stack = [root]
    while stack:
        directory = stack.pop()
        for path in sorted(directory.iterdir()):
            name = path.relative_to(root).as_posix()
            if any(name == p or name.startswith(p + "/") for p in exclude):
                continue
            mode = path.lstat().st_mode
            if stat.S_ISDIR(mode):
                if len(path.relative_to(root).parts) > 64:
                    raise Refusal("source depth exceeds limit")
                result.append({"path": name, "kind": "directory"})
                stack.append(path)
            elif stat.S_ISREG(mode):
                data = read(path)
                size += len(data)
                result.append({"path": name, "kind": "file", "digest": digest(data),
                               "bytes": len(data), "executable": bool(mode & 0o111)})
            else:
                raise Refusal("source contains a link or nonregular entry")
            if len(result) > MAX_FILES or size > MAX_TREE:
                raise Refusal("source exceeds file/byte bounds")
    return sorted(result, key=lambda row: row["path"])
