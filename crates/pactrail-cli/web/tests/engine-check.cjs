/* Real engine, disposable workspaces, deterministic local model. No API credentials.
   Run model-fixture.py on 4190, then this script with Playwright in NODE_PATH. */
const { chromium } = require("playwright");
const { spawn, execFileSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const assert = require("node:assert/strict");
const repo = path.resolve(__dirname, "../../../.."),
  assets = path.resolve(__dirname, ".."),
  out = "/tmp/pactrail-ledger-engine-qa";
fs.mkdirSync(out, { recursive: true });
const children = new Set();
process.on("exit", () => {
  for (const pid of children) {
    try {
      process.kill(-pid, "SIGKILL");
    } catch {}
  }
});
const binary =
  process.env.PACTRAIL_BINARY || path.join(repo, "target/debug/pactrail");
(async () => {
  const browser = await chromium.launch({
    headless: true,
    ...(process.env.PACTRAIL_CHROMIUM
      ? { executablePath: process.env.PACTRAIL_CHROMIUM }
      : {}),
  });
  const results = [];
  let port = 4200;
  async function setup(model) {
    const workspace = fs.mkdtempSync("/tmp/pactrail-ledger-" + model + "-");
    fs.writeFileSync(
      path.join(workspace, "README.md"),
      "# Disposable Ledger Rail test\n",
    );
    if (model === "fixture-verified") {
      fs.mkdirSync(path.join(workspace, "src"));
      fs.writeFileSync(
        path.join(workspace, "Cargo.toml"),
        '[package]\nname="ledger-test"\nversion="0.1.0"\nedition="2024"\n',
      );
      fs.writeFileSync(
        path.join(workspace, "src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a - b }\n",
      );
    }
    const p = port++,
      server = spawn(
        binary,
        ["--workspace", workspace, "web", "--port", String(p)],
        { stdio: ["ignore", "pipe", "pipe"], detached: true },
      );
    children.add(server.pid);
    let logs = "";
    server.stderr.on("data", (b) => (logs += b));
    await new Promise((resolve, reject) => {
      const timer = setTimeout(
        () => reject(Error("Server did not start: " + logs)),
        10000,
      );
      server.stdout.once("data", () => {
        clearTimeout(timer);
        resolve();
      });
      server.once("exit", (code) =>
        reject(Error("Server exited " + code + ": " + logs)),
      );
    });
    const context = await browser.newContext({
      viewport: { width: 1440, height: 1000 },
    });
    await context.addInitScript(
      (config) =>
        localStorage.setItem("pactrail.config", JSON.stringify(config)),
      {
        provider: "open-ai-compatible",
        model,
        base_url: "http://127.0.0.1:4190/v1",
        api_key_env: "",
        process_backend: "disabled",
      },
    );
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    for (const file of ["app.js", "app.css", "boot.js"])
      await page.route("**/" + file, (r) =>
        r.fulfill({
          status: 200,
          contentType: file.endsWith(".css") ? "text/css" : "text/javascript",
          body: fs.readFileSync(path.join(assets, file), "utf8"),
        }),
      );
    await page.goto("http://127.0.0.1:" + p + "/");
    await page.getByLabel("What should change?", { exact: true }).waitFor();
    await page.waitForTimeout(200);
    return {
      workspace,
      server,
      page,
      context,
      errors,
      url: "http://127.0.0.1:" + p,
    };
  }
  async function dispatch(t, goal, host = false) {
    await t.page.getByLabel("What should change?", { exact: true }).fill(goal);
    if (host) {
      await t.page.getByRole("radio", { name: "Host", exact: true }).click();
      await t.page.getByLabel("I allow this run").check();
    }
    await t.page.locator("#dispatch-button").click();
    await t.page.waitForURL(/\/runs\//, { timeout: 15000 });
  }
  async function finish(t, label) {
    await t.page
      .locator(".run-heading .state-chip")
      .filter({ hasText: label })
      .waitFor({ timeout: 40000 });
    await t.page.waitForTimeout(300);
    assert.deepEqual(t.errors, []);
  }
  async function close(t) {
    t.server.kill("SIGINT");
    await t.context.close();
  }
  let t = await setup("fixture-read");
  await dispatch(t, "Explain this repository. Do not modify files.");
  await finish(t, "Answered");
  await t.page.getByRole("tab", { name: "Answer", exact: true }).click();
  assert.ok(
    (await t.page.locator(".answer-prose").innerText()).includes("disposable"),
  );
  await t.page.screenshot({ path: out + "/answered.png" });
  results.push("read-only dispatch → Starting → Trace → Answer");
  await close(t);
  t = await setup("fixture-edit");
  await dispatch(t, "Create candidate.md containing a short test heading.");
  await finish(t, "Awaiting review");
  await t.page.getByRole("tab", { name: /Changes/ }).click();
  assert.equal(fs.existsSync(path.join(t.workspace, "candidate.md")), false);
  await t.page
    .getByRole("button", { name: "Apply 1 file", exact: true })
    .click();
  const primary = t.page
    .locator("#dialog")
    .getByRole("button", { name: "Apply 1 file" });
  assert.equal(await primary.isDisabled(), true);
  await t.page.getByLabel("I understand this candidate is not backed").check();
  await primary.click();
  await t.page
    .locator(".run-banners h2")
    .filter({ hasText: "Applied 1 file" })
    .waitFor({ timeout: 15000 });
  assert.equal(
    fs.readFileSync(path.join(t.workspace, "candidate.md"), "utf8"),
    "# Test candidate\n",
  );
  await t.page.screenshot({ path: out + "/applied.png" });
  results.push(
    "isolated edit → unverified acknowledgment → Apply → persistent result",
  );
  await close(t);
  t = await setup("fixture-edit");
  await dispatch(t, "Create candidate.md containing a short test heading.");
  await finish(t, "Awaiting review");
  await t.page.getByRole("tab", { name: /Changes/ }).click();
  await t.page.getByRole("button", { name: "Discard", exact: true }).click();
  await t.page
    .getByLabel("Note (optional)", { exact: false })
    .fill("Rejected in UI test.");
  await t.page
    .locator("#dialog")
    .getByRole("button", { name: "Discard candidate", exact: true })
    .click();
  await t.page
    .locator(".run-banners h2")
    .filter({ hasText: "Candidate discarded" })
    .waitFor({ timeout: 15000 });
  assert.equal(fs.existsSync(path.join(t.workspace, "candidate.md")), false);
  await t.page.reload();
  await t.page
    .getByText("Your note (this browser only): Rejected in UI test.")
    .waitFor();
  await t.page.screenshot({ path: out + "/discarded.png" });
  results.push(
    "Discard dialog → workspace untouched → persistent browser-local note",
  );
  await close(t);
  t = await setup("fixture-verified");
  await dispatch(
    t,
    "Fix addition in src/lib.rs so add(2,3) returns 5. Preserve a regression test.",
    true,
  );
  await finish(t, "Awaiting review");
  await t.page.getByRole("tab", { name: /Changes/ }).click();
  await t.page
    .getByRole("button", { name: "Apply 1 file", exact: true })
    .click();
  const apply = t.page
    .locator("#dialog")
    .getByRole("button", { name: "Apply 1 file" });
  const receipt = await t.page.request.get(
    t.url +
      "/api/runs/" +
      t.page.url().match(/\/runs\/([^?]+)/)[1] +
      "/inspect",
  );
  const r = await receipt.json();
  const deterministic = r.evidence.some(
    (e) => e.grade === "deterministic" && e.status === "passed",
  );
  if (deterministic) assert.equal(await apply.isEnabled(), true);
  else {
    await t.page
      .getByLabel("I understand this candidate is not backed")
      .check();
  }
  await t.page.screenshot({ path: out + "/verified-apply.png" });
  await apply.click();
  await t.page
    .locator(".run-banners h2")
    .filter({ hasText: "Applied 1 file" })
    .waitFor({ timeout: 15000 });
  results.push(
    "host acknowledgment → real Cargo verification → Apply (deterministic pass: " +
      deterministic +
      ")",
  );
  await close(t);
  t = await setup("fixture-stop");
  await dispatch(t, "Explain this repository without modifying files.");
  await t.page.locator(".run-heading").waitFor({ timeout: 15000 });
  await t.page.locator(".new-run").click();
  await t.page.locator(".active-run-banner").waitFor();
  await t.page.getByRole("button", { name: "Open active run" }).click();
  await t.page.getByRole("button", { name: "Stop", exact: true }).click();
  await t.page.getByRole("button", { name: "Stop run", exact: true }).click();
  await finish(t, "Stopped");
  assert.equal(
    await t.page.getByRole("button", { name: /Apply \d+ files/ }).count(),
    0,
  );
  await t.page.screenshot({ path: out + "/stopped.png" });
  results.push("Stop → cooperative cancellation → partial receipt, no Apply");
  await close(t);
  t = await setup("fixture-fail");
  await dispatch(t, "Explain this repository without modifying files.");
  await finish(t, "Failed");
  await t.page
    .locator(".run-banners")
    .getByText(/Deliberate test provider failure/)
    .first()
    .waitFor({ timeout: 15000 });
  await t.page
    .getByRole("button", { name: "Jump to failure in trace" })
    .click();
  await t.page.locator(".trace-row.flash").first().waitFor();
  await t.page.screenshot({ path: out + "/failed.png" });
  results.push("provider failure → recorded reason → Jump to failure");
  await close(t);
  t = await setup("fixture-verified");
  let template = execFileSync(
    binary,
    [
      "--workspace",
      t.workspace,
      "task-template",
      "Fix addition and prove add(2,3) returns 5",
    ],
    { encoding: "utf8" },
  );
  const obligation = template.match(/id = "([^"]+)"/)[1];
  template = template.replace(
    /    "(?:network|secret_use|external_write)",\n/g,
    "",
  );
  template = template.replace(
    '    "memory_read",',
    '    "memory_read",\n    "process_spawn",\n    "network",\n    "secret_use",\n    "external_write",',
  );
  template +=
    '\n[[acceptance_checks]]\nobligation_id = "' +
    obligation +
    '"\nprogram = "cargo"\nargs = ["test", "--offline", "addition"]\ndescription = "Requested addition behavior"\n';
  const task = path.join(out, "acceptance-task.toml");
  fs.writeFileSync(task, template);
  const checked = JSON.parse(
    execFileSync(
      binary,
      [
        "--workspace",
        t.workspace,
        "run",
        "--task",
        task,
        "--provider",
        "open-ai-compatible",
        "--model",
        "fixture-verified",
        "--base-url",
        "http://127.0.0.1:4190/v1",
        "--process-backend",
        "native",
        "--process-approval",
        "allow-run",
        "--output",
        "json",
      ],
      { encoding: "utf8", timeout: 30000 },
    ),
  );
  await t.page.goto(t.url + "/runs/" + checked.run_id + "?view=changes");
  await t.page
    .getByRole("button", { name: "Apply 1 file", exact: true })
    .waitFor();
  await t.page
    .getByRole("button", { name: "Apply 1 file", exact: true })
    .click();
  assert.equal(
    await t.page
      .locator("#dialog")
      .getByRole("button", { name: "Apply 1 file" })
      .isEnabled(),
    true,
  );
  await t.page.screenshot({ path: out + "/acceptance-confirm.png" });
  await t.page
    .locator("#dialog")
    .getByRole("button", { name: "Apply 1 file" })
    .click();
  await t.page
    .locator(".run-banners h2")
    .filter({ hasText: "Applied 1 file" })
    .waitFor();
  results.push(
    "CLI acceptance-bound deterministic pass → single-confirm Apply",
  );
  await close(t);
  t = await setup("fixture-stop");
  await dispatch(t, "Explain this repository without modifying files.");
  await t.page.locator(".run-heading").waitFor({ timeout: 15000 });
  await t.page
    .getByRole("searchbox", { name: "Search trace" })
    .fill("contract");
  await t.page
    .locator(".trace-row-head")
    .filter({ hasText: "Task contract registered" })
    .click();
  const before = await t.page.locator(".trace-row").count();
  process.kill(-t.server.pid, "SIGKILL");
  await t.page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await t.page
    .locator("#connection-banner")
    .filter({ hasText: "Lost connection" })
    .waitFor({ timeout: 10000 });
  t.server = spawn(
    binary,
    ["--workspace", t.workspace, "web", "--port", new URL(t.url).port],
    { stdio: ["ignore", "pipe", "pipe"], detached: true },
  );
  children.add(t.server.pid);
  await new Promise((resolve) => t.server.stdout.once("data", resolve));
  await t.page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await t.page
    .locator("#connection-banner")
    .filter({ hasText: "Reconnected" })
    .waitFor({ timeout: 15000 });
  await t.page.waitForTimeout(500);
  assert.equal(
    await t.page.getByRole("searchbox", { name: "Search trace" }).inputValue(),
    "contract",
  );
  assert.equal(
    await t.page.locator('.trace-row-head[aria-expanded="true"]').count(),
    1,
  );
  assert.equal(await t.page.locator(".trace-row").count(), before);
  await t.page.screenshot({ path: out + "/engine-reconnected.png" });
  results.push(
    "kill engine mid-run → reconnect → no duplicated rows, filter and expansion retained",
  );
  await close(t);
  await browser.close();
  fs.writeFileSync(out + "/results.json", JSON.stringify(results, null, 2));
  console.log(JSON.stringify({ results, out }));
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
