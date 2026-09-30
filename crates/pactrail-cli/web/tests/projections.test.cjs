const { test } = require("node:test");
const assert = require("node:assert/strict");
const {
  formatUsage,
  pricingToEngine,
  parseUnifiedDiff,
  freshness,
  provenance,
  usageProjection,
  evidenceSummary,
} = require("../app.js");
test("usage preserves absent, zero and partial reporting", () => {
  for (const n of [null, undefined, "", NaN, Infinity, -1])
    assert.equal(formatUsage(n).text, "—");
  assert.equal(formatUsage(0).text, "0");
  assert.equal(formatUsage(0, { kind: "cost" }).text, "$0.0000");
  assert.equal(
    formatUsage(12480, { coverage: { reported: 6, total: 7 } }).text,
    "≥ 12,480",
  );
  assert.equal(formatUsage(214600, { kind: "tokens" }).text, "214.6k");
  assert.equal(formatUsage(221, { kind: "duration" }).text, "3:41");
});
test("pricing conversion preserves zero and requires complete cards for a cap", () => {
  assert.deepEqual(
    pricingToEngine(
      { input: 0, cached_input: 0, cache_creation: 0, output: 0 },
      "0.01",
    ),
    {
      input_price: 0,
      cached_input_price: 0,
      cache_creation_price: 0,
      output_price: 0,
      max_cost_microusd: 10000,
    },
  );
  assert.equal(
    pricingToEngine({
      input: 0.123456,
      cached_input: 0.01,
      cache_creation: 1,
      output: 2,
    }).input_price,
    123456,
  );
  assert.throws(() => pricingToEngine({ input: 0 }), /all four/);
  assert.throws(() => pricingToEngine(null, "1"), /all four/);
  assert.deepEqual(pricingToEngine(null), {});
});
const event = (sequence, actor, observed_effects = [], attributes = {}) => ({
  sequence,
  event: {
    type: "action_completed",
    data: {
      actor,
      action: actor.startsWith("model:") ? "invoke" : "edit",
      observed_effects,
      attributes,
    },
  },
});
test("freshness follows event order and does not claim missing evidence is fresh", () => {
  const write = event(198, "tool:edit_file", ["fs.changed:src/😀.rs"]);
  const check = {
    sequence: 150,
    event: { type: "evidence_recorded", data: {} },
  };
  assert.equal(freshness([check, write]).stale, true);
  assert.equal(freshness([write, { ...check, sequence: 200 }]).stale, false);
  assert.deepEqual(freshness([write]), {
    stale: false,
    lastWrite: 198,
    lastEvidence: null,
  });
});
test("provenance uses exact observed paths and recorded turns", () => {
  const events = [
    event(1, "model:p/m", [], { turn: "2" }),
    event(2, "tool:edit_file", ["fs.changed:src/😀.rs"]),
    event(3, "tool:read_file", ["fs.read:src/😀.rs"]),
    event(4, "model:p/m", [], { turn: "4" }),
    event(5, "tool:write_file", ["fs.write:src/😀.rs"]),
  ];
  assert.deepEqual(provenance(events, "src/😀.rs"), {
    actions: 2,
    turns: [2, 4],
    sequences: [2, 5],
  });
  assert.equal(provenance(events, "src/other.rs").actions, 0);
});
test("usage sums available fields without hiding incomplete turns", () => {
  const data = [
    event(1, "model:p/m", [], { input_tokens: "0", output_tokens: "0" }),
    event(2, "model:p/m", [], { input_tokens: "12" }),
    event(3, "model:p/m"),
  ];
  const u = usageProjection(data);
  assert.equal(u.total, 12);
  assert.equal(u.reported, 1);
  assert.equal(u.turns, 3);
  assert.equal(
    formatUsage(u.total, { coverage: { reported: u.reported, total: u.turns } })
      .state,
    "partial",
  );
  assert.equal(usageProjection([event(1, "model:p/m")]).total, null);
});
test("diff parsing handles hunks, Unicode, deleted and mode-only files", () => {
  const text =
    "--- a/src/😀.rs\n+++ b/src/😀.rs\n@@ -2,2 +2,2 @@\n context\n-old\n+new\n\\ No newline at end of file\n";
  const changes = [
    {
      path: "src/😀.rs",
      before_digest: "a",
      after_digest: "b",
      before_unix_mode: 420,
      after_unix_mode: 420,
    },
    {
      path: "mode",
      before_digest: "a",
      after_digest: "a",
      before_unix_mode: 420,
      after_unix_mode: 493,
    },
  ];
  const f = parseUnifiedDiff(text, changes);
  assert.equal(f[0].added, 1);
  assert.equal(f[0].removed, 1);
  assert.equal(f[0].hunks[0].lines[2].new, 3);
  assert.equal(f[1].modeChanged, true);
  assert.equal(f[1].hunks.length, 0);
  const deletion = parseUnifiedDiff(
    "--- a/gone\n+++ /dev/null\n@@ -1 +0,0 @@\n-gone\n",
    [{ path: "gone", before_digest: "x", after_digest: null }],
  );
  assert.equal(deletion[0].status, "D");
  assert.equal(deletion[0].path, "gone");
});
test("only deterministic passing evidence supplies the strongest apply signal", () => {
  assert.equal(
    evidenceSummary([{ grade: "model_assessed", status: "passed" }])
      .deterministicPass,
    false,
  );
  assert.equal(
    evidenceSummary([{ grade: "deterministic", status: "passed" }])
      .deterministicPass,
    true,
  );
});

test("diff content that resembles file headers remains a hunk line", () => {
  const files = parseUnifiedDiff(
    "--- a/test\n+++ b/test\n@@ -1 +1 @@\n--- old heading\n+++ new heading\n",
  );
  assert.equal(files.length, 1);
  assert.equal(files[0].removed, 1);
  assert.equal(files[0].added, 1);
  assert.equal(files[0].hunks[0].lines[0].text, "-- old heading");
});
