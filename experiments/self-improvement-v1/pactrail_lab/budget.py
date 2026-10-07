"""External physical-request reservations survive failures and crashes."""
import time
from .safe import Refusal, canonical, digest, integer


def maximum_charge(model):
    # Ignore optimistic cache discounts and reserve the whole declared context.
    numerator = model["context_tokens"] * model["input_rate"] + model["output_tokens"] * model["output_rate"]
    return (numerator + 999999) // 1000000


def reserve(store, request_id, request, model, limits, lease):
    integer(lease["deadline_ns"], 1, 2**63 - 1, "lease deadline")
    request_hash = digest(canonical(request))
    charge = maximum_charge(model)
    def action():
        reservations = store.value("reservations", {})
        if len(reservations) >= limits["requests"]:
            raise Refusal("campaign model-request budget exhausted")
        if time.time_ns() >= lease["deadline_ns"]:
            raise Refusal("model lease expired")
        deadline = store.value("deadline-ns")
        if deadline is not None and time.time_ns() >= deadline:
            raise Refusal("campaign wall-clock budget exhausted")
        if sum(r["reserved"] for r in reservations.values()) + charge > limits["cost_microusd"]:
            raise Refusal("campaign spending reservation would exceed cap")
        count = sum(r["lease"] == lease["id"] for r in reservations.values())
        if count >= lease["requests"]:
            raise Refusal("trial model-request budget exhausted")
        result = {"request_id": request_id, "request_hash": request_hash, "reserved": charge,
                  "lease": lease["id"], "state": "reserved", "usage": None}
        reservations[request_id] = result
        store.set("reservations", reservations)
        store.append("model-reserved", result)
        return result
    result = store.mutate(request_id, store.head(), {"operation": "reserve", "hash": request_hash, "lease": lease}, action)
    # Admission identity is durable, but external I/O is not safely repeatable.
    if store.value("dispatched", {}).get(request_id):
        raise Refusal("model invocation already dispatched; uncertain requests cannot be replayed")
    with store.transaction():
        dispatched = store.value("dispatched", {})
        if request_id in dispatched:
            raise Refusal("duplicate model dispatch")
        dispatched[request_id] = True
        store.set("dispatched", dispatched)
    return result


def settle(store, request_id, response_artifact, usage=None):
    with store.transaction():
        reservations = store.value("reservations", {})
        if request_id not in reservations or reservations[request_id]["state"] != "reserved":
            raise Refusal("unknown or already settled model request")
        if usage is not None:
            for key in ("input_tokens", "output_tokens"):
                integer(usage[key], 0, 2**63 - 1, key)
        reservations[request_id]["state"] = "settled"
        reservations[request_id]["usage"] = usage
        reservations[request_id]["response"] = response_artifact
        # Keep the conservative charge even when usage is absent or provider
        # billing cannot be independently reconstructed.
        store.set("reservations", reservations)
        store.append("model-settled", reservations[request_id])
