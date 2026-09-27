"use strict";
const $ = (id) => document.getElementById(id),
  node = (tag, cls, text) => {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined) e.textContent = String(text);
    return e;
  },
  clear = (e) => e.replaceChildren();
const store = (key, value) =>
  localStorage.setItem("pactrail." + key, JSON.stringify(value));
const recall = (key, fallback) => {
  try {
    const v = JSON.parse(localStorage.getItem("pactrail." + key));
    return v ?? fallback;
  } catch {
    return fallback;
  }
};
const state = {
  bootstrap: null,
  runs: [],
  jobs: [],
  detail: null,
  diff: null,
  events: [],
  route: "/",
  tab: "diff",
  compact: "trace",
  filter: recall("filter", "all"),
  traceFilter: recall("traceFilter", "all"),
  search: "",
  theme: recall("theme", null),
  diffMode: recall("diffMode", "unified"),
  titles: recall("titles", {}),
  config: recall("config", null),
  eventSource: null,
  jobId: null,
  offline: false,
  loading: false,
  following: true,
  newEvents: 0,
  lastCount: 0,
  selectedFile: null,
  traceLimit: 500,
  ledgerLimit: 200,
  ledgerCollapsed: recall("ledgerCollapsed", false),
  diffLineLimit: 500,
};
const icon = (name) =>
  `<svg aria-hidden="true"><use href="/icons.svg#${name}"></use></svg>`;
const html = (s) =>
  String(s ?? "").replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ],
  );
const short = (id) => String(id || "").slice(0, 8);
const runTitle = (r) =>
  state.titles[r?.run_id] ||
  (r?.goal ? r.goal.split(/[.!?]\s+/)[0].slice(0, 200) : short(r?.run_id));
const status = (r) => {
  const job = state.jobs.find(
    (j) => j.goal === r?.goal && j.state === "cancelled",
  );
  const s = String(
    r?.outcome || (job ? "cancelled" : r?.state) || "",
  ).toLowerCase();
  if (s === "ready_to_apply" || s === "awaiting_apply")
    return { key: "ready", label: "Awaiting review" };
  if (s === "applied") return { key: "applied", label: "Applied" };
  if (s === "discarded") return { key: "discarded", label: "Discarded" };
  if (s === "failed") return { key: "failed", label: "Failed" };
  if (s === "cancelled")
    return { key: "cancelled", label: "Stopped · partial" };
  if (s === "answered" || s === "completed")
    return { key: "applied", label: "Answered" };
  return { key: "running", label: s === "created" ? "Dispatched" : "Running" };
};
const fmtTime = (value) => {
  const d = new Date(value);
  return Number.isNaN(d.getTime())
    ? ""
    : d.toLocaleTimeString([], {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
        hour12: false,
      });
};
const fmtDate = (value) => {
  const d = new Date(value);
  return Number.isNaN(d.getTime())
    ? ""
    : d.toLocaleDateString([], {
        year: "numeric",
        month: "short",
        day: "numeric",
      });
};
const elapsed = (value, end) => {
  const n = Math.max(
    0,
    Math.floor(
      ((end ? new Date(end).getTime() : Date.now()) -
        new Date(value).getTime()) /
        1000,
    ),
  );
  if (!Number.isFinite(n)) return "";
  return n >= 3600
    ? `${Math.floor(n / 3600)}:${String(Math.floor((n % 3600) / 60)).padStart(2, "0")}:${String(n % 60).padStart(2, "0")}`
    : `${Math.floor(n / 60)}:${String(n % 60).padStart(2, "0")}`;
};
const price = (n) =>
  n == null ? "" : "$" + (Number(n) / 1e6).toFixed(Number(n) < 1e6 ? 4 : 2);
function toast(message, error = false) {
  const e = $("toast");
  e.textContent = message;
  e.classList.toggle("error", error);
  e.hidden = false;
  clearTimeout(state.toastTimer);
  state.toastTimer = setTimeout(() => (e.hidden = true), 4000);
}
async function api(path, options = {}) {
  let r;
  try {
    r = await fetch(path, {
      ...options,
      headers: {
        ...(options.body ? { "Content-Type": "application/json" } : {}),
        ...options.headers,
      },
    });
  } catch (error) {
    const e = Error("Engine unreachable");
    e.network = true;
    throw e;
  }
  let value;
  try {
    value = await r.json();
  } catch {
    throw Error("Unexpected engine response");
  }
  if (!r.ok) throw Error(value.error || `Request failed (${r.status})`);
  return value;
}
function navigate(path) {
  history.pushState({}, "", path);
  route();
}
function setTheme(choice) {
  state.theme = choice;
  store("theme", choice);
  document.documentElement.dataset.theme = choice || "";
  $("theme-toggle").innerHTML = icon(choice === "carbon" ? "sun" : "moon");
  document.querySelector('meta[name="theme-color"]').content =
    choice === "carbon" ? "#131210" : "#F5F2EA";
}
function route() {
  closeLedger();
  const path = location.pathname;
  state.route = path;
  if (path === "/" || path === "/settings") {
    closeStream();
    state.detail = null;
    state.events = [];
    state.diff = null;
    state.traceLimit = 500;
    state.jobId = null;
  }
  if (path === "/") {
    state.tab = "diff";
    $("breadcrumb").textContent = "DISPATCH / NEW RUN";
    renderDispatch();
  } else if (path === "/settings") {
    $("breadcrumb").textContent = "SETTINGS";
    renderSettings();
  } else if (/^\/runs\/[0-9a-f-]{36}$/.test(path)) {
    const id = path.split("/")[2];
    $("breadcrumb").textContent = `RUN / ${short(id)}`;
    if (state.detail?.run_id !== id) {
      state.detail = null;
      state.diff = null;
      state.events = [];
      state.traceLimit = 500;
      state.selectedFile = null;
      loadRun(id);
    } else renderRun();
  } else {
    history.replaceState({}, "", "/");
    route();
  }
  renderLedger();
  updateTitle();
}
function renderLedger() {
  const list = $("ledger-list");
  clear(list);
  let rows = state.runs.filter(
    (r) => state.filter === "all" || status(r).key === state.filter,
  );
  if (state.search.trim())
    rows = rows.filter(
      (r) =>
        (r.goal || "").toLowerCase().includes(state.search.toLowerCase()) ||
        r.run_id.includes(state.search),
    );
  const filters = $("ledger-filters");
  clear(filters);
  for (const [key, label] of [
    ["all", "All"],
    ["running", "Running"],
    ["ready", "Review"],
    ["applied", "Applied"],
    ["failed", "Failed"],
  ]) {
    const b = node(
      "button",
      "filter" + (state.filter === key ? " active" : ""),
      label,
    );
    b.type = "button";
    b.onclick = () => {
      state.filter = key;
      store("filter", key);
      renderLedger();
    };
    filters.append(b);
  }
  if (!state.runs.length) {
    list.append(
      node(
        "p",
        "ledger-empty",
        "No runs yet. Dispatch your first task — changes stay isolated until you apply them.",
      ),
    );
    return;
  }
  if (!rows.length) {
    const p = node(
      "p",
      "ledger-empty",
      `No runs match ${state.search ? `“${state.search}”` : "this filter"}.`,
    );
    const b = node("button", "btn quiet", "Clear filters");
    b.onclick = () => {
      state.search = "";
      state.filter = "all";
      $("ledger-search").value = "";
      store("filter", "all");
      renderLedger();
    };
    list.append(p, b);
    return;
  }
  let group = "";
  for (const r of rows.slice(0, state.ledgerLimit)) {
    const time = runTime(r);
    const d = time ? new Date(time) : null;
    const today = new Date().toDateString(),
      yesterday = new Date(Date.now() - 86400000).toDateString();
    const next =
      d?.toDateString() === today
        ? "TODAY"
        : d?.toDateString() === yesterday
          ? "YESTERDAY"
          : "EARLIER";
    if (next !== group) {
      group = next;
      list.append(node("div", "ledger-group eyebrow", group));
    }
    const s = status(r),
      a = node(
        "a",
        "ledger-item" +
          (location.pathname === "/runs/" + r.run_id ? " selected" : ""),
      );
    a.href = "/runs/" + r.run_id;
    a.dataset.link = "";
    a.innerHTML = `<div class="ledger-title"><span class="lamp ${s.key}"></span><span>${html(runTitle(r))}</span></div><div class="ledger-meta">${html(s.label)}${time ? " · " + html(fmtDate(time)) : ""}</div>`;
    list.append(a);
  }
  if (rows.length > state.ledgerLimit) {
    const more = node(
      "button",
      "btn quiet ledger-more",
      `Show ${Math.min(200, rows.length - state.ledgerLimit)} more runs`,
    );
    more.onclick = () => {
      state.ledgerLimit += 200;
      renderLedger();
    };
    list.append(more);
  }
}
function renderDispatch() {
  const main = $("main");
  main.className = "main";
  main.innerHTML = `<section class="dispatch"><div class="eyebrow overline">NEW RUN / TASK BRIEF</div><h1>What needs to change?</h1><form id="dispatch-form"><div class="field"><label class="field-label" for="task-title">Title <span class="optional">optional · stored in this browser</span></label><input class="text-input" id="task-title" maxlength="200" placeholder="A short name for this run"></div><div class="field"><label class="field-label" for="task-goal">Task brief</label><textarea class="brief" id="task-goal" maxlength="16000" required placeholder="Describe the change you want. Include the failing symptom, the file or area if you know it, and what 'done' looks like."></textarea><div id="dispatch-error" class="error-text" role="alert" hidden></div></div><div class="config-row"><div class="config-box"><span class="eyebrow">WORKSPACE</span><div class="config-value" title="${html(state.bootstrap?.workspace || "")}">${icon("terminal")}<span>${html(state.bootstrap?.workspace || "Connecting…")}</span></div></div><div class="config-box"><label class="eyebrow" for="provider">PROVIDER / MODEL</label><select class="select" id="provider"></select><input class="text-input mono" id="model" placeholder="Model ID" aria-label="Model ID" style="margin-top:6px"></div></div><details class="guardrails"><summary>Guardrails <span class="micro">ENFORCED BY ENGINE</span></summary><div class="guard-grid"><label>Maximum turns<input id="max-turns" class="number-input mono" type="number" min="1" max="200" value="24"></label><label>Max cost, USD<input id="max-cost" class="number-input mono" type="number" min="0" step="0.000001" placeholder="Not set"><small>Requires pricing configured in CLI.</small></label><label>Request timeout, seconds<input id="timeout" class="number-input mono" type="number" min="1" max="3600" value="300"></label><label>Process execution<select id="process-backend" class="select mono"><option value="disabled">Disabled</option><option value="oci">OCI sandbox</option><option value="native">Native host</option></select><small>Native runs commands on your host.</small></label><label id="sandbox-image-wrap" hidden>Local OCI image<input id="sandbox-image" class="text-input mono" placeholder="image:tag"></label><label>Base URL override<input id="base-url" class="text-input mono" placeholder="Provider default"></label><label>API key variable<input id="api-key-env" class="text-input mono" placeholder="Environment variable name"></label></div></details><div class="dispatch-actions"><button id="dispatch-button" class="btn solid" type="submit">${icon("arrow-right")} Dispatch run</button><span class="muted">⌘ / Ctrl + Enter</span></div></form><div class="recent-head"><span class="eyebrow">RECENT RUNS</span><a href="#ledger" id="all-runs">All runs →</a></div><div id="recent-list" class="recent-list"></div></section>`;
  const c = state.config || state.bootstrap?.defaults || {};
  const provider = $("provider");
  for (const p of state.bootstrap?.providers || [
    "ollama",
    "open-ai-compatible",
    "open-ai",
    "anthropic",
    "gemini",
  ]) {
    const o = node("option", "", p);
    o.value = p;
    provider.append(o);
  }
  provider.value = c.provider || "ollama";
  $("model").value = c.model || "";
  $("base-url").value = c.base_url || "";
  $("api-key-env").value = c.api_key_env || "";
  $("process-backend").onchange = () => {
    $("sandbox-image-wrap").hidden = $("process-backend").value !== "oci";
  };
  provider.onchange = () => {
    if (provider.value !== c.provider) {
      $("base-url").value = "";
      $("api-key-env").value = "";
    }
  };
  $("task-goal").value = state.prefill || "";
  state.prefill = "";
  $("dispatch-form").onsubmit = dispatch;
  const recent = $("recent-list");
  if (!state.runs.length)
    recent.append(
      node(
        "p",
        "empty-note",
        "No runs yet. Dispatch your first task above — everything runs in an isolated workspace until you apply it.",
      ),
    );
  for (const r of state.runs.slice(0, 5)) {
    const s = status(r),
      a = node("a", "recent-item");
    a.href = "/runs/" + r.run_id;
    a.dataset.link = "";
    a.innerHTML = `<span class="lamp ${s.key}"></span><span class="title">${html(runTitle(r))}</span><span class="meta">${html(s.label)}</span>`;
    recent.append(a);
  }
  $("all-runs").onclick = (e) => {
    e.preventDefault();
    openLedger();
  };
}
async function dispatch(e) {
  e.preventDefault();
  const goal = $("task-goal").value.trim(),
    model = $("model").value.trim(),
    provider = $("provider").value;
  const err = $("dispatch-error");
  err.hidden = true;
  if (!goal || !model) {
    err.textContent = "Enter a task brief and model ID.";
    err.hidden = false;
    return;
  }
  const maxCost = $("max-cost").value.trim();
  const body = {
    goal,
    provider,
    model,
    max_turns: Number($("max-turns").value) || 24,
    request_timeout_seconds: Number($("timeout").value) || 300,
    process_backend: $("process-backend").value,
  };
  body.base_url = $("base-url").value.trim() || undefined;
  body.api_key_env = $("api-key-env").value.trim() || undefined;
  if (body.process_backend === "oci")
    body.sandbox_image = $("sandbox-image").value.trim();
  if (maxCost) body.max_cost_microusd = Math.round(Number(maxCost) * 1e6);
  state.config = {
    provider,
    model,
    base_url: body.base_url,
    api_key_env: body.api_key_env,
  };
  store("config", state.config);
  const button = $("dispatch-button");
  button.disabled = true;
  button.textContent = "Dispatching…";
  try {
    const job = await api("/api/runs", {
      method: "POST",
      body: JSON.stringify(body),
    });
    state.jobId = job.id;
    state.pendingGoal = goal;
    state.pendingTitle = $("task-title").value.trim();
    state.pendingIds = new Set(state.runs.map((r) => r.run_id));
    toast("Run dispatched. Waiting for the engine to register it.");
    await refresh();
  } catch (error) {
    err.textContent = error.message;
    err.hidden = false;
    button.disabled = false;
    button.innerHTML = icon("arrow-right") + " Dispatch run";
  }
}
function runTime(r) {
  const id = r.run_id;
  if (r.created_at) return r.created_at;
  const t = state.events[0]?.timestamp;
  if (location.pathname === "/runs/" + id && t) return t;
  return null;
}
async function loadRun(id) {
  $("main").innerHTML =
    '<div class="loading-run"><div class="skeleton-row"></div><div class="skeleton-row"></div><div class="skeleton-row"></div></div>';
  try {
    const detail = await api(`/api/runs/${id}/inspect`);
    const trace = await api(`/api/runs/${id}/trace`);
    state.detail = detail;
    state.events = Array.isArray(trace) ? trace : [];
    state.diff = null;
    renderRun();
    if (!traceIsTerminal()) openStream(id);
    loadDiff(id);
  } catch (error) {
    $("main").innerHTML =
      `<div class="candidate-empty"><strong>Unable to load run</strong>${html(error.message)}<br><button id="retry-run-load" class="btn quiet" style="margin-top:12px">Retry</button></div>`;
    $("retry-run-load").onclick = () => loadRun(id);
  }
}
async function loadDiff(id) {
  try {
    state.diff = await api(`/api/runs/${id}/diff`);
    if (location.pathname === "/runs/" + id) renderCandidate();
  } catch {
    state.diff = null;
  }
}
function openStream(id) {
  closeStream();
  const source = new EventSource(`/api/runs/${id}/events`);
  state.eventSource = source;
  source.addEventListener("trace", (e) => {
    let fresh;
    try {
      fresh = JSON.parse(e.data);
    } catch {
      return;
    }
    if (!Array.isArray(fresh) || !fresh.length) return;
    const seen = new Set(state.events.map((x) => x.sequence));
    for (const event of fresh)
      if (!seen.has(event.sequence)) state.events.push(event);
    if (location.pathname === "/runs/" + id) {
      renderTrace();
      renderRunHead();
      if (fresh.some((x) => x.event?.type === "state_changed")) refresh();
      if (traceIsTerminal()) closeStream();
    }
  });
}
function traceIsTerminal() {
  const lastState = [...state.events]
    .reverse()
    .find((e) => e.event?.type === "state_changed")?.event?.data?.to;
  return ["completed", "applied", "discarded", "failed", "cancelled"].includes(
    lastState,
  );
}
function closeStream() {
  state.eventSource?.close();
  state.eventSource = null;
}
function selectedRun() {
  return (
    state.runs.find((r) => r.run_id === state.detail?.run_id) ||
    state.detail ||
    {}
  );
}
function activeJob() {
  return state.jobs.find(
    (j) =>
      (j.state === "running" || j.state === "cancelling") &&
      (j.output?.run_id === state.detail?.run_id ||
        j.goal === state.detail?.contract?.goal ||
        state.jobId === j.id),
  );
}
function usage() {
  let input = 0,
    output = 0,
    cached = 0,
    cost = null;
  for (const e of state.events) {
    const a = e.event?.type === "action_completed" ? e.event.data : null;
    if (!a || !String(a.actor).startsWith("model:")) continue;
    const at = a.attributes || {};
    input += Number(at.input_tokens || 0);
    output += Number(at.output_tokens || 0);
    cached += Number(at.cached_input_tokens || 0);
    if (at.cumulative_cost_microusd !== undefined)
      cost = Number(at.cumulative_cost_microusd);
  }
  const web = state.detail?.web_result;
  if (web?.cost_microusd != null) cost = Number(web.cost_microusd);
  return { input, output, cached, cost };
}
function renderRun() {
  const d = state.detail;
  if (!d) return;
  const main = $("main");
  main.className = "main";
  main.innerHTML = `<article class="run-page"><header class="run-head" id="run-head"></header><div class="compact-tabs" role="tablist" aria-label="Run panels"><button class="tab" data-view="trace">Trace</button><button class="tab" data-view="diff">Diff</button><button class="tab" data-view="evidence">Evidence</button><button class="tab" data-view="receipt">Receipt</button></div><div class="run-body" id="run-body"><section class="trace-panel" aria-label="Trace"><div class="panel-head"><span class="eyebrow">TRACE</span><span id="trace-count" class="count"></span><span class="panel-spacer"></span><span class="micro muted" id="trace-live">LIVE</span></div><div id="trace-filters" class="trace-filters"></div><div class="trace-scroll" id="trace-scroll"><div class="trace-list" id="trace-list" role="log" aria-live="polite" aria-relevant="additions"></div><button id="live-jump" class="live-jump" hidden></button></div></section><section class="candidate-panel" aria-label="Candidate"><div class="candidate-tabs" role="tablist" aria-label="Review tabs"><button class="tab" data-tab="diff">Diff</button><button class="tab" data-tab="evidence">Evidence</button><button class="tab" data-tab="receipt">Receipt</button></div><div class="candidate-scroll" id="candidate-scroll"></div></section></div></article>`;
  for (const b of main.querySelectorAll("[data-tab]"))
    b.onclick = () => {
      state.tab = b.dataset.tab;
      renderCandidate();
    };
  for (const b of main.querySelectorAll("[data-view]"))
    b.onclick = () => {
      state.compact = b.dataset.view;
      if (b.dataset.view !== "trace") state.tab = b.dataset.view;
      renderTabs();
      renderCandidate();
    };
  renderRunHead();
  renderTabs();
  renderTrace();
  renderCandidate();
  $("trace-scroll").onscroll = () => {
    const t = $("trace-scroll");
    state.following = t.scrollHeight - t.scrollTop - t.clientHeight < 80;
    if (state.following) {
      state.newEvents = 0;
      $("live-jump").hidden = true;
    }
  };
  $("live-jump").onclick = () => {
    state.following = true;
    state.newEvents = 0;
    $("live-jump").hidden = true;
    $("trace-scroll").scrollTop = $("trace-scroll").scrollHeight;
  };
}
function renderTabs() {
  const body = $("run-body");
  if (!body) return;
  body.dataset.view = state.compact;
  for (const b of document.querySelectorAll("[data-view]")) {
    b.classList.toggle("active", b.dataset.view === state.compact);
    b.setAttribute("aria-selected", String(b.dataset.view === state.compact));
  }
  for (const b of document.querySelectorAll("[data-tab]")) {
    b.classList.toggle("active", b.dataset.tab === state.tab);
    b.setAttribute("aria-selected", String(b.dataset.tab === state.tab));
  }
}
function renderRunHead() {
  const d = state.detail;
  if (!d || !$("run-head")) return;
  const r = selectedRun(),
    s = status(r),
    u = usage(),
    start = state.events[0]?.timestamp,
    job = activeJob();
  const latest = state.events.at(-1);
  const durationEnd = s.key === "running" ? null : latest?.timestamp;
  const failedJob = state.jobs.find(
    (j) => j.goal === (d.contract?.goal || r.goal) && j.state === "failed",
  );
  const summary = failedJob?.error
    ? `Failed: ${failedJob.error}`
    : latest?.event?.data?.summary || latest?.event?.data?.message || "";
  const model = state.events
    .map((e) => e.event?.data?.actor)
    .find((a) => String(a).startsWith("model:"))
    ?.replace(/^model:/, "");
  const head = $("run-head");
  head.innerHTML = `<div class="run-primary"><div class="run-ident"><div class="status-line"><span class="lamp ${s.key}"></span><span class="status-label">${html(s.label)}</span></div><h1 class="run-title" id="editable-title" tabindex="0" title="Edit run title (stored in this browser)">${html(runTitle(r))}</h1></div><div class="run-metrics">${model ? `<div class="metric"><small>MODEL</small><strong title="${html(model)}">${html(model.split("/").at(-1))}</strong></div>` : ""}${start ? `<div class="metric"><small>ELAPSED</small><strong id="elapsed">${html(elapsed(start, durationEnd))}</strong></div>` : ""}${u.input + u.output ? `<div class="metric"><small>TOKENS</small><strong>${(u.input + u.output).toLocaleString()}</strong></div>` : ""}${u.cost !== null ? `<div class="metric"><small>COST</small><strong>${price(u.cost)}</strong></div>` : ""}</div><div class="run-actions" id="run-actions">${job ? `<button class="btn quiet" id="stop-run" ${job.state === "cancelling" ? "disabled" : ""}>${icon("square")} ${job.state === "cancelling" ? "Stopping… waiting for the current tool call to finish" : "Stop"}</button>` : s.key === "ready" ? `<button class="btn green" id="apply-run">Apply</button><button class="btn quiet" id="discard-run">Discard</button>` : s.key === "failed" ? `<button class="btn quiet" id="retry-run">Retry</button><button class="btn quiet" id="edit-run">Edit prompt</button>` : ""}</div></div><div class="run-subline"><span class="mono">${html(state.bootstrap?.workspace || "")}</span><span class="mono">${html(d.run_id)}</span><button id="copy-run" title="Copy run ID" aria-label="Copy run ID">${icon("copy")}</button></div>${summary ? `<div class="run-current">current: ${html(summary)}</div>` : ""}`;
  const stop = $("stop-run");
  if (stop) stop.onclick = () => stopRun(job, stop);
  const apply = $("apply-run");
  if (apply) apply.onclick = () => confirmAction("apply");
  const discard = $("discard-run");
  if (discard) discard.onclick = () => confirmAction("discard");
  const retry = $("retry-run"),
    edit = $("edit-run");
  if (retry) retry.onclick = () => prefillRun(true);
  if (edit) edit.onclick = () => prefillRun(false);
  $("copy-run").onclick = () => copy(d.run_id, "Run ID copied");
  $("editable-title").onkeydown = (event) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      event.currentTarget.click();
    }
  };
  $("editable-title").onclick = () => {
    const input = node("input", "run-title run-title-input");
    input.setAttribute("aria-label", "Run title, stored in this browser");
    input.maxLength = 200;
    input.value = runTitle(r);
    $("editable-title").replaceWith(input);
    input.focus();
    input.select();
    let saved = false;
    const finish = (commit) => {
      if (saved) return;
      saved = true;
      if (commit) {
        if (input.value.trim()) state.titles[d.run_id] = input.value.trim();
        else delete state.titles[d.run_id];
        store("titles", state.titles);
      }
      renderRunHead();
      renderLedger();
      updateTitle();
    };
    input.onkeydown = (event) => {
      if (event.key === "Enter") finish(true);
      if (event.key === "Escape") finish(false);
    };
    input.onblur = () => finish(true);
  };
}
function prefillRun(dispatchNow) {
  state.prefill = state.detail?.contract?.goal || selectedRun().goal || "";
  navigate("/");
  $("task-goal")?.focus();
  if (dispatchNow) toast("Review the task and dispatch again.");
}
async function stopRun(job, button) {
  button.disabled = true;
  button.textContent = "Stopping… waiting for the current tool call to finish";
  try {
    await api(`/api/jobs/${job.id}/cancel`, { method: "POST" });
    toast("Stop requested. Waiting for the current tool call to finish.");
  } catch (e) {
    toast(e.message, true);
    button.disabled = false;
  }
}
function eventView(e) {
  const type = e.event?.type || "unknown",
    d = e.event?.data || {};
  if (type === "action_completed") {
    const actor = String(d.actor || "");
    return {
      kind: actor.startsWith("model:") ? "model" : "tool",
      summary: d.summary || d.action || actor,
      raw: JSON.stringify(d, null, 2),
      fail: !d.succeeded,
    };
  }
  if (type === "contract_registered")
    return {
      kind: "brief",
      summary: d.goal || "Task contract registered",
      raw: JSON.stringify(d, null, 2),
    };
  if (type === "state_changed")
    return {
      kind: "state",
      summary: `${d.from || ""} → ${d.to || ""}`,
      raw: null,
    };
  if (type === "evidence_recorded")
    return {
      kind: "check",
      summary: d.summary || d.kind || "Evidence recorded",
      raw: JSON.stringify(d, null, 2),
      fail: String(d.status).toLowerCase() === "failed",
    };
  if (type === "note_recorded")
    return { kind: "note", summary: d.message || "Note", raw: null };
  if (type === "effect_prepared")
    return {
      kind: "tool",
      summary: `${d.tool || "Tool"} prepared`,
      raw: JSON.stringify(d, null, 2),
    };
  if (type === "effect_completed")
    return {
      kind: "tool",
      summary: `Tool effect ${d.succeeded ? "completed" : "failed"}`,
      raw: JSON.stringify(d, null, 2),
      fail: !d.succeeded,
    };
  return {
    kind: type.replaceAll("_", " "),
    summary: type.replaceAll("_", " "),
    raw: JSON.stringify(d, null, 2),
  };
}
function renderTrace() {
  const list = $("trace-list");
  if (!list) return;
  const wasNear = state.following;
  const filtered = state.events.filter(
    (e) =>
      state.traceFilter === "all" || eventView(e).kind === state.traceFilter,
  );
  const filters = $("trace-filters");
  clear(filters);
  for (const k of ["all", "brief", "model", "tool", "check", "state", "note"]) {
    const b = node(
      "button",
      "filter" + (state.traceFilter === k ? " active" : ""),
      k === "all" ? "All" : k,
    );
    b.onclick = () => {
      state.traceFilter = k;
      store("traceFilter", k);
      renderTrace();
    };
    filters.append(b);
  }
  $("trace-count").textContent = `${filtered.length} events`;
  $("trace-live").textContent =
    status(selectedRun()).key === "running" ? "LIVE" : "RECORD";
  const oldCount = state.lastCount;
  clear(list);
  if (filtered.length > state.traceLimit) {
    const more = node(
      "button",
      "btn quiet",
      `Show earlier ${Math.min(500, filtered.length - state.traceLimit)} events`,
    );
    more.onclick = () => {
      state.traceLimit += 500;
      renderTrace();
    };
    list.append(more);
  }
  if (!filtered.length)
    list.append(node("p", "empty-note", "No trace events match this filter."));
  for (const e of filtered.slice(-state.traceLimit)) {
    const v = eventView(e),
      row = node(
        "div",
        "trace-row" +
          (v.fail ? " fail" : "") +
          (e.sequence >= oldCount ? " new" : ""),
      );
    row.id = "event-" + e.sequence;
    row.innerHTML = `<div class="trace-top"><time class="trace-time" title="${html(e.timestamp)}">${html(fmtTime(e.timestamp))}</time><span class="trace-kind">${html(v.kind)}</span></div><div class="trace-summary">${html(v.summary)}</div>`;
    if (v.raw) {
      const details = node("details", "trace-detail");
      details.open = !!v.fail;
      details.innerHTML = `<summary>Raw event</summary><pre>${html(v.raw)}</pre>`;
      row.append(details);
    }
    list.append(row);
  }
  state.lastCount = state.events.length;
  if (wasNear) {
    const scroll = $("trace-scroll");
    scroll.scrollTop = scroll.scrollHeight;
  } else if (state.events.length > oldCount) {
    state.newEvents += state.events.length - oldCount;
    const jump = $("live-jump");
    jump.hidden = false;
    jump.textContent = `${state.newEvents} new events — jump to live`;
  }
}
function renderCandidate() {
  const area = $("candidate-scroll");
  if (!area || !state.detail) return;
  renderTabs();
  if (state.tab === "diff") renderDiff(area);
  else if (state.tab === "evidence") renderEvidence(area);
  else renderReceipt(area);
}
function parseDiff(text) {
  const files = [];
  let current = null;
  for (const line of String(text || "").split("\n")) {
    if (line.startsWith("diff --git ")) {
      current = { header: [], lines: [], path: "" };
      files.push(current);
    }
    if (!current) {
      current = { header: [], lines: [], path: "" };
      files.push(current);
    }
    if (line.startsWith("+++ b/")) current.path = line.slice(6);
    if (!current.path && line.startsWith("diff --git ")) {
      const m = line.match(/ b\/(.+)$/);
      if (m) current.path = m[1];
    }
    current.lines.push(line);
  }
  return files.filter((f) => f.lines.some(Boolean));
}
function renderDiff(area) {
  const diff = state.diff;
  if (!diff) {
    area.innerHTML =
      '<div class="candidate-empty"><strong>Candidate forming</strong>The diff becomes available when the engine records a receipt.</div>';
    return;
  }
  const changes = diff.changes || [];
  if (!changes.length) {
    area.innerHTML =
      '<div class="candidate-empty"><strong>No file changes</strong>The agent answered in the trace without editing files.</div>';
    return;
  }
  const files = parseDiff(diff.unified_diff);
  const counts = new Map();
  for (const f of files) {
    let add = 0,
      del = 0;
    for (const l of f.lines) {
      if (l.startsWith("+") && !l.startsWith("+++")) add++;
      if (l.startsWith("-") && !l.startsWith("---")) del++;
    }
    counts.set(f.path, { add, del });
  }
  if (
    !state.selectedFile ||
    !changes.some((c) => c.path === state.selectedFile)
  )
    state.selectedFile = changes[0].path;
  const selected = changes.find((c) => c.path === state.selectedFile),
    file = files.find((f) => f.path === state.selectedFile);
  const total = { add: 0, del: 0 };
  const verdict = status(selectedRun());
  for (const c of counts.values()) {
    total.add += c.add;
    total.del += c.del;
  }
  area.innerHTML = `<div class="diff-toolbar"><span class="eyebrow">${changes.length} FILE${changes.length === 1 ? "" : "S"} · <span class="mono">+${total.add} −${total.del}</span>${verdict.key === "ready" ? "" : " · " + html(verdict.label.toUpperCase())}</span><div class="segmented"><button data-mode="unified" class="${state.diffMode === "unified" ? "active" : ""}">Unified</button><button data-mode="split" class="${state.diffMode === "split" ? "active" : ""}">Split</button></div></div><div class="diff-layout"><nav class="file-tree" aria-label="Changed files"><div class="file-tree-head eyebrow">FILES</div></nav><div class="diff-code"><div class="file-head"><span>${selected.before_digest ? "M" : selected.after_digest ? "A" : "D"}</span><span class="path" title="${html(selected.path)}">${html(selected.path)}</span><button id="copy-path" aria-label="Copy file path">${icon("copy")}</button></div><div id="diff-lines"></div></div></div>`;
  const tree = area.querySelector(".file-tree");
  let dir = "";
  for (const c of changes) {
    const parts = c.path.split("/");
    const next = parts.length > 1 ? parts.slice(0, -1).join("/") : "/";
    if (next !== dir) {
      dir = next;
      tree.append(node("div", "file-dir", dir));
    }
    const n = counts.get(c.path) || { add: 0, del: 0 },
      button = node(
        "button",
        "file-entry" + (c.path === state.selectedFile ? " active" : ""),
      );
    button.innerHTML = `<span>${c.before_digest ? "M" : c.after_digest ? "A" : "D"}</span><span class="path" title="${html(c.path)}">${html(parts.at(-1))}</span><span class="delta">+${n.add} −${n.del}</span>`;
    button.onclick = () => {
      state.selectedFile = c.path;
      state.diffLineLimit = 500;
      renderDiff(area);
    };
    tree.append(button);
  }
  for (const b of area.querySelectorAll("[data-mode]"))
    b.onclick = () => {
      state.diffMode = b.dataset.mode;
      store("diffMode", state.diffMode);
      renderDiff(area);
    };
  $("copy-path").onclick = () => copy(selected.path, "Path copied");
  const lines = area.querySelector("#diff-lines");
  if (!file) {
    lines.append(
      node(
        "div",
        "candidate-empty",
        "The engine recorded a changed file, but this diff has no patch for it.",
      ),
    );
    return;
  }
  const forceUnified = matchMedia("(max-width:719px)").matches;
  const visibleLines = file.lines.slice(0, state.diffLineLimit);
  const paired = diffPairs(visibleLines);
  if (state.diffMode === "split" && !forceUnified) {
    const split = node("div", "split-diff");
    for (let i = 0; i < visibleLines.length; ) {
      const start = i;
      const removed = [];
      const added = [];
      while (
        visibleLines[i]?.startsWith("-") &&
        !visibleLines[i].startsWith("---")
      )
        removed.push([i, visibleLines[i++]]);
      while (
        visibleLines[i]?.startsWith("+") &&
        !visibleLines[i].startsWith("+++")
      )
        added.push([i, visibleLines[i++]]);
      if (i > start) {
        for (let j = 0; j < Math.max(removed.length, added.length); j++) {
          split.append(
            removed[j]
              ? diffLine(removed[j][1], paired.get(removed[j][0]))
              : diffLine(" "),
            added[j]
              ? diffLine(added[j][1], paired.get(added[j][0]))
              : diffLine(" "),
          );
        }
      } else {
        split.append(diffLine(visibleLines[i]), diffLine(visibleLines[i]));
        i++;
      }
    }
    lines.append(split);
  } else {
    const block = node("div", "diff-lines");
    for (let i = 0; i < visibleLines.length; i++)
      block.append(diffLine(visibleLines[i], paired.get(i)));
    lines.append(block);
  }
  if (file.lines.length > state.diffLineLimit) {
    const more = node(
      "button",
      "btn quiet diff-more",
      `Show next ${Math.min(500, file.lines.length - state.diffLineLimit)} lines`,
    );
    more.onclick = () => {
      const scrollTop = area.scrollTop;
      state.diffLineLimit += 500;
      renderDiff(area);
      area.scrollTop = scrollTop;
    };
    lines.append(more);
  }
}
function diffPairs(lines) {
  const result = new Map();
  for (let i = 0; i < lines.length; ) {
    const removed = [],
      added = [];
    while (lines[i]?.startsWith("-") && !lines[i].startsWith("---"))
      removed.push(i++);
    while (lines[i]?.startsWith("+") && !lines[i].startsWith("+++"))
      added.push(i++);
    for (let j = 0; j < Math.min(removed.length, added.length); j++) {
      result.set(removed[j], lines[added[j]].slice(1));
      result.set(added[j], lines[removed[j]].slice(1));
    }
    if (!removed.length && !added.length) i++;
  }
  return result;
}
function wordDiff(text, other) {
  if (!other) return html(text);
  let prefix = 0,
    suffix = 0;
  while (
    prefix < text.length &&
    prefix < other.length &&
    text[prefix] === other[prefix]
  )
    prefix++;
  while (
    suffix < text.length - prefix &&
    suffix < other.length - prefix &&
    text[text.length - suffix - 1] === other[other.length - suffix - 1]
  )
    suffix++;
  if (prefix + suffix === text.length) return html(text);
  return (
    html(text.slice(0, prefix)) +
    '<span class="word">' +
    html(text.slice(prefix, text.length - suffix)) +
    "</span>" +
    html(text.slice(text.length - suffix))
  );
}
function diffLine(line, other) {
  const cls = line.startsWith("@@")
    ? "hunk"
    : line.startsWith("+++") ||
        line.startsWith("---") ||
        line.startsWith("diff --git")
      ? "file-marker"
      : line.startsWith("+")
        ? "add"
        : line.startsWith("-")
          ? "del"
          : "";
  const e = node("div", "diff-line " + cls);
  e.innerHTML = `<span class="gutter">${html(line[0] || " ")}</span><span class="text">${cls === "add" || cls === "del" ? wordDiff(line.slice(1), other) : html(line.slice(1))}</span>`;
  return e;
}
function renderEvidence(area) {
  clear(area);
  const evidence = state.detail?.evidence || [];
  if (!evidence.length) {
    area.append(
      node("div", "candidate-empty", "No evidence was recorded for this run."),
    );
    return;
  }
  const list = node("div", "evidence-list");
  for (const ev of evidence) {
    const card = node("article", "evidence-item"),
      status = String(ev.status || "").toLowerCase();
    card.innerHTML = `<div class="evidence-top"><span class="tag">${html(ev.kind || "EVIDENCE")}</span><strong>${html(ev.summary || ev.kind || "Evidence")}</strong><span class="evidence-status ${status.includes("pass") ? "pass" : status.includes("fail") ? "fail" : ""}">${html(ev.status || "recorded")}</span></div>${ev.reproduction ? `<p class="mono">${html(ev.reproduction)}</p>` : ""}`;
    const d = node("details");
    d.innerHTML = `<summary>Raw evidence</summary><pre class="raw-output">${html(JSON.stringify(ev, null, 2))}</pre>`;
    card.append(d);
    const origin = state.events.find(
      (e) =>
        e.event?.type === "evidence_recorded" &&
        e.event?.data?.summary === ev.summary,
    );
    if (origin) {
      const link = node("button", "btn quiet", "Open in trace →");
      link.style.marginTop = "8px";
      link.onclick = () => {
        state.traceFilter = "all";
        state.traceLimit = Math.max(500, state.events.length - origin.sequence);
        state.compact = "trace";
        renderTabs();
        renderTrace();
        document
          .getElementById("event-" + origin.sequence)
          ?.scrollIntoView({ block: "center", behavior: "smooth" });
      };
      card.append(link);
    }
    list.append(card);
  }
  area.append(list);
}
function receiptRow(label, value) {
  return `<div class="receipt-row"><dt>${html(label)}</dt><dd>${html(value)}</dd></div>`;
}
function renderReceipt(area) {
  const d = state.detail,
    u = usage(),
    r = selectedRun(),
    s = status(r),
    date = state.events[0]?.timestamp || r.created_at;
  const receipt = d.contract ? d : null;
  area.innerHTML = `<div class="receipt"><div class="receipt-head"><span class="eyebrow">PACTRAIL / CHANGE RECEIPT</span><h2>${html(receipt?.contract?.goal || r.goal || "Run in progress")}</h2><span class="muted small">${html(s.label)}</span></div><section class="receipt-section"><h3>IDENTITY</h3><dl>${receiptRow("Run ID", d.run_id)}${date ? receiptRow("Dispatched", `${fmtDate(date)} · ${fmtTime(date)}`) : ""}${state.bootstrap?.version ? receiptRow("Engine", `v${state.bootstrap.version}`) : ""}${
    state.events
      .map((e) => e.event?.data?.actor)
      .find((a) => String(a).startsWith("model:"))
      ? receiptRow(
          "Model",
          state.events
            .map((e) => e.event?.data?.actor)
            .find((a) => String(a).startsWith("model:"))
            .replace(/^model:/, ""),
        )
      : ""
  }${state.bootstrap?.workspace ? receiptRow("Workspace", state.bootstrap.workspace) : ""}${receipt?.baseline_digest ? receiptRow("Baseline digest", short(receipt.baseline_digest)) : ""}${receipt?.integrity_hash ? receiptRow("Receipt integrity", short(receipt.integrity_hash)) : ""}</dl></section><section class="receipt-section"><h3>USAGE</h3><dl>${u.input ? receiptRow("Input tokens", u.input.toLocaleString()) : ""}${u.output ? receiptRow("Output tokens", u.output.toLocaleString()) : ""}${u.cached ? receiptRow("Cached input tokens", u.cached.toLocaleString()) : ""}${u.cost != null ? `<div class="receipt-row"><dt>Total cost</dt><dd class="receipt-total">${price(u.cost)}</dd></div>` : '<div class="settings-note">Pricing not set. Token usage appears when reported by the engine.</div>'}</dl></section>${receipt ? `<section class="receipt-section"><h3>CHANGES & CHECKS</h3><dl>${receiptRow("Files changed", receipt.changes?.length || 0)}${receiptRow("Checks passed", receipt.verification?.passed || 0)}${receiptRow("Checks failed", receipt.verification?.failed || 0)}${receiptRow("Inconclusive", receipt.verification?.inconclusive || 0)}</dl></section>` : ""}<div class="verdict ${s.key}">${html(s.label.toUpperCase())}</div>${receipt?.contract?.goal ? `<section class="receipt-section"><h3>ORIGINAL TASK / THE PACT</h3><p class="receipt-pact">${html(receipt.contract.goal)}</p></section>` : ""}<div class="receipt-actions"><button class="btn quiet" id="copy-markdown">Copy as Markdown</button><button class="btn quiet" id="copy-json">Copy as JSON</button></div></div>`;
  $("copy-json").onclick = (event) =>
    copyInline(event.currentTarget, JSON.stringify(d, null, 2));
  $("copy-markdown").onclick = (event) =>
    copyInline(event.currentTarget, receiptMarkdown(d, u, s));
}
async function copyInline(button, value) {
  const label = button.textContent;
  try {
    await navigator.clipboard.writeText(value);
    button.textContent = "Copied";
    setTimeout(() => {
      if (button.isConnected) button.textContent = label;
    }, 1800);
  } catch {
    toast("Clipboard unavailable", true);
  }
}
function receiptMarkdown(d, u, s) {
  return `# Pactrail receipt\n\n- Run: ${d.run_id}\n- Outcome: ${s.label}\n- Workspace: ${state.bootstrap?.workspace || ""}\n- Input tokens: ${u.input}\n- Output tokens: ${u.output}${u.cost != null ? `\n- Cost: ${price(u.cost)}` : ""}\n\n## Task\n\n${d.contract?.goal || ""}\n\n## Changes\n\n${(d.changes || []).map((c) => "- " + c.path).join("\n")}\n`;
}
async function copy(value, message) {
  try {
    await navigator.clipboard.writeText(value);
    toast(message);
  } catch {
    toast("Clipboard unavailable", true);
  }
}
function confirmAction(action) {
  const d = $("confirm-dialog"),
    count = state.detail?.changes?.length || 0;
  $("dialog-kicker").textContent =
    action === "apply" ? "APPLY CANDIDATE" : "DISCARD CANDIDATE";
  $("dialog-title").textContent =
    action === "apply"
      ? `Apply ${count} file${count === 1 ? "" : "s"} to the working tree?`
      : "Discard these changes?";
  $("dialog-copy").textContent =
    action === "apply"
      ? "Pactrail will verify the candidate and apply its changes to the working tree."
      : "The isolated candidate and its changes will be deleted. The trace and receipt are kept. This cannot be undone.";
  $("dialog-confirm").textContent =
    action === "apply" ? "Apply changes" : "Discard changes";
  $("dialog-confirm").className =
    "btn " + (action === "apply" ? "green" : "solid");
  d.onclose = async () => {
    if (d.returnValue !== "confirm") return;
    try {
      await api(`/api/runs/${state.detail.run_id}/${action}`, {
        method: "POST",
      });
      toast(action === "apply" ? "Changes applied." : "Candidate discarded.");
      await refresh();
      await loadRun(state.detail.run_id);
    } catch (e) {
      toast(e.message, true);
    }
  };
  d.showModal();
}
function renderRecentOnly() {
  const goal = $("task-goal")?.value,
    title = $("task-title")?.value,
    model = $("model")?.value,
    provider = $("provider")?.value;
  if (!goal && state.bootstrap && !$("model")?.value) {
    renderDispatch();
    return;
  }
  const recent = $("recent-list");
  if (!recent) return;
  clear(recent);
  if (!state.runs.length)
    recent.append(
      node(
        "p",
        "empty-note",
        "No runs yet. Dispatch your first task above — everything runs in an isolated workspace until you apply it.",
      ),
    );
  for (const r of state.runs.slice(0, 5)) {
    const st = status(r),
      a = node("a", "recent-item");
    a.href = "/runs/" + r.run_id;
    a.dataset.link = "";
    a.innerHTML = `<span class="lamp ${st.key}"></span><span class="title">${html(runTitle(r))}</span><span class="meta">${html(st.label)}</span>`;
    recent.append(a);
  }
  if (state.bootstrap && $("model") && !model) {
    $("model").value = state.bootstrap.defaults?.model || "";
    $("provider").value = state.bootstrap.defaults?.provider || provider;
  }
  if (goal !== undefined) $("task-goal").value = goal;
  if (title !== undefined) $("task-title").value = title;
}
function renderSettings() {
  const main = $("main");
  main.className = "main";
  main.innerHTML = `<section class="settings-page"><div class="eyebrow muted">LOCAL CONFIGURATION</div><h1>Settings</h1><section class="settings-section"><h2>Engine</h2><div class="settings-row"><span>Connection</span><span>${state.offline ? "Unreachable" : "Connected to localhost"}</span></div><div class="settings-row"><span>Version</span><span>${html(state.bootstrap?.version || "—")}</span></div><div class="settings-row"><span>Workspace</span><span>${html(state.bootstrap?.workspace || "—")}</span></div></section><section class="settings-section"><h2>Run defaults</h2><p class="settings-note">These values come from this local engine session. Edit them for each run on the dispatch screen.</p><div class="settings-row"><span>Provider</span><span>${html(state.bootstrap?.defaults?.provider || "—")}</span></div><div class="settings-row"><span>Model</span><span>${html(state.bootstrap?.defaults?.model || "Not configured")}</span></div><div class="settings-row"><span>Base URL</span><span>${html(state.bootstrap?.defaults?.base_url || "Provider default")}</span></div></section><section class="settings-section"><h2>Appearance</h2><div class="settings-row"><label for="theme-choice">Theme</label><select class="select" id="theme-choice"><option value="system">System</option><option value="paper">Paper</option><option value="carbon">Carbon</option></select></div><div class="settings-row"><label for="diff-choice">Diff view</label><select class="select" id="diff-choice"><option value="unified">Unified</option><option value="split">Split</option></select></div></section><section class="settings-section"><h2>Provider configuration</h2><p class="settings-note">Provider keys and model pricing are configured in the local CLI environment. The browser never receives secret values. Cost appears only when the engine reports it.</p></section></section>`;
  $("theme-choice").value = state.theme || "system";
  $("theme-choice").onchange = (e) =>
    setTheme(e.target.value === "system" ? null : e.target.value);
  $("diff-choice").value = state.diffMode;
  $("diff-choice").onchange = (e) => {
    state.diffMode = e.target.value;
    store("diffMode", state.diffMode);
  };
}
function openLedger() {
  $("ledger").classList.add("open");
  $("ledger-shade").hidden = false;
  $("ledger-search").focus();
}
function closeLedger() {
  $("ledger").classList.remove("open");
  $("ledger-shade").hidden = true;
}
function updateTitle() {
  let title = "Pactrail",
    color = "#1f6d45";
  if (state.detail) {
    const s = status(selectedRun()),
      start = state.events[0]?.timestamp;
    title = `● ${start && s.key === "running" ? elapsed(start) + " — " : ""}${runTitle(selectedRun()).slice(0, 60)} · Pactrail`;
    color =
      {
        running: "#e89b0c",
        ready: "#1f6d45",
        applied:
          state.theme === "carbon" ||
          (state.theme === null &&
            matchMedia("(prefers-color-scheme: dark)").matches)
            ? "#e8e2d4"
            : "#1a1712",
        discarded: "#9a917d",
        failed: "#b03426",
        cancelled: "#b03426",
      }[s.key] || color;
  } else if (location.pathname === "/settings") title = "Settings · Pactrail";
  document.title = title;
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><circle cx="16" cy="16" r="8" fill="${color}"/></svg>`;
  let link = document.querySelector('link[rel="icon"]');
  if (!link) {
    link = node("link");
    link.rel = "icon";
    document.head.append(link);
  }
  link.href = "data:image/svg+xml," + encodeURIComponent(svg);
}
function syncDispatchAvailability() {
  const form = $("dispatch-form");
  if (!form) return;
  for (const control of form.querySelectorAll("input,textarea,select"))
    control.disabled = state.offline;
  const busy = state.jobs.some((job) =>
    ["running", "cancelling"].includes(job.state),
  );
  const button = $("dispatch-button");
  button.disabled = state.offline || busy || Boolean(state.pendingGoal);
  button.innerHTML = state.pendingGoal
    ? "Dispatching…"
    : busy
      ? "Run in progress"
      : `${icon("arrow-right")} Dispatch run`;
}
async function refresh() {
  if (state.loading) return;
  state.loading = true;
  try {
    if (!state.bootstrap) state.bootstrap = await api("/api/bootstrap");
    const [runs, jobs] = await Promise.all([
      api("/api/runs"),
      api("/api/jobs"),
    ]);
    state.runs = Array.isArray(runs)
      ? runs.sort((a, b) => b.run_id.localeCompare(a.run_id))
      : [];
    state.jobs = Array.isArray(jobs) ? jobs : [];
    state.offline = false;
    state.lastOfflineToast = false;
    $("offline").hidden = true;
    document.querySelector(".wordmark .lamp").className = "lamp connected";
    $("engine-indicator").innerHTML =
      `<span class="lamp connected"></span>Engine v${html(state.bootstrap.version)} · connected`;
    if (state.pendingGoal) {
      const found = state.runs.find(
        (r) => r.goal === state.pendingGoal && !state.pendingIds?.has(r.run_id),
      );
      if (found) {
        if (state.pendingTitle) {
          state.titles[found.run_id] = state.pendingTitle;
          store("titles", state.titles);
        }
        state.pendingGoal = null;
        state.pendingTitle = null;
        navigate("/runs/" + found.run_id);
      } else {
        const pendingJob = state.jobs.find((job) => job.id === state.jobId);
        if (pendingJob && ["failed", "cancelled"].includes(pendingJob.state)) {
          state.pendingGoal = null;
          const error = $("dispatch-error");
          if (error) {
            error.textContent = pendingJob.error || "Run could not be started.";
            error.hidden = false;
          }
        }
      }
    }
    if (location.pathname === "/") {
      if (!document.querySelector("#dispatch-form")) renderDispatch();
      else renderRecentOnly();
    } else if (location.pathname === "/settings") renderSettings();
    else if (state.detail) {
      const r = state.runs.find((x) => x.run_id === state.detail.run_id);
      if (r && status(r).key !== status(state.detail).key) {
        state.detail = await api(`/api/runs/${r.run_id}/inspect`);
        loadDiff(r.run_id);
      }
      renderRunHead();
    }
    renderLedger();
    syncDispatchAvailability();
    updateTitle();
  } catch (e) {
    if (e.network) {
      state.offline = true;
      $("offline").hidden = false;
      document.querySelector(".wordmark .lamp").className = "lamp offline";
      $("engine-indicator").innerHTML =
        '<span class="lamp offline"></span>Engine unreachable';
      if (!state.lastOfflineToast) {
        toast("Engine unreachable. Retrying in 5s.", true);
        state.lastOfflineToast = true;
      }
      syncDispatchAvailability();
    } else toast(e.message, true);
  } finally {
    state.loading = false;
  }
}
function onKey(e) {
  const target = e.target;
  const typing = target.closest("input,textarea,select,[contenteditable]");
  if (e.key === "Escape") {
    closeLedger();
    return;
  }
  if (
    (e.metaKey || e.ctrlKey) &&
    e.key === "Enter" &&
    location.pathname === "/"
  ) {
    e.preventDefault();
    $("dispatch-form")?.requestSubmit();
    return;
  }
  if (typing || e.metaKey || e.ctrlKey || e.altKey) return;
  if (e.key === "?") {
    $("help-dialog").showModal();
    return;
  }
  if (e.key === "n") {
    e.preventDefault();
    navigate("/");
    $("task-goal")?.focus();
  } else if (e.key === "/") {
    e.preventDefault();
    openLedger();
  } else if (e.key === "j" || e.key === "k") {
    e.preventDefault();
    const i = state.runs.findIndex(
        (r) => location.pathname === "/runs/" + r.run_id,
      ),
      next = Math.max(
        0,
        Math.min(state.runs.length - 1, i + (e.key === "j" ? 1 : -1)),
      );
    if (state.runs[next]) navigate("/runs/" + state.runs[next].run_id);
  } else if (location.pathname.startsWith("/runs/")) {
    if (["1", "2", "3"].includes(e.key)) {
      e.preventDefault();
      state.compact = { 1: "trace", 2: "diff", 3: "receipt" }[e.key];
      if (state.compact !== "trace") state.tab = state.compact;
      renderTabs();
      renderCandidate();
    } else if (e.key === "e" && $("apply-run")) {
      e.preventDefault();
      confirmAction("apply");
    } else if (e.key === "x" && $("discard-run")) {
      e.preventDefault();
      confirmAction("discard");
    }
  }
}
function interfaceError(error) {
  const message = String(error?.message || error || "Unknown interface error");
  closeStream();
  const main = $("main");
  main.innerHTML = `<div class="candidate-empty"><strong>Something broke in the interface.</strong>The engine and your runs are unaffected.<div class="receipt-actions"><button class="btn quiet" id="copy-error">Copy error</button><button class="btn solid" id="reload-ui">Reload</button></div></div>`;
  $("copy-error").onclick = () => copy(message, "Error copied");
  $("reload-ui").onclick = () => location.reload();
}
window.addEventListener("error", (event) => {
  if (event.error) interfaceError(event.error);
});
window.addEventListener("unhandledrejection", (event) =>
  interfaceError(event.reason),
);
window.addEventListener("popstate", route);
document.addEventListener("click", (e) => {
  const a = e.target.closest("a[data-link]");
  if (a && a.origin === location.origin) {
    e.preventDefault();
    navigate(a.pathname);
  }
});
document.addEventListener("keydown", onKey);
$("ledger-open").onclick = openLedger;
$("ledger-collapse").onclick = () => {
  state.ledgerCollapsed = !state.ledgerCollapsed;
  store("ledgerCollapsed", state.ledgerCollapsed);
  $("app").classList.toggle("ledger-collapsed", state.ledgerCollapsed);
  $("ledger-collapse").setAttribute(
    "aria-label",
    state.ledgerCollapsed ? "Expand ledger" : "Collapse ledger",
  );
  $("ledger-collapse").innerHTML = icon(
    state.ledgerCollapsed ? "panel-left-open" : "panel-left-close",
  );
};
$("ledger-close").onclick = closeLedger;
$("ledger-shade").onclick = closeLedger;
$("ledger-search").oninput = (e) => {
  state.search = e.target.value;
  renderLedger();
};
$("help-toggle").onclick = () => $("help-dialog").showModal();
$("retry-now").onclick = refresh;
$("theme-toggle").onclick = () =>
  setTheme(state.theme === "carbon" ? "paper" : "carbon");
setTheme(state.theme);
$("app").classList.toggle("ledger-collapsed", state.ledgerCollapsed);
$("ledger-collapse").setAttribute(
  "aria-label",
  state.ledgerCollapsed ? "Expand ledger" : "Collapse ledger",
);
$("ledger-collapse").innerHTML = icon(
  state.ledgerCollapsed ? "panel-left-open" : "panel-left-close",
);
route();
refresh();
setInterval(refresh, 5000);
setInterval(() => {
  if (state.detail) {
    const el = $("elapsed");
    if (el && state.events[0])
      el.textContent = elapsed(
        state.events[0].timestamp,
        status(selectedRun()).key === "running"
          ? null
          : state.events.at(-1)?.timestamp,
      );
    updateTitle();
  }
}, 1000);
