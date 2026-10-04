/* Development-only visual/interaction checks. Requires Playwright outside the app.
   NODE_PATH=/path/to/playwright/node_modules node web/tests/browser-check.cjs
   Uses explicit synthetic fixtures; engine workflows are checked separately. */
const { chromium } = require("playwright");
const fs = require("node:fs");
const path = require("node:path");
const http = require("node:http");
const assert = require("node:assert/strict");
const root = path.resolve(__dirname, ".."),
  out = process.env.PACTRAIL_UI_ARTIFACTS || "/tmp/pactrail-ledger-qa";
fs.mkdirSync(out, { recursive: true });
const id = "019f6db5-0000-7000-8000-000000000001",
  workspace = "/tmp/ledger-fixture/workspace";
const now = Date.now();
const contract = {
  goal: "Repair the parser without changing its public API.",
  workspace_root: workspace,
  allowed_write_paths: ["src/"],
  out_of_scope: ["Generated assets"],
  obligations: [
    {
      id: "o1",
      description: "The parser handles empty input without panic.",
      kind: "functional",
      required: true,
    },
    {
      id: "o2",
      description: "Existing regression tests still pass.",
      kind: "regression",
      required: true,
    },
  ],
  budget: {
    wall_time_seconds: 3600,
    model_tokens: 250000,
    cost_microusd: 1000000,
    max_concurrency: 4,
    max_model_attempts: 24,
  },
};
const evidence = [
  {
    id: "e1",
    obligation_id: "o1",
    grade: "unverified",
    kind: "test",
    status: "inconclusive",
    summary: "Verification commands require process permission",
    artifact_digest: null,
    reproduction: null,
  },
  {
    id: "e2",
    obligation_id: "o2",
    grade: "deterministic",
    kind: "test",
    status: "passed",
    summary: "The repository regression suite passed.",
    artifact_digest: "a".repeat(64),
    reproduction: "cargo test --offline",
  },
];
const changes = [
  {
    path: "src/parser.rs",
    before_digest: "1".repeat(64),
    after_digest: "2".repeat(64),
    bytes_added: 42,
    bytes_removed: 7,
    before_unix_mode: 420,
    after_unix_mode: 420,
  },
];
const diff =
  "--- a/src/parser.rs\n+++ b/src/parser.rs\n@@ -1,3 +1,4 @@\n fn parse(input: &str) {\n-    consume(input);\n+    if input.is_empty() { return; }\n+    consume(input.trim());\n }\n";
function env(sequence, type, data) {
  return {
    schema_version: 1,
    run_id: id,
    sequence,
    timestamp: new Date(now + sequence * 1000).toISOString(),
    previous_hash: "0".repeat(64),
    hash: "a".repeat(64),
    event: { type, data },
  };
}
function events(state = "awaiting_apply", count = 10) {
  let e = [
    env(0, "contract_registered", contract),
    env(1, "state_changed", { from: "created", to: "contracting" }),
    env(2, "state_changed", { from: "contracting", to: "investigating" }),
  ];
  for (let n = 0; n < count; n++) {
    e.push(
      env(e.length, "action_completed", {
        actor: "model:fixture/test-model",
        action: "invoke",
        summary: "Model turn completed",
        succeeded: true,
        duration_ms: 1400,
        attributes: {
          turn: String(n + 1),
          ...(n % 3
            ? {
                input_tokens: "2400",
                output_tokens: "80",
                cached_input_tokens: "0",
              }
            : {}),
        },
        observed_effects: [],
      }),
    );
    e.push(
      env(e.length, "action_completed", {
        actor: "tool:read_file",
        action: "read_file",
        summary: "Read current parser source " + n,
        succeeded: true,
        duration_ms: n ? 12 : 0,
        attributes: {
          call_id: "call" + n,
          risk: "readonly",
          output_bytes: "480",
          output_truncated: "false",
        },
        observed_effects: ["fs.read:src/parser.rs"],
      }),
    );
  }
  e.push(
    env(e.length, "state_changed", { from: "investigating", to: "executing" }),
    env(e.length + 1, "action_completed", {
      actor: "tool:edit_file",
      action: "edit_file",
      summary: "Updated the parser’s empty-input guard.",
      succeeded: true,
      duration_ms: 9,
      attributes: { risk: "workspacemutation" },
      observed_effects: ["fs.changed:src/parser.rs"],
    }),
  );
  for (const record of evidence)
    e.push(env(e.length, "evidence_recorded", record));
  if (state !== "executing")
    e.push(env(e.length, "state_changed", { from: "executing", to: state }));
  return e;
}
let fixture = {
  state: "awaiting_apply",
  receipt: true,
  events: events(),
  changes,
  diff,
  jobs: [],
  runs: true,
  cost: 0,
};
function run() {
  return {
    run_id: id,
    state: fixture.state,
    outcome:
      {
        awaiting_apply: "ready_to_apply",
        completed: "answered",
        cancelled: "cancelled",
      }[fixture.state] || fixture.state,
    goal: contract.goal,
    changes: fixture.changes.length,
    created_at: new Date(now).toISOString(),
  };
}
function receipt() {
  return {
    schema_version: 1,
    run_id: id,
    created_at: new Date(now).toISOString(),
    contract,
    outcome: run().outcome,
    changes: fixture.changes,
    evidence,
    approvals: [],
    verification: {
      passed: 1,
      failed: 0,
      inconclusive: 1,
      skipped: 0,
      highest_grade: "deterministic",
    },
    unresolved_risks: [
      "The requested behavior is not covered by a task-specific acceptance check.",
    ],
    baseline_digest: "b".repeat(64),
    final_event_hash: "c".repeat(64),
    integrity_hash: "d".repeat(64),
    metadata: {},
    web_result: {
      summary:
        "# Repository overview\n\nThe parser reads **tokens** and returns a syntax tree.\n\n- Entry point: `src/parser.rs`\n- No source files were changed.\n\n```rust\nparse(input)\n```",
      tokens: null,
      cost_microusd: fixture.cost,
    },
  };
}
const server = http.createServer((req, res) => {
  const url = new URL(req.url, "http://localhost");
  res.setHeader(
    "Content-Security-Policy",
    "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; font-src 'self'",
  );
  const send = (x) => {
    res.setHeader("Content-Type", "application/json");
    res.end(JSON.stringify(x));
  };
  if (url.pathname === "/api/bootstrap")
    return send({
      workspace,
      version: "fixture",
      providers: ["open-ai-compatible"],
      defaults: {
        provider: "open-ai-compatible",
        model: "fixture/test-model",
        base_url: "http://127.0.0.1:1234/v1",
        api_key_env: "FIXTURE_KEY",
      },
    });
  if (url.pathname === "/api/runs") return send(fixture.runs ? [run()] : []);
  if (url.pathname === "/api/jobs") return send(fixture.jobs);
  if (url.pathname.endsWith("/trace")) return send(fixture.events);
  if (url.pathname.endsWith("/inspect"))
    return send(
      fixture.receipt
        ? receipt()
        : {
            run_id: id,
            state: fixture.state,
            events: fixture.events.length,
            receipt: null,
          },
    );
  if (url.pathname.endsWith("/diff"))
    return send({
      unified_diff: fixture.diff,
      changes: fixture.changes,
      receipt_integrity: "verified",
      outcome: run().outcome,
    });
  if (url.pathname.endsWith("/events")) {
    res.setHeader("Content-Type", "text/event-stream");
    res.write("event: trace\ndata: " + JSON.stringify(fixture.events) + "\n\n");
    const interval = setInterval(
      () =>
        res.write(
          "event: trace\ndata: " + JSON.stringify(fixture.events) + "\n\n",
        ),
      800,
    );
    req.on("close", () => clearInterval(interval));
    return;
  }
  let file =
    url.pathname === "/" ||
    url.pathname === "/settings" ||
    url.pathname.startsWith("/runs/")
      ? "index.html"
      : url.pathname.slice(1);
  if (!/^[\w./-]+$/.test(file) || file.includes("..")) {
    res.statusCode = 404;
    return res.end();
  }
  const target = path.join(root, file);
  if (!fs.existsSync(target)) {
    res.statusCode = 404;
    return res.end();
  }
  res.setHeader(
    "Content-Type",
    file.endsWith(".js")
      ? "text/javascript"
      : file.endsWith(".css")
        ? "text/css"
        : file.endsWith(".svg")
          ? "image/svg+xml"
          : file.endsWith(".woff2")
            ? "font/woff2"
            : "text/html",
  );
  res.end(fs.readFileSync(target));
});
(async () => {
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const base = "http://127.0.0.1:" + server.address().port;
  const launch = { headless: true };
  if (process.env.PACTRAIL_CHROMIUM)
    launch.executablePath = process.env.PACTRAIL_CHROMIUM;
  const browser = await chromium.launch(launch);
  const errors = [];
  const page = await browser.newPage({
    viewport: { width: 1440, height: 1000 },
  });
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error" && /Content Security Policy/.test(m.text()))
      errors.push(m.text());
  });
  const goto = async (route) => {
    await page.goto(base + route);
    await page.waitForTimeout(250);
  };
  const shot = async (name) =>
    page.screenshot({ path: path.join(out, name + ".png") });
  await goto("/");
  await page
    .getByLabel("What should change?", { exact: true })
    .fill("Preserve this draft.");
  await page.reload();
  await page.waitForTimeout(500);
  assert.equal(
    await page.getByLabel("What should change?", { exact: true }).inputValue(),
    "Preserve this draft.",
  );
  // The setup disclosure must not remount the editor or forget its draft.
  const editor = await page.locator("#task-goal").elementHandle();
  await page.locator(".setup-summary").click();
  assert.equal(await page.locator(".run-setup").evaluate((e) => e.open), false);
  assert.equal(
    await editor.evaluate(
      (e) => e.isConnected && e.value === "Preserve this draft.",
    ),
    true,
  );
  await page.locator(".setup-summary").click();
  assert.equal(
    await page.locator("#task-goal").inputValue(),
    "Preserve this draft.",
  );
  await page.getByRole("radio", { name: "Host", exact: true }).click();
  assert.equal(await page.locator("#dispatch-button").isDisabled(), true);
  await page.getByLabel("I allow this run").check();
  assert.equal(await page.locator("#dispatch-button").isDisabled(), false);
  await page.reload();
  await page.waitForTimeout(500);
  assert.equal(await page.getByLabel("I allow this run").isChecked(), false);
  await goto("/runs/" + id + "?view=changes");
  await page.setViewportSize({ width: 1920, height: 1000 });
  await page.locator("#diff-mode").click();
  assert.ok(/[?&]mode=(split|unified)/.test(page.url()));
  const layout = new URL(page.url()).searchParams.get("mode");
  await page.reload();
  await page.waitForTimeout(350);
  assert.equal(new URL(page.url()).searchParams.get("mode"), layout);
  await page.getByLabel("Viewed", { exact: true }).check();
  await page.reload();
  await page.waitForTimeout(350);
  assert.equal(
    await page.getByLabel("Viewed", { exact: true }).isChecked(),
    true,
  );
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.getByRole("button", { name: "Apply 1 file", exact: true }).click();
  assert.equal(await page.locator("#dialog").isVisible(), true);
  assert.equal(
    await page.evaluate(() => document.activeElement.textContent),
    "Cancel",
  );
  assert.equal(
    await page
      .locator("#dialog")
      .getByRole("button", { name: "Apply 1 file" })
      .isEnabled(),
    true,
  );
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await page.getByRole("tab", { name: "Trace", exact: true }).click();
  await page
    .locator(".phase-heading")
    .filter({ hasText: "Investigate" })
    .click();
  await page
    .getByRole("button", { name: /read_file Read current parser source 0/ })
    .click();
  const expanded = page
    .locator('.trace-row-head[aria-expanded="true"]')
    .first();
  await page.getByRole("searchbox", { name: "Search trace" }).fill("parser");
  await page.waitForTimeout(1700);
  assert.equal(
    await page.locator('.trace-row-head[aria-expanded="true"]').count(),
    1,
  );
  assert.equal(
    await page.getByRole("searchbox", { name: "Search trace" }).inputValue(),
    "parser",
  );
  const largePath = "src/" + "long-path-".repeat(20) + "😀.rs";
  fixture.changes = [
    ...changes,
    {
      path: largePath,
      before_digest: "x",
      after_digest: "y",
      before_unix_mode: 420,
      after_unix_mode: 493,
      bytes_added: 5000,
      bytes_removed: 1,
    },
  ];
  fixture.diff =
    diff +
    "\n--- a/" +
    largePath +
    "\n+++ b/" +
    largePath +
    "\n@@ -1 +1,5000 @@\n-old\n" +
    Array.from(
      { length: 5000 },
      (_, i) => "+" + (i === 0 ? "x".repeat(600) : "line " + i),
    ).join("\n") +
    "\n";
  for (const width of [360, 414, 768, 1024, 1280, 1440, 1920, 2560]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const route of [
      "/",
      "/runs/" + id + "?view=trace",
      "/runs/" + id + "?view=changes",
      "/settings",
    ]) {
      await goto(route);
      assert.ok(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
        `Page overflow ${width} ${route}`,
      );
    }
    await shot("responsive-" + width);
  }
  fixture.changes = Array.from({ length: 32 }, (_, i) => ({
    ...changes[0],
    path: `src/group${i % 4}/file${i}.rs`,
  }));
  fixture.diff = fixture.changes
    .map((c) => `--- a/${c.path}\n+++ b/${c.path}\n@@ -1 +1 @@\n-old\n+new\n`)
    .join("");
  await page.setViewportSize({ width: 1440, height: 1000 });
  await goto("/runs/" + id + "?view=changes");
  assert.equal(await page.locator(".file-row").count(), 32);
  await shot("32-files");
  const touch = await browser.newPage({
    viewport: { width: 320, height: 740 },
    isMobile: true,
    hasTouch: true,
  });
  for (const route of ["/", "/runs/" + id + "?view=changes", "/settings"]) {
    await touch.goto(base + route);
    await touch.waitForTimeout(300);
    assert.ok(
      await touch.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
      `320px overflow: ${route}`,
    );
    const small = await touch
      .locator("button:visible:not(:disabled)")
      .evaluateAll((nodes) =>
        nodes
          .filter((n) => {
            const r = n.getBoundingClientRect();
            return r.width < 43.5 || r.height < 43.5;
          })
          .map((n) => n.textContent || n.getAttribute("aria-label")),
      );
    assert.deepEqual(small, [], `Coarse pointer targets: ${route}`);
  }
  await touch.screenshot({ path: path.join(out, "coarse-320.png") });
  await touch.close();
  await page.setViewportSize({ width: 720, height: 500 });
  await goto("/runs/" + id + "?view=changes");
  assert.ok(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
    "200% effective viewport overflow",
  );
  fixture.changes = changes;
  fixture.diff = diff;
  const cases = [
    [
      "S1",
      "/",
      () => {
        fixture.runs = false;
      },
    ],
    [
      "S2",
      "/",
      () => {
        fixture.runs = true;
      },
    ],
    [
      "S3",
      "/runs/starting?job=start",
      () => {
        fixture.jobs = [
          { id: "start", state: "running", goal: "Starting fixture" },
        ];
      },
    ],
    [
      "S4",
      "/runs/" + id + "?view=trace",
      () => {
        fixture.state = "executing";
        fixture.receipt = false;
        fixture.events = events("executing");
        fixture.jobs = [{ id: "job", state: "running" }];
      },
    ],
    [
      "S5",
      "/runs/" + id + "?view=trace",
      () => {
        fixture.jobs = [{ id: "job", state: "cancelling" }];
      },
    ],
    [
      "S6",
      "/runs/" + id + "?view=changes",
      () => {
        fixture.state = "cancelled";
        fixture.receipt = true;
        fixture.events = events("cancelled");
        fixture.jobs = [];
      },
    ],
    [
      "S7",
      "/runs/" + id + "?view=answer",
      () => {
        fixture.state = "completed";
        fixture.events = events("completed");
        fixture.changes = [];
      },
    ],
    [
      "S8",
      "/runs/" + id + "?view=changes",
      () => {
        fixture.state = "awaiting_apply";
        fixture.events = events();
        fixture.changes = changes;
      },
    ],
    ["S9b", "/runs/" + id + "?view=evidence", () => {}],
    ["S9c", "/runs/" + id + "?view=receipt", () => {}],
    ["S10", "/runs/" + id + "?view=changes", () => {}],
    [
      "S11",
      "/runs/" + id + "?view=changes",
      () => {
        fixture.state = "applied";
        fixture.events = events("applied");
      },
    ],
    [
      "S11-discarded",
      "/runs/" + id + "?view=changes",
      () => {
        fixture.state = "discarded";
        fixture.events = events("discarded");
      },
    ],
    [
      "S12",
      "/runs/" + id + "?view=trace",
      () => {
        fixture.state = "failed";
        fixture.events = events("failed");
        fixture.jobs = [
          {
            id: "job",
            state: "failed",
            output: { run_id: id },
            error: "Fixture: provider response was incomplete.",
          },
        ];
      },
    ],
    ["S13", "/settings", () => {}],
    ["S14", "/runs/" + id + "?view=trace", () => {}],
  ];
  for (const theme of ["light", "dark"])
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: 1000 });
      await page.addInitScript(
        (theme) =>
          localStorage.setItem("pactrail.theme", JSON.stringify(theme)),
        theme,
      );
      fixture = {
        state: "awaiting_apply",
        receipt: true,
        events: events(),
        changes,
        diff,
        jobs: [],
        runs: true,
        cost: 0,
      };
      for (const [name, route, setup] of cases) {
        setup();
        await goto(route);
        if (name === "S10")
          await page
            .getByRole("button", { name: "Apply 1 file", exact: true })
            .click();
        if (name === "S14") {
          await page.route("**/api/**", (r) => r.abort());
          await page.evaluate(() => window.dispatchEvent(new Event("focus")));
          await page.waitForTimeout(250);
        }
        await shot(`${name}-${theme}-${width}`);
        if (name === "S14") await page.unroute("**/api/**");
      }
    }
  fixture = {
    state: "executing",
    receipt: false,
    events: events("executing", 1000),
    changes,
    diff,
    jobs: [{ id: "job", state: "running" }],
    runs: true,
    cost: null,
  };
  await page.setViewportSize({ width: 1440, height: 1000 });
  await goto("/runs/" + id + "?view=trace");
  await page.waitForTimeout(700);
  const count = await page.locator(".trace-row").count();
  assert.ok(count >= 2000);
  await page.getByRole("searchbox", { name: "Search trace" }).fill("parser");
  assert.ok((await page.locator("mark").count()) > 0);
  await page.getByRole("searchbox", { name: "Search trace" }).fill("");
  const scroller = page.locator(".trace-scroller");
  await scroller.evaluate((e) => {
    e.scrollTop = 100;
    e.dispatchEvent(new WheelEvent("wheel", { deltaY: -10 }));
  });
  const scrollBefore = await scroller.evaluate((e) => e.scrollTop);
  fixture.events.push(
    env(fixture.events.length, "note_recorded", {
      message: "New fixture event",
    }),
  );
  await page.waitForTimeout(1700);
  assert.equal(await scroller.evaluate((e) => e.scrollTop), scrollBefore);
  assert.equal(await page.locator(".trace-row").count(), count + 1);
  await page.getByRole("searchbox", { name: "Search trace" }).fill("parser");
  const edit = page
    .locator(".trace-row-head")
    .filter({ hasText: "Updated the parser" });
  await edit.click();
  await scroller.evaluate((e) => {
    e.scrollTop = 100;
    e.dispatchEvent(new WheelEvent("wheel", { deltaY: -10 }));
  });
  await page.evaluate(() => {
    window.traceBatchMeasurements = [];
    window.addEventListener("pactrail:trace-batch", (e) =>
      window.traceBatchMeasurements.push(e.detail),
    );
    const text = [...document.querySelectorAll(".trace-row-summary")].find(
      (e) => e.textContent.includes("Updated the parser"),
    );
    const range = document.createRange();
    range.selectNodeContents(text);
    const selection = getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
  });
  const selectionBefore = await page.evaluate(() => getSelection().toString());
  const reconnectScroll = await scroller.evaluate((e) => e.scrollTop);
  await page.route("**/api/**", (r) => r.abort());
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await page
    .locator("#connection-banner")
    .filter({ hasText: "Lost connection" })
    .waitFor();
  await page.unroute("**/api/**");
  for (let n = 0; n < 50; n++)
    fixture.events.push(
      env(fixture.events.length, "note_recorded", {
        message: "parser fixture batch " + n,
      }),
    );
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await page
    .locator("#connection-banner")
    .filter({ hasText: "Reconnected" })
    .waitFor();
  await page.waitForTimeout(1800);
  assert.equal(
    await page.getByRole("searchbox", { name: "Search trace" }).inputValue(),
    "parser",
  );
  assert.equal(
    await page.evaluate(() => getSelection().toString()),
    selectionBefore,
  );
  assert.equal(
    await page.locator('.trace-row-head[aria-expanded="true"]').count(),
    1,
  );
  assert.equal(await scroller.evaluate((e) => e.scrollTop), reconnectScroll);
  assert.equal(await page.locator(".trace-row").count(), count + 51);
  const batches = await page.evaluate(() => window.traceBatchMeasurements);
  assert.ok(batches.some((b) => b.count === 50));
  assert.ok(
    batches.every((b) => b.durationMs < 50),
    JSON.stringify(batches),
  );
  await shot("reconnected-trace");
  await shot("large-trace");
  assert.deepEqual(errors, []);
  console.log(
    JSON.stringify({
      screenshots: fs.readdirSync(out).filter((f) => f.endsWith(".png")).length,
      errors,
      largeTraceRows: count,
      traceBatches: batches,
      output: out,
    }),
  );
  await browser.close();
  server.close();
})().catch((e) => {
  console.error(e);
  server.close();
  process.exit(1);
});
