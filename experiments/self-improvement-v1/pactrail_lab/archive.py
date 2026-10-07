"""No extractall, links, executable deserialization or compressed archives."""
import io
from pathlib import Path
import tarfile

from .safe import MAX_ARTIFACT, MAX_FILES, Refusal, relative, tree


def pack(root):
    entries = tree(root)
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w", format=tarfile.PAX_FORMAT) as archive:
        for entry in entries:
            path = Path(root) / entry["path"]
            info = tarfile.TarInfo(entry["path"])
            info.mtime = 0
            if entry["kind"] == "directory":
                info.type, info.mode = tarfile.DIRTYPE, 0o755
                archive.addfile(info)
            else:
                info.size, info.mode = entry["bytes"], 0o755 if entry["executable"] else 0o644
                if buffer.tell() + info.size > MAX_ARTIFACT - 10240:
                    raise Refusal("retained execution archive exceeds 64 MiB")
                with path.open("rb") as stream: archive.addfile(info, stream)
    data = buffer.getvalue()
    if len(data) > MAX_ARTIFACT: raise Refusal("oversized execution archive")
    return data


def unpack(data, root):
    if not isinstance(data, bytes) or len(data) > MAX_ARTIFACT:
        raise Refusal("invalid execution archive size")
    root = Path(root)
    root.mkdir(parents=True, exist_ok=False, mode=0o700)
    seen, total = set(), 0
    try:
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:") as archive:
            for entry in archive:
                name = str(relative(entry.name.rstrip("/") if entry.isdir() else entry.name))
                if name in seen or len(seen) >= MAX_FILES or not (entry.isfile() or entry.isdir()):
                    raise Refusal("duplicate or unsafe archive entry")
                seen.add(name)
                if entry.size < 0 or entry.size > MAX_ARTIFACT or entry.mode & ~0o777:
                    raise Refusal("invalid archive metadata")
                total += entry.size
                if total > MAX_ARTIFACT: raise Refusal("archive logical size exceeds bound")
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                if entry.isdir():
                    path.mkdir(exist_ok=True)
                else:
                    with archive.extractfile(entry) as stream:
                        body = stream.read(entry.size + 1)
                    if len(body) != entry.size: raise Refusal("archive length mismatch")
                    path.write_bytes(body)
                    path.chmod(0o755 if entry.mode & 0o111 else 0o644)
    except (tarfile.TarError, OSError) as error:
        raise Refusal("malformed execution archive") from error
