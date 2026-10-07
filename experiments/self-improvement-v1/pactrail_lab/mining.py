"""Observable failure categories and bounded source context, not model truth."""
from .safe import canonical


def classify(row):
    if row["status"] != "completed": return "infrastructure-or-provider"
    if row["task_success"] and not row["strict_completion"]: return "completion-or-assurance"
    observations = row["outcome"].get("observations", [])
    text = " ".join(str(e.get("summary", "")) for e in observations).lower()
    if any(word in text for word in ("budget", "turn limit", "token limit", "exhausted")): return "budget-exhaustion"
    if any(word in text for word in ("not authorized", "permission", "policy denied")): return "policy-or-verification-configuration"
    if any(e.get("succeeded") is False and str(e.get("actor", "")).startswith("tool:") for e in observations): return "tool-interface"
    return "solution-or-localization-unresolved"


def source_context(store, revision, budget_bytes):
    source = store.load(store.load(revision)["source"])
    wanted = ("controller.rs", "adaptive.rs", "edit_file.rs", "verification.rs", "completion.rs", "context.rs")
    entries = [e for e in source["entries"] if e["kind"] == "file"]
    selected, used = [], 0
    for entry in sorted(entries, key=lambda e: (not any(e["path"].endswith(n) for n in wanted), e["path"])):
        if not entry["path"].endswith(".rs"): continue
        content = store.get(entry["digest"]).decode("utf-8", "replace")[:2400]
        record = {"path": entry["path"], "digest": entry["digest"], "prefix_excerpt": content,
                  "truncated": entry["bytes"] > len(content.encode())}
        size = len(canonical(record))
        if used + size > budget_bytes: continue
        selected.append(record)
        used += size
        if len(selected) >= 6: break
    return selected
