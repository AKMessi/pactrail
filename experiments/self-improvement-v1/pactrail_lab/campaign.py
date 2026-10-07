"""A separate authority plane for bounded experiments and reversible selection."""
import os
from pathlib import Path
import random
import time
import uuid

from . import contracts
from .execution import Execution, freeze_inputs, supervisor_inputs, verify_supervisor
from .gateway import Gateway
from .judge import decide
from .process import Oci
from .safe import Refusal, canonical, decode, digest, hash_id, read, real_path, fields, integer
from .snapshots import diff, import_git, revision
from .store import Store


def initialize(root, manifest_path):
    admitted = contracts.manifest(decode(read(manifest_path)))
    binary = read(admitted["baseline_binary"])
    if digest(binary) != admitted["baseline_binary_sha256"]:
        raise Refusal("baseline executable does not match admitted identity")
    protocols = {name: contracts.protocol(decode(read(admitted[name + "_protocol"])), admitted["limits"]["repetitions"])
                 for name in ("development", "confirmation")}
    a, b = protocols.values()
    if {t["id"] for t in a["tasks"]} & {t["id"] for t in b["tasks"]} or \
            {(str(real_path(t["repository"], True)), t["commit"]) for t in a["tasks"]} & \
            {(str(real_path(t["repository"], True)), t["commit"]) for t in b["tasks"]}:
        raise Refusal("development and confirmation task populations overlap")
    for protocol in protocols.values():
        limits = protocol["limits"]
        if limits["context_tokens"] != admitted["model"]["context_tokens"] or limits["output_tokens"] != admitted["model"]["output_tokens"]:
            raise Refusal("protocol/model context or output ceilings differ")
        if limits["wall_seconds"] > admitted["limits"]["wall_seconds"] or limits["model_turns"] > admitted["limits"]["requests"]:
            raise Refusal("trial ceilings exceed campaign budget")
    operations = [admitted["build"], *admitted["gates"].values()]
    operations += [task[name] for protocol in protocols.values() for task in protocol["tasks"] for name in ("targeted", "regression")]
    with Store(root, create=True) as store, store.transaction():
        store.set("supervisor-inputs", supervisor_inputs())
        for path, expected in store.value("supervisor-inputs").items():
            if store.put(read(path)) != expected: raise Refusal("supervisor changed while freezing")
        store.set("external-inputs", freeze_inputs(store, operations))
        store.set("manifest", store.record(admitted))
        source = import_git(store, admitted["source"], admitted["source_commit"])
        baseline = revision(store, source, binary, admitted["configuration"], admitted["memory"])
        store.set("baseline", baseline)
        store.set("active", baseline)
        store.set("lineage", {baseline: {"parent": None, "state": "active"}})
        store.set("deadline-ns", time.time_ns() + admitted["limits"]["wall_seconds"] * 10**9)
        store.set("confirmation-spent", False)
        for name, protocol in protocols.items():
            tasks = []
            for task in protocol["tasks"]:
                base = import_git(store, task["repository"], task["commit"])
                gold = import_git(store, task["repository"], task["gold_commit"])
                tasks.append({**task, "base_source": base, "gold_source": gold})
            store.set(name + "-protocol", store.record({**protocol, "tasks": tasks}))
        store.append("campaign-admitted", {"baseline": baseline, "kind": admitted["kind"]})
        return status(store)


def status(store):
    return {"schema_version": 1, "head": store.head(), "kind": store.load(store.value("manifest"))["kind"],
            "active": store.value("active"), "baseline": store.value("baseline"), "lineage": store.value("lineage"),
            "pending": store.value("pending"), "inflight": store.value("inflight"),
            "confirmation_spent": store.value("confirmation-spent"), "proposals": store.value("proposals", []),
            "verdicts": store.value("verdicts", {}), "reservations": store.value("reservations", {}),
            "qualified": store.value("qualified", False), "baseline_results": store.value("baseline-results")}


def grade(result):
    report = result.get("verification")
    fields(report, {"schema_version", "executed", "passed", "failed", "errors", "skipped"})
    for name in ("executed", "passed", "failed", "errors", "skipped"):
        integer(report[name], 0, 100000, "grader " + name)
    if not report["executed"] or report["executed"] != report["passed"] + report["failed"] or report["errors"] or report["skipped"]:
        raise Refusal("grader did not execute the complete declared behavior checks")
    execution = result.get("execution", {})
    if execution.get("reason") or execution.get("exit_code") not in (0, 1):
        raise Refusal("grader infrastructure failed")
    passed = report["failed"] == 0
    if passed != result["passed"] or execution["exit_code"] != (0 if passed else 1):
        raise Refusal("grader report contradicts its execution")
    return passed


class Campaign:
    def __init__(self, store, execution=None):
        self.store = store
        verify_supervisor(store)
        self.manifest = store.load(store.value("manifest"))
        self.execution = execution or Execution(store)
        if execution is not None and self.manifest["kind"] != "fixture":
            raise Refusal("injected execution is restricted to mechanics fixtures")

    def _action(self, command_id, head, kind, payload, work):
        request = {"operation": kind, **payload}
        def start():
            if self.store.value("inflight"): raise Refusal("another operation is pending; recover before continuing")
            if time.time_ns() >= self.store.value("deadline-ns"): raise Refusal("campaign wall-clock ceiling reached")
            self.store.set("inflight", {"id": command_id, "kind": kind, "request": request, "state": "started"})
            return {"admitted": command_id}
        admitted = self.store.mutate(command_id, head, request, start)
        inflight = self.store.value("inflight")
        if not inflight or inflight["id"] != admitted["admitted"]:
            result = self.store.value("operation-results", {}).get(command_id)
            if result: return self.store.load(result)
            raise Refusal("interrupted operation cannot be silently replayed; use a new command ID")
        # Re-entering the same physical invocation after a crash is forbidden.
        with self.store.transaction():
            if inflight["state"] != "started": raise Refusal("uncertain operation requires recovery")
            inflight["state"] = "dispatched"
            self.store.set("inflight", inflight)
        try:
            result = work()
            with self.store.transaction():
                key = self.store.record(result)
                results = self.store.value("operation-results", {})
                results[command_id] = key
                self.store.set("operation-results", results)
                self.store.set("inflight", None)
                self.store.append("operation-completed", {"id": command_id, "record": key})
            return result
        except Exception:
            with self.store.transaction():
                self.store.append("operation-interrupted", {"id": command_id, "kind": kind, "no_automatic_replay": True})
            raise

    def qualify(self, command_id, head):
        def work():
            baseline = self.store.load(self.store.value("baseline"))
            validations = {}
            for name, operation in sorted(self.manifest["gates"].items()):
                validations[name] = self.execution.operation(baseline["source"], baseline["binary"], operation)
                if not validations[name]["passed"]: raise Refusal("baseline gate failed: " + name)
            for cohort in ("development", "confirmation"):
                protocol = self.store.load(self.store.value(cohort + "-protocol"))
                for task in protocol["tasks"]:
                    tested = []
                    for source, name in ((task["base_source"], "targeted"), (task["gold_source"], "targeted"), (task["gold_source"], "regression")):
                        tested.append(self.execution.operation(source, baseline["binary"], task[name], task["image"]))
                    if [grade(r) for r in tested] != [False, True, True]:
                        raise Refusal("external grader failed bad/gold validation: " + task["id"])
                    validations[cohort + ":" + task["id"]] = tested
            with self.store.transaction():
                self.store.set("qualification", self.store.record(validations))
                self.store.set("qualified", True)
            return {"qualification": self.store.value("qualification"), "passed": True}
        return self._action(command_id, head, "qualify", {}, work)

    def _run(self, protocol, arms, experiment_id):
        order = [(task, arm, repeat) for task in protocol["tasks"] for arm in sorted(arms)
                 for repeat in range(protocol["repetitions"])]
        random.Random(protocol["seed"]).shuffle(order)
        frozen = {"schema_version": 1, "protocol": protocol, "arms": arms,
                  "order": [[t["id"], a, r] for t, a, r in order], "model": self.manifest["model"],
                  "supervisor": self.store.value("supervisor-inputs"), "alpha": .025 / self.manifest["limits"]["cycles"]}
        with self.store.transaction():
            protocol_hash = self.store.record(frozen)
            experiments = self.store.value("experiments", {})
            if experiment_id in experiments: raise Refusal("experiment ID already spent")
            experiments[experiment_id] = {"protocol": protocol_hash, "state": "running", "rows": []}
            self.store.set("experiments", experiments)
            self.store.append("experiment-frozen", {"id": experiment_id, "protocol": protocol_hash})
        rows = []
        blocked = False
        for task, arm, repeat in order:
            trial_id = "trial-" + uuid.uuid4().hex
            with self.store.transaction():
                self.store.append("trial-declared", {"id": trial_id, "experiment": experiment_id,
                                  "task": task["id"], "arm": arm, "repeat": repeat})
            started = time.monotonic()
            try:
                outcome = {"status": "not-attempted", "checks": {}, "candidate_source": None} if blocked else self.execution.trial(arms[arm], task["base_source"], task["goal"], protocol["limits"], trial_id)
            except (Refusal, OSError) as error:
                outcome = {"status": "infrastructure-failed", "checks": {}, "candidate_source": None,
                           "error_type": type(error).__name__}
                blocked = True
            graded = {"targeted": None, "regression": None}
            if outcome.get("candidate_source"):
                binary = self.store.load(self.store.value("baseline"))["binary"]
                for name in graded:
                    graded[name] = self.execution.operation(outcome["candidate_source"], binary, task[name], task["image"])
                    grade(graded[name])
            checks = outcome.get("checks", {})
            functional = all(graded[n] is not None and graded[n]["passed"] is True for n in graded)
            row = {"task": task["id"], "arm": arm, "repeat": repeat, "trial_id": trial_id,
                   "status": outcome["status"], "strict_completion": functional and all(checks.get(n) is True for n in
                       ("source_isolation_valid", "trace_valid", "receipt_valid", "ready_to_apply")),
                   "task_success": functional, "checks": checks, "grading": graded, "outcome": outcome,
                   "wall_time_ms": round((time.monotonic() - started) * 1000)}
            reservations = [r for r in self.store.value("reservations", {}).values() if r["lease"] == trial_id]
            def total(field):
                values = [(r.get("usage") or {}).get(field) for r in reservations]
                return sum(values) if values and all(v is not None for v in values) else None
            row["metrics"] = {"physical_model_requests": len(reservations), "input_tokens": total("input_tokens"),
                              "output_tokens": total("output_tokens"), "cached_input_tokens": total("cached_input_tokens"),
                              "reserved_cost_microusd": sum(r["reserved"] for r in reservations),
                              "actual_cost_microusd": None, "provider_model_time_ms": None,
                              "token_coverage": sum(r.get("usage") is not None for r in reservations)}
            rows.append(row)
            with self.store.transaction():
                experiments = self.store.value("experiments")
                experiments[experiment_id]["rows"].append(self.store.record(row))
                self.store.set("experiments", experiments)
                self.store.append("trial-completed", {"id": trial_id, "record": experiments[experiment_id]["rows"][-1]})
        with self.store.transaction():
            experiments = self.store.value("experiments")
            experiments[experiment_id]["state"] = "complete"
            self.store.set("experiments", experiments)
        return rows, protocol_hash

    def baseline(self, command_id, head):
        def work():
            if not self.store.value("qualified"): raise Refusal("qualify engineering and graders first")
            protocol = self.store.load(self.store.value("development-protocol"))
            rows, frozen = self._run(protocol, {"parent": self.store.value("active")}, "baseline-" + command_id)
            with self.store.transaction():
                key = self.store.record({"rows": rows, "protocol": frozen, "parent": self.store.value("active")})
                self.store.set("baseline-results", key)
                weaknesses = []
                for row in rows:
                    if not row["strict_completion"]:
                        weaknesses.append(self.store.record({"schema_version": 1, "task": row["task"],
                            "trial": self.store.record(row), "classification": "unresolved-development-failure",
                            "observed": {"functional": row["task_success"], "assurance": row["checks"],
                                         "trace_observations": row["outcome"].get("observations", [])}}))
                self.store.set("weaknesses", weaknesses)
            return {"results": key, "weaknesses": weaknesses}
        return self._action(command_id, head, "baseline", {}, work)

    def propose(self, command_id, head):
        def work():
            weaknesses = self.store.value("weaknesses", [])
            if not weaknesses: raise Refusal("no observed development failure available; do not invent a weakness")
            if len(self.store.value("proposals", [])) >= self.manifest["limits"]["cycles"]:
                raise Refusal("proposal cycle budget exhausted")
            parent = self.store.value("active")
            model = self.manifest["model"]
            gateway = Gateway(self.store.root, model, self.manifest["limits"], os.environ.get(model["api_key_env"]))
            token = gateway.lease("proposal-" + command_id, 1, min(300, self.manifest["limits"]["wall_seconds"]))
            proposal_id = "proposal-" + uuid.uuid4().hex[:16]
            template = {"schema_version": 1, "id": proposal_id, "parent": parent, "weaknesses": weaknesses[:16],
                        "hypothesis": "a specific falsifiable statement", "mechanism": "one coherent change and its predicted observable effect",
                        "predicted_gain": .05, "risks": "security and regression risks", "primary_metric": "strict_completion"}
            observations = [self.store.load(key) for key in weaknesses[:4]]
            body = {"model": model["id"], "max_tokens": model["output_tokens"],
                    "reasoning_effort": model["reasoning_effort"], "messages": [
                        {"role": "system", "content": "Propose one testable harness change, never a benchmark-specific fix. Return only JSON matching the template. No claims of improvement without measurement. Preserve the supplied IDs and evidence references. The external verifier, credentials, budgets and promotion authority cannot be changed."},
                        {"role": "user", "content": canonical({"template": template, "observations": observations}).decode()}]}
            try: response = decode(gateway.submit(token, body), 8_388_608)
            finally: gateway.revoke(token)
            proposed = contracts.proposal(decode(response["choices"][0]["message"]["content"]))
            if proposed["id"] != proposal_id or proposed["parent"] != parent or not set(proposed["weaknesses"]) <= set(weaknesses):
                raise Refusal("proposal is not grounded in the active development evidence")
            with self.store.transaction():
                key = self.store.record(proposed)
                proposals = self.store.value("proposals", [])
                if len(proposals) >= self.manifest["limits"]["cycles"]: raise Refusal("proposal cycle budget exhausted")
                self.store.set("proposals", proposals + [key])
                self.store.append("proposal-admitted", {"proposal": key})
            return {"proposal": key}
        return self._action(command_id, head, "propose", {}, work)

    def implement(self, command_id, head, proposal_key):
        hash_id(proposal_key)
        def work():
            if proposal_key not in self.store.value("proposals", []): raise Refusal("unknown proposal")
            proposed = contracts.proposal(self.store.load(proposal_key))
            if proposed["parent"] != self.store.value("active"): raise Refusal("proposal parent is no longer active")
            with self.store.transaction():
                spent = self.store.value("implementation-spent", [])
                if proposal_key in spent: raise Refusal("proposal implementation already attempted; no free retry")
                self.store.set("implementation-spent", spent + [proposal_key])
            parent = self.store.load(proposed["parent"])
            protocol = self.store.load(self.store.value("development-protocol"))
            goal = "Implement this single harness proposal in the isolated candidate. Do not change evaluation criteria or invent evidence. Preserve compatibility.\n" + canonical(proposed).decode()
            outcome = self.execution.trial(proposed["parent"], parent["source"], goal, protocol["limits"], "implement-" + uuid.uuid4().hex)
            if not outcome.get("candidate_source") or not all(outcome["checks"].values()):
                raise Refusal("implementation did not produce a parent-validated reviewable candidate")
            if not diff(self.store, parent["source"], outcome["candidate_source"]): raise Refusal("proposal produced no change")
            built = self.execution.operation(outcome["candidate_source"], parent["binary"], self.manifest["build"])
            if not built["passed"] or not built["binary"]: raise Refusal("candidate build failed or produced no /work/pactrail")
            candidate = revision(self.store, outcome["candidate_source"], self.store.get(built["binary"]),
                                 parent["configuration"], parent["memory"], proposed["parent"], proposal_key)
            gates = {}
            for name, operation in sorted(self.manifest["gates"].items()):
                gates[name] = self.execution.operation(outcome["candidate_source"], built["binary"], operation)
            with self.store.transaction():
                lineage = self.store.value("lineage")
                lineage[candidate] = {"parent": proposed["parent"], "state": "candidate", "proposal": proposal_key,
                                      "gates": gates, "implementation": outcome, "build": built}
                self.store.set("lineage", lineage)
                self.store.append("candidate-created", {"revision": candidate})
            return {"candidate": candidate, "gates": gates}
        return self._action(command_id, head, "implement", {"proposal": proposal_key}, work)

    def evaluate(self, command_id, head, candidate):
        hash_id(candidate)
        def work():
            lineage = self.store.value("lineage")
            entry = lineage.get(candidate)
            if not entry or entry["state"] != "candidate" or entry["parent"] != self.store.value("active"):
                raise Refusal("candidate is not bound to the active parent")
            with self.store.transaction():
                spent = self.store.value("evaluation-spent", [])
                if candidate in spent: raise Refusal("candidate evaluation already attempted; freeze a new candidate instead of cherry-picking a rerun")
                self.store.set("evaluation-spent", spent + [candidate])
            gates = {name: value["passed"] for name, value in entry["gates"].items()}
            if set(gates) != contracts.GATES or not all(gates.values()): raise Refusal("candidate failed a mandatory gate")
            parent = entry["parent"]
            # Distinct trial materializations reconstruct restored source/binary,
            # configuration and memory from the parent record, never from child.
            arms = {"parent": parent, "candidate": candidate, "restored": parent}
            development = self.store.load(self.store.value("development-protocol"))
            rows, frozen = self._run(development, arms, "development-" + command_id)
            result = decide(rows, development, gates, .025 / self.manifest["limits"]["cycles"])
            confirmation_hash = None
            if result["verdict"] == "supported":
                with self.store.transaction():
                    if self.store.value("confirmation-spent"): raise Refusal("confirmation set already spent; initialize a new campaign with fresh tasks")
                    self.store.set("confirmation-spent", True)
                    self.store.append("confirmation-consumed", {"candidate": candidate})
                confirmation = self.store.load(self.store.value("confirmation-protocol"))
                confirm_rows, confirmation_hash = self._run(confirmation, arms, "confirmation-" + command_id)
                result = decide(confirm_rows, confirmation, gates, .025 / self.manifest["limits"]["cycles"])
            result.update(candidate=candidate, parent=parent, development_protocol=frozen,
                          confirmation_protocol=confirmation_hash, kind=self.manifest["kind"], proposal=entry["proposal"])
            if self.manifest["kind"] == "fixture": result["claim"] = "fixture runtime mechanics only; no measured model gain"
            with self.store.transaction():
                key = self.store.record(result)
                verdicts = self.store.value("verdicts", {})
                verdicts[candidate] = key
                self.store.set("verdicts", verdicts)
                self.store.set("pending", key if result["verdict"] == "supported" and confirmation_hash else None)
                self.store.append("selection-verdict", {"record": key})
            return {"verdict": key, "result": result}
        return self._action(command_id, head, "evaluate", {"candidate": candidate}, work)

    def approve(self, command_id, head, verdict_key, acknowledgment):
        hash_id(verdict_key)
        def work():
            if acknowledgment is not True: raise Refusal("explicit human review acknowledgment is required")
            if self.store.value("pending") != verdict_key: raise Refusal("verdict is not the pending exact revision")
            verdict = self.store.load(verdict_key)
            candidate, parent = verdict["candidate"], verdict["parent"]
            lineage = self.store.value("lineage")
            if verdict["verdict"] != "supported" or not verdict["confirmation_protocol"] or parent != self.store.value("active"):
                raise Refusal("promotion requires exact supported confirmation and active-parent binding")
            with self.store.transaction():
                lineage[parent]["state"], lineage[candidate]["state"] = "ancestor", "active"
                self.store.set("lineage", lineage)
                self.store.set("active", candidate)
                self.store.set("pending", None)
                self.store.set("weaknesses", [])
                self.store.append("human-approved", {"revision": candidate, "verdict": verdict_key,
                                  "production_installed": False, "source_applied": False})
            return {"active": candidate, "production_installed": False}
        return self._action(command_id, head, "approve", {"verdict": verdict_key, "acknowledgment": acknowledgment}, work)

    def undo(self, command_id, head, revision_key):
        hash_id(revision_key)
        def work():
            lineage = self.store.value("lineage")
            active = self.store.value("active")
            chain = []
            while active:
                chain.append(active)
                active = lineage[active]["parent"]
            if revision_key not in chain or lineage[revision_key]["parent"] is None:
                raise Refusal("Undo must name an activated logical change; baseline has no parent")
            restore = lineage[revision_key]["parent"]
            retired = set(chain[:chain.index(revision_key) + 1])
            changed = True
            while changed:
                changed = False
                for key, value in lineage.items():
                    if value["parent"] in retired and key not in retired:
                        retired.add(key)
                        changed = True
            restored = self.store.load(restore)
            self.store.get(restored["binary"])
            self.store.load(restored["source"])
            with self.store.transaction():
                for key in retired: lineage[key]["state"] = "retired"
                lineage[restore]["state"] = "active"
                self.store.set("lineage", lineage)
                self.store.set("active", restore)
                self.store.set("pending", None)
                self.store.set("weaknesses", [])
                self.store.append("revision-undone", {"restored": restore, "retired": sorted(retired),
                                  "restores": ["source", "binary", "configuration", "memory"], "production_installed": False})
            return {"active": restore, "retired": sorted(retired), "production_installed": False}
        return self._action(command_id, head, "undo", {"revision": revision_key}, work)

    def recover(self, command_id, head):
        # Recovery is never a replay request. Named containers are cleaned first.
        def admit():
            inflight = self.store.value("inflight")
            self.store.set("inflight", {"id": command_id, "kind": "recover", "interrupted": inflight})
            return {"interrupted": inflight}
        admitted = self.store.mutate(command_id, head, {"operation": "recover"}, admit)
        completed = self.store.value("operation-results", {}).get(command_id)
        if completed: return self.store.load(completed)
        Oci(self.store, self.manifest["image"]).recover()
        with self.store.transaction():
            experiments = self.store.value("experiments", {})
            for entry in experiments.values():
                if entry["state"] == "running": entry["state"] = "interrupted-unqualified"
            self.store.set("experiments", experiments)
            self.store.set("inflight", None)
            self.store.append("campaign-recovered", {**admitted, "requests_replayed": 0})
            result = {**admitted, "requests_replayed": 0}
            results = self.store.value("operation-results", {})
            results[command_id] = self.store.record(result)
            self.store.set("operation-results", results)
        return result


def fork_campaign(parent_root, new_root, manifest_path):
    """A fresh confirmation cohort and budget, with accepted ancestry preserved."""
    with Store(parent_root) as previous:
        verify_supervisor(previous)
        if previous.value("inflight"): raise Refusal("recover parent campaign before forking")
        active_key = previous.value("active")
        active = previous.load(active_key)
        lineage = previous.value("lineage")
        if lineage[active_key]["parent"] is None:
            raise Refusal("recursive continuation requires an accepted change")
        initialize(new_root, manifest_path)
        with Store(new_root) as store, store.transaction():
            current = store.load(store.value("active"))
            for field in ("source", "binary", "configuration", "memory"):
                if current[field] != active[field]:
                    raise Refusal("new campaign must bind exactly to the exported accepted parent")
            previous_tasks = set(previous.value("used-confirmation-sources", []))
            previous_tasks.update(t["base_source"] for t in previous.load(previous.value("confirmation-protocol"))["tasks"])
            new_tasks = {t["base_source"] for name in ("development", "confirmation")
                         for t in store.load(store.value(name + "-protocol"))["tasks"]}
            if new_tasks & previous_tasks:
                raise Refusal("previous confirmation tasks cannot be reused for development or confirmation")
            # Evidence is copied by verified content identity, never by a mutable
            # path dependency. Only records reachable from accepted ancestry are
            # required for Undo; do not duplicate old trial archives or graders.
            chain, key = {}, active_key
            while key:
                entry = lineage[key]
                chain[key] = {"parent": entry["parent"], "state": "active" if key == active_key else "ancestor",
                              "inherited_from": previous.head()}
                record = previous.load(key)
                store.put(previous.get(key))
                store.put(previous.get(record["binary"]))
                source = previous.load(record["source"])
                store.put(previous.get(record["source"]))
                for item in source["entries"]:
                    if item["kind"] == "file": store.put(previous.get(item["digest"]))
                if record["proposal"]: store.put(previous.get(record["proposal"]))
                key = entry["parent"]
            store.set("lineage", chain)
            store.set("active", active_key)
            store.set("baseline", active_key)
            store.set("used-confirmation-sources", sorted(previous_tasks))
            store.append("campaign-forked", {"parent_campaign_head": previous.head(), "active": active_key,
                                           "fresh_confirmation": True, "production_installed": False})
            return status(store)
