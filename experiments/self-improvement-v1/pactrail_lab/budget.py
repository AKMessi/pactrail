"""External physical-request reservations survive failures and crashes."""
import time
from .safe import Refusal, canonical, digest, integer


def reservations(store):
    """Individual projections keep one request from rewriting all past usage.

    Read the earlier map format too; migrated entries override it by identity.
    The event journal still binds each projection, without a database migration.
    """
    result = store.value("reservations", {})
    for (name,) in store.db.execute("SELECT name FROM values_store WHERE name LIKE 'model-reservation-%' ORDER BY name"):
        row = store.value(name)
        result[row["request_id"]] = row
    return result


def maximum_charge(model):
    # Ignore optimistic cache discounts and reserve the whole declared context.
    numerator = model["context_tokens"] * model["input_rate"] + model["output_tokens"] * model["output_rate"]
    return (numerator + 999999) // 1000000


def reserve(store, request_id, request, model, limits, lease):
    integer(lease["deadline_ns"], 1, 2**63 - 1, "lease deadline")
    request_hash = digest(canonical(request))
    charge = maximum_charge(model)
    def action():
        existing = reservations(store)
        if len(existing) >= limits["requests"]:
            raise Refusal("campaign model-request budget exhausted")
        if time.time_ns() >= lease["deadline_ns"]:
            raise Refusal("model lease expired")
        deadline = store.value("deadline-ns")
        if deadline is not None and time.time_ns() >= deadline:
            raise Refusal("campaign wall-clock budget exhausted")
        if sum(r["reserved"] for r in existing.values()) + charge > limits["cost_microusd"]:
            raise Refusal("campaign spending reservation would exceed cap")
        count = sum(r["lease"] == lease["id"] for r in existing.values())
        if count >= lease["requests"]:
            raise Refusal("trial model-request budget exhausted")
        result = {"request_id": request_id, "request_hash": request_hash, "reserved": charge,
                  "lease": lease["id"], "state": "reserved", "usage": None}
        store.set("model-reservation-" + request_id, result)
        store.append("model-reserved", result)
        return result
    result = store.mutate(request_id, store.head(), {"operation": "reserve", "hash": request_hash, "lease": lease}, action)
    # Admission identity is durable, but external I/O is not safely repeatable.
    if reservations(store)[request_id].get("dispatched") or store.value("dispatched", {}).get(request_id):
        raise Refusal("model invocation already dispatched; uncertain requests cannot be replayed")
    with store.transaction():
        row = reservations(store)[request_id]
        if row.get("dispatched"):
            raise Refusal("duplicate model dispatch")
        row["dispatched"] = True
        store.set("model-reservation-" + request_id, row)
    return result


def settle(store, request_id, response_artifact, usage=None):
    with store.transaction():
        existing = reservations(store)
        if request_id not in existing or existing[request_id]["state"] != "reserved":
            raise Refusal("unknown or already settled model request")
        if usage is not None:
            for key in ("input_tokens", "output_tokens"):
                integer(usage[key], 0, 2**63 - 1, key)
        row = existing[request_id]
        row["state"] = "settled"
        row["usage"] = usage
        row["response"] = response_artifact
        # Keep the conservative charge even when usage is absent or provider
        # billing cannot be independently reconstructed.
        store.set("model-reservation-" + request_id, row)
        store.append("model-settled", row)
