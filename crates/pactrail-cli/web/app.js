"use strict";
const $ = (id) => document.getElementById(id);
const state = {
  runs: [],
  jobs: [],
  selected: null,
  tab: "timeline",
  details: {},
  config: {
    provider: "ollama",
    model: "",
    base_url: "",
    api_key_env: "OPENAI_API_KEY",
    max_turns: 24,
    process_backend: "disabled",
  },
  timer: null,
};
const el = (tag, cls, text) => {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text !== undefined) node.textContent = String(text);
  return node;
};
const clear = (node) => node.replaceChildren();
const short = (value) => String(value || "").slice(0, 8);
const status = (run) =>
  String(run.outcome || run.state || "unknown")
    .replaceAll("_", " ")
    .replace(/([a-z])([A-Z])/g, "$1 $2");
const isFailed = (value) => /fail|reject|abort|cancel/i.test(String(value));
const isReady = (value) => /ready/i.test(String(value));
function toast(message, error = false) {
  const node = $("toast");
  node.textContent = message;
  node.classList.toggle("error", error);
  node.classList.remove("hidden");
  clearTimeout(state.toastTimer);
  state.toastTimer = setTimeout(() => node.classList.add("hidden"), 4500);
}
async function api(path, options) {
  const response = await fetch(path, {
    ...options,
    headers: {
      ...(options?.headers || {}),
      ...(options?.body ? { "Content-Type": "application/json" } : {}),
    },
  });
  let data;
  try {
    data = await response.json();
  } catch {
    throw Error("Unexpected server response");
  }
  if (!response.ok)
    throw Error(data.error || `Request failed (${response.status})`);
  return data;
}
function configFromForm() {
  return {
    provider: $("provider").value,
    model: $("model").value.trim(),
    base_url: $("base-url").value.trim(),
    api_key_env: $("api-key-env").value.trim(),
    max_turns: Number($("max-turns").value),
    process_backend: $("process-backend").value,
  };
}
function syncConfig() {
  const c = state.config;
  $("provider").value = c.provider;
  $("model").value = c.model;
  $("base-url").value = c.base_url || "";
  $("api-key-env").value = c.api_key_env || "";
  $("max-turns").value = c.max_turns || 24;
  $("process-backend").value = c.process_backend || "disabled";
  $("model-label").textContent = c.model || "Choose a model";
  $("runtime-provider").textContent = c.provider.replaceAll("-", " ");
  $("runtime-model").textContent = c.model || "—";
  $("runtime-process").textContent =
    c.process_backend === "disabled"
      ? "Disabled"
      : c.process_backend.toUpperCase();
  updateProcessNote();
}
function updateProcessNote() {
  const type = $("process-backend").value;
  $("process-note").textContent =
    type === "native"
      ? "Native process access can reach your host, network, and environment. Use only in a trusted workspace."
      : type === "oci"
        ? "Process commands run in the configured OCI sandbox. The runtime and image must be available locally."
        : "The agent can read and edit isolated candidate files. Process execution is disabled.";
}
function openSettings() {
  syncConfig();
  $("settings-backdrop").classList.remove("hidden");
  $("model").focus();
}
function closeSettings() {
  $("settings-backdrop").classList.add("hidden");
}
function renderRuns() {
  const list = $("run-list");
  clear(list);
  $("run-count").textContent = String(state.runs.length);
  if (!state.runs.length) {
    list.append(el("div", "history-empty", "No runs yet"));
    return;
  }
  for (const run of state.runs) {
    const button = el(
      "button",
      "history-item" + (state.selected === run.run_id ? " selected" : ""),
    );
    const title = el("div", "history-title", run.goal || short(run.run_id));
    const meta = el("div", "history-meta");
    meta.append(
      el("span", "history-status" + (isFailed(status(run)) ? " failed" : "")),
      el("span", "", status(run)),
      el("span", "", "· " + (run.changes || 0) + " files"),
    );
    button.append(title, meta);
    button.onclick = () => selectRun(run.run_id);
    list.append(button);
  }
}
function renderActivity() {
  const activity = $("activity");
  clear(activity);
  if (!state.runs.length) {
    activity.append(
      $("empty-template")?.content?.cloneNode(true) || emptyActivity(),
    );
    return;
  }
  for (const run of state.showAll ? state.runs : state.runs.slice(0, 5)) {
    const card = el("button", "activity-card");
    const icon = el("div", "activity-icon", isFailed(status(run)) ? "!" : "◇");
    const info = el("div", "activity-info");
    info.append(
      el("strong", "", run.goal || short(run.run_id)),
      el(
        "small",
        "",
        `${status(run)} · ${run.changes || 0} changed files · ${short(run.run_id)}`,
      ),
    );
    card.append(icon, info, el("span", "activity-arrow", "↗"));
    card.onclick = () => selectRun(run.run_id);
    activity.append(card);
  }
}
function emptyActivity() {
  const container = el("div", "empty-activity");
  const orbit = el("div", "empty-orbit");
  orbit.append(el("div", "orbit-inner", "◇"));
  container.append(
    orbit,
    el("h2", "", "A clear trail starts here."),
    el(
      "p",
      "",
      "Your runs will appear here with their changes, evidence, and decisions.",
    ),
  );
  return container;
}
function renderJobs() {
  const active = state.jobs.find((j) => j.state === "running");
  const summary = $("inspector-summary");
  if (active) {
    clear(summary);
    summary.append(
      el("div", "inspector-empty-icon", "◌"),
      el("strong", "", "Agent running"),
      el("p", "", active.goal),
    );
    $("submit-task").disabled = true;
    return;
  }
  $("submit-task").disabled = false;
  const latest = state.jobs[0];
  if (latest && latest.state === "failed") {
    clear(summary);
    summary.append(
      el("div", "inspector-empty-icon", "!"),
      el("strong", "", "Run failed"),
      el("p", "", latest.error || "Check the configuration."),
    );
    return;
  }
  if (state.selected) {
    const run = state.runs.find((r) => r.run_id === state.selected);
    if (run) {
      clear(summary);
      summary.append(
        el("div", "inspector-empty-icon", "◇"),
        el("strong", "", status(run)),
        el("p", "", `${run.changes || 0} changed files · ${short(run.run_id)}`),
      );
      return;
    }
  }
  clear(summary);
  summary.append(
    el("div", "inspector-empty-icon", "◇"),
    el("strong", "", "No active run"),
    el("p", "", "Start a task to see its progress and evidence here."),
  );
}
async function refresh() {
  try {
    const [runs, jobs] = await Promise.all([
      api("/api/runs"),
      api("/api/jobs"),
    ]);
    const prior = state.jobs.find((j) => j.state === "running");
    state.runs = Array.isArray(runs)
      ? runs.sort((a, b) => String(b.run_id).localeCompare(String(a.run_id)))
      : [];
    state.jobs = Array.isArray(jobs) ? jobs : [];
    renderRuns();
    renderJobs();
    if (!state.selected) {
      renderActivity();
    } else if (prior && !state.jobs.some((j) => j.state === "running")) {
      state.details = {};
      await loadDetail(state.selected, state.tab);
    }
    if (state.jobs.some((j) => j.state === "running")) {
      renderActiveJob();
      if (state.selected && state.tab === "timeline") {
        const events = await api(
          `/api/runs/${encodeURIComponent(state.selected)}/trace`,
        ).catch(() => null);
        if (events && events.length !== state.details.timeline?.length) {
          state.details.timeline = events;
          const body = $("detail-body");
          clear(body);
          renderTimeline(body, events, state.details.inspect);
        }
      }
    }
  } catch (error) {
    toast(error.message, true);
  }
}
function renderActiveJob() {
  const activity = $("activity");
  if (state.selected) return;
  const job = state.jobs.find((j) => j.state === "running");
  if (!job) return;
  const card = el("div", "job-card");
  card.append(el("span", "spin", "◌"));
  const text = el("div", "job-text");
  text.append(
    el("strong", "", "Pactrail is working"),
    el("small", "", job.goal),
  );
  card.append(text);
  const cancel = el("button", "danger", "Stop");
  cancel.onclick = async () => {
    try {
      await api("/api/jobs/" + encodeURIComponent(job.id) + "/cancel", {
        method: "POST",
      });
      toast("Stopping run…");
      refresh();
    } catch (e) {
      toast(e.message, true);
    }
  };
  card.append(cancel);
  clear(activity);
  activity.append(card);
  for (const run of state.runs.slice(0, 4)) {
    const item = el("button", "activity-card");
    item.append(
      el("div", "activity-icon", "◇"),
      el("div", "activity-info", run.goal || short(run.run_id)),
    );
    item.onclick = () => selectRun(run.run_id);
    activity.append(item);
  }
}
async function selectRun(id) {
  state.selected = id;
  state.details = {};
  state.tab = "timeline";
  $("welcome").classList.add("hidden");
  $("session-head").classList.remove("hidden");
  $("activity-heading").classList.add("hidden");
  $("activity").classList.add("hidden");
  $("detail").classList.remove("hidden");
  const run = state.runs.find((r) => r.run_id === id);
  $("page-title").textContent = "Run " + short(id);
  $("run-short").textContent = short(id);
  $("session-title").textContent = run?.goal || "Run " + short(id);
  $("run-id").textContent = id;
  $("run-changes").textContent = `${run?.changes || 0} files changed`;
  $("run-state").textContent = status(run || {});
  $("run-state").className =
    "state-pill" +
    (isFailed(status(run || {}))
      ? " failed"
      : isReady(status(run || {}))
        ? ""
        : "");
  renderRuns();
  renderJobs();
  await loadDetail(id, "timeline");
}
function overview() {
  state.selected = null;
  state.tab = "timeline";
  $("welcome").classList.remove("hidden");
  $("session-head").classList.add("hidden");
  $("activity-heading").classList.remove("hidden");
  $("activity").classList.remove("hidden");
  $("detail").classList.add("hidden");
  $("page-title").textContent = "Overview";
  renderRuns();
  renderActivity();
  renderJobs();
  $("goal").focus();
}
async function loadDetail(id, tab) {
  state.tab = tab;
  document
    .querySelectorAll(".tab")
    .forEach((button) =>
      button.classList.toggle("active", button.dataset.tab === tab),
    );
  const body = $("detail-body");
  clear(body);
  body.append(el("div", "detail-placeholder", "Loading " + tab + "…"));
  try {
    const [inspect, data] = await Promise.all([
      state.details.inspect ||
        api(`/api/runs/${encodeURIComponent(id)}/inspect`),
      state.details[tab] ||
        api(
          `/api/runs/${encodeURIComponent(id)}/${tab === "timeline" ? "trace" : tab === "receipt" ? "inspect" : "diff"}`,
        ),
    ]);
    if (state.selected !== id || state.tab !== tab) return;
    state.details.inspect = inspect;
    state.details[tab] = data;
    clear(body);
    if (tab === "timeline") renderTimeline(body, data, inspect);
    else if (tab === "diff") renderDiff(body, data, inspect);
    else renderReceipt(body, inspect);
  } catch (error) {
    if (state.selected !== id) return;
    clear(body);
    body.append(el("div", "error-card", error.message));
  }
}
function reviewActions(body, inspect) {
  if (!inspect) return;
  if (inspect.receipt === null) {
    const bar = el("div", "review-bar");
    bar.append(
      el(
        "p",
        "",
        "This run has no receipt yet. Continue it from its latest safe checkpoint.",
      ),
    );
    const resume = el("button", "primary", "Resume run");
    resume.onclick = () => action("resume");
    bar.append(resume);
    body.append(bar);
    return;
  }
  if (!isReady(inspect.outcome)) return;
  const bar = el("div", "review-bar");
  bar.append(
    el(
      "p",
      "",
      "Candidate is ready. Review the diff before applying it to your workspace.",
    ),
  );
  const apply = el("button", "primary", "Apply changes");
  apply.onclick = () => action("apply");
  const discard = el("button", "danger", "Discard");
  discard.onclick = () => action("discard");
  bar.append(apply, discard);
  body.append(bar);
}
async function action(kind) {
  if (
    kind === "discard" &&
    !confirm(
      "Discard this candidate? Its receipt and trace will remain available.",
    )
  )
    return;
  try {
    await api(`/api/runs/${encodeURIComponent(state.selected)}/${kind}`, {
      method: "POST",
    });
    toast(
      kind === "resume"
        ? "Run resumed"
        : kind === "apply"
          ? "Changes applied to workspace"
          : "Candidate discarded",
    );
    state.details = {};
    await refresh();
    if (kind !== "resume") await selectRun(state.selected);
  } catch (error) {
    toast(error.message, true);
  }
}
function renderDiff(body, diff, inspect) {
  reviewActions(body, inspect);
  const changes = Array.isArray(diff.changes) ? diff.changes : [];
  if (!changes.length) {
    body.append(
      el("div", "detail-placeholder", "No file changes in this run."),
    );
    return;
  }
  const panel = el("div", "code-panel");
  panel.append(el("pre", "", diff.unified_diff || ""));
  body.append(panel);
}
function renderReceipt(body, receipt) {
  reviewActions(body, receipt);
  const completedJob = state.jobs.find(
    (j) => j.output?.run_id === receipt.run_id,
  );
  const webResult = receipt.web_result || completedJob?.output;
  if (webResult?.summary) {
    const section = el("div", "receipt-section");
    section.append(
      el("h3", "", "AGENT SUMMARY"),
      el("div", "summary-text", webResult.summary),
    );
    body.append(section);
  }
  const grid = el("div", "receipt-grid");
  for (const [label, value] of [
    ["Outcome", receipt.outcome || receipt.state || "—"],
    ["Changed files", (receipt.changes || []).length],
    ["Evidence passed", receipt.verification?.passed ?? "—"],
    ["Evidence failed", receipt.verification?.failed ?? "—"],
    ["Tokens used", webResult?.tokens ?? "—"],
    [
      "Estimated cost",
      webResult?.cost_microusd == null
        ? "—"
        : "$" + (webResult.cost_microusd / 1000000).toFixed(4),
    ],
  ]) {
    const card = el("div", "receipt-stat");
    card.append(el("span", "", label), el("strong", "", value));
    grid.append(card);
  }
  body.append(grid);
  const changes = receipt.changes || [];
  if (changes.length) {
    const section = el("div", "receipt-section");
    section.append(el("h3", "", "CHANGED FILES"));
    const list = el("ul", "receipt-list");
    for (const change of changes)
      list.append(el("li", "", change.path || String(change)));
    section.append(list);
    body.append(section);
  }
  if (receipt.unresolved_risks?.length) {
    const section = el("div", "receipt-section");
    section.append(el("h3", "", "UNRESOLVED RISKS"));
    const list = el("ul", "receipt-list");
    for (const risk of receipt.unresolved_risks)
      list.append(
        el("li", "", typeof risk === "string" ? risk : JSON.stringify(risk)),
      );
    section.append(list);
    body.append(section);
  }
}
function eventText(event) {
  const data = event.event || event.payload || event;
  const kind = data.type || "event";
  const value = data.data ?? {};
  if (
    [
      "checkpoint_created",
      "effect_prepared",
      "effect_completed",
      "contract_registered",
    ].includes(kind)
  )
    return null;
  if (
    kind === "action_completed" &&
    ["assess_progress", "enter_phase"].includes(value.action)
  )
    return null;
  if (kind === "state_changed")
    return {
      kind: "State · " + String(value.to || "updated").replaceAll("_", " "),
      text: "The run moved to its next stage.",
    };
  if (kind === "action_completed")
    return {
      kind: value.action || "Action completed",
      text: value.summary || "Completed",
      meta: [
        value.actor,
        value.duration_ms !== undefined ? value.duration_ms + " ms" : null,
      ]
        .filter(Boolean)
        .join(" · "),
    };
  if (kind === "evidence_recorded")
    return {
      kind: "Evidence · " + (value.status || "recorded"),
      text: value.summary || "Verification evidence recorded.",
    };
  if (kind === "note_recorded")
    return { kind: "Agent note", text: value.message || "" };
  if (kind === "approval_decided")
    return {
      kind: "Approval decision",
      text: value.decision || "A scoped decision was recorded.",
    };
  if (kind === "policy_evaluated")
    return {
      kind: "Policy check",
      text: value.reason || "A capability was evaluated.",
    };
  return { kind: kind.replaceAll("_", " "), text: "Event recorded" };
}
function renderTimeline(body, events, inspect) {
  reviewActions(body, inspect);
  if (!Array.isArray(events) || !events.length) {
    body.append(el("div", "detail-placeholder", "No trace events yet."));
    return;
  }
  for (const event of events) {
    const info = eventText(event);
    if (!info) continue;
    const row = el("div", "timeline-item");
    row.append(el("strong", "", info.kind), el("p", "", info.text));
    if (info.meta) row.append(el("small", "", info.meta));
    body.append(row);
  }
}
async function submit() {
  const goal = $("goal").value.trim();
  if (!goal) {
    $("goal").focus();
    return;
  }
  if (!state.config.model) {
    openSettings();
    toast("Choose a model before starting");
    return;
  }
  const request = { ...state.config, goal, apply: false };
  try {
    await api("/api/runs", { method: "POST", body: JSON.stringify(request) });
    $("goal").value = "";
    overview();
    toast("Run started. Changes will wait for review.");
    await refresh();
  } catch (error) {
    toast(error.message, true);
  }
}
async function start() {
  try {
    const boot = await api("/api/bootstrap");
    $("workspace-path").textContent = boot.workspace;
    const parts = boot.workspace.split(/[\\/]/).filter(Boolean);
    $("workspace-name").textContent = parts.at(-1) || boot.workspace;
    $("version").textContent = "v" + boot.version;
    state.config = { ...state.config, ...boot.defaults };
    const saved = localStorage.getItem("pactrail-run-config");
    if (saved) {
      try {
        const parsed = JSON.parse(saved);
        for (const key of [
          "provider",
          "model",
          "base_url",
          "api_key_env",
          "max_turns",
          "process_backend",
        ])
          if (parsed[key] !== undefined) state.config[key] = parsed[key];
      } catch {}
    }
    syncConfig();
    await refresh();
    state.timer = setInterval(refresh, 2500);
  } catch (error) {
    $("workspace-path").textContent = "Connection failed";
    toast(error.message, true);
  }
}
$("new-task").onclick = () => {
  overview();
  $("sidebar").classList.remove("open");
};
$("menu-button").onclick = () => $("sidebar").classList.add("open");
$("close-sidebar").onclick = () => $("sidebar").classList.remove("open");
$("refresh").onclick = refresh;
$("view-all").onclick = () => {
  state.showAll = !state.showAll;
  $("view-all").textContent = state.showAll
    ? "Show recent ↗"
    : "View all runs ↗";
  renderActivity();
};
$("settings-toggle").onclick = openSettings;
$("settings-close").onclick = closeSettings;
$("settings-cancel").onclick = closeSettings;
$("settings-backdrop").onclick = (event) => {
  if (event.target === $("settings-backdrop")) closeSettings();
};
$("settings-save").onclick = () => {
  const next = configFromForm();
  if (!next.model) {
    toast("Enter a model identifier", true);
    return;
  }
  if (!next.max_turns || next.max_turns < 1 || next.max_turns > 200) {
    toast("Maximum turns must be between 1 and 200", true);
    return;
  }
  state.config = next;
  localStorage.setItem("pactrail-run-config", JSON.stringify(next));
  syncConfig();
  closeSettings();
  toast("Runtime settings saved locally");
};
$("process-backend").onchange = updateProcessNote;
$("submit-task").onclick = submit;
$("goal").onkeydown = (event) => {
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    submit();
  }
};
document.querySelectorAll(".tab").forEach(
  (button) =>
    (button.onclick = () => {
      if (state.selected) loadDetail(state.selected, button.dataset.tab);
    }),
);
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") closeSettings();
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
    event.preventDefault();
    overview();
  }
});
start();
