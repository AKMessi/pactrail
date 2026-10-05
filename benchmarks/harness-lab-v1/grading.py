"""Fail-closed interpretation of externally parsed, explicitly declared tests.

No test execution or parser emulation lives here. The upstream parser and its
input log must be separately pinned by the execution runner. This module keeps
shell exit status from masquerading as measured behavioral correctness.
"""

KNOWN_STATUSES = frozenset(("PASSED", "FAILED", "SKIPPED", "ERROR"))
MAX_TESTS = 100_000


def declared_tests(value):
    if not isinstance(value, list) or not 1 <= len(value) <= MAX_TESTS:
        raise ValueError("a bounded, nonempty declared test list is required")
    if any(not isinstance(name, str) or not name or len(name) > 8192 for name in value):
        raise ValueError("invalid declared test identity")
    if len(set(value)) != len(value):
        raise ValueError("duplicate declared test identity")
    return value


def summarize(required, statuses, *, timed_out=False, infrastructure_error=False):
    """Missing/skipped/error outcomes never qualify; unavailable is not zero.

    FAILED means an executed assertion failure supplied by the external parser.
    ERROR means the test did not establish the intended behavioral result.
    Extra tests remain in retained parser output but cannot satisfy requirements.
    """
    required = declared_tests(required)
    if type(timed_out) is not bool or type(infrastructure_error) is not bool:
        raise ValueError("execution flags must be booleans")
    if not isinstance(statuses, dict) or len(statuses) > MAX_TESTS:
        raise ValueError("bounded parsed test status map required")
    if any(not isinstance(name, str) or not name or len(name) > 8192
           or not isinstance(status, str) or status not in KNOWN_STATUSES
           for name, status in statuses.items()):
        raise ValueError("invalid parsed test status")
    groups = {status: [] for status in KNOWN_STATUSES}
    missing = []
    for name in required:
        if name not in statuses:
            missing.append(name)
        else:
            groups[statuses[name]].append(name)
    passed = groups["PASSED"]
    failed = groups["FAILED"]
    complete = not (missing or groups["SKIPPED"] or groups["ERROR"]
                    or timed_out or infrastructure_error)
    return {"required_tests": len(required),
            "tests_executed": len(passed) + len(failed),
            "tests_passed": len(passed), "tests_failed": len(failed),
            "missing": missing, "skipped": groups["SKIPPED"],
            "errors": groups["ERROR"], "timed_out": timed_out,
            "infrastructure_error": infrastructure_error,
            "coverage_complete": complete,
            "behavior_passed": complete and len(passed) == len(required),
            "behavior_failed": complete and bool(failed)}
