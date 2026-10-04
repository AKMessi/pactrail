"use strict";
/* Pure projections are shared by the browser and dependency-free Node tests. */
const PRICE_FIELDS = ["input", "cached_input", "cache_creation", "output"];
const TERMINAL = new Set([
  "completed",
  "applied",
  "discarded",
  "failed",
  "cancelled",
]);
const PHASES = [
  "Contract",
  "Investigate",
  "Plan",
  "Execute",
  "Verify",
  "Review",
  "Decision",
];
const PHASE_STATE = {
  created: 0,
  contracting: 0,
  investigating: 1,
  planning: 2,
  executing: 3,
  verifying: 4,
  reviewing: 5,
  awaiting_apply: 6,
  applied: 6,
  discarded: 6,
  completed: 6,
};
const WARNING_ACTIONS = new Set([
  "steer_implementation",
  "reject_unavailable_tool",
]);
const WARNING_TEXT = /stopped|intervened|steered|rejected|recovery/i;
function numeric(value) {
  if (
    value === null ||
    value === undefined ||
    (typeof value === "string" && value.trim() === "") ||
    typeof value === "boolean"
  )
    return null;
  const n = Number(value);
  return Number.isFinite(n) && n >= 0 ? n : null;
}
function formatUsage(value, { kind = "number", coverage, reason } = {}) {
  const n = numeric(value);
  if (n === null)
    return {
      text: "—",
      state: "absent",
      reason: reason || "Not reported by the engine.",
    };
  let text;
  if (kind === "cost") text = "$" + (n / 1e6).toFixed(n < 1e6 ? 4 : 2);
  else if (kind === "duration") {
    if (n < 1) text = n.toFixed(1) + " s";
    else if (n < 60) text = Math.floor(n) + " s";
    else if (n < 3600)
      text = `${Math.floor(n / 60)}:${String(Math.floor(n % 60)).padStart(2, "0")}`;
    else
      text = `${Math.floor(n / 3600)}:${String(Math.floor(n / 60) % 60).padStart(2, "0")}:${String(Math.floor(n % 60)).padStart(2, "0")}`;
  } else if (kind === "tokens" && n >= 100000)
    text = (n / 1000).toFixed(1) + "k";
  else text = n.toLocaleString("en-US", { maximumFractionDigits: 2 });
  const partial = coverage && coverage.reported < coverage.total;
  return {
    text: (partial ? "≥ " : "") + text,
    state: partial ? "partial" : "measured",
    reason: partial
      ? `Reported for ${coverage.reported} of ${coverage.total} turns.`
      : reason || "Measured by the engine.",
  };
}
function pricingToEngine(card, maxCost = "") {
  const fields = PRICE_FIELDS.filter((k) => numeric(card?.[k]) !== null);
  if (fields.length && fields.length !== 4)
    throw Error("Provide all four model prices or none");
  const body = {};
  for (const field of fields) {
    const n = Number(card[field]);
    if (Math.abs(n * 1e6 - Math.round(n * 1e6)) > 0.0000001)
      throw Error("Use at most six decimal places");
    if (!Number.isSafeInteger(Math.round(n * 1e6)))
      throw Error("Price is too large");
    body[field + "_price"] = Math.round(n * 1e6);
  }
  if (String(maxCost).trim() !== "") {
    const cost = numeric(maxCost);
    if (cost === null || !Number.isSafeInteger(Math.round(cost * 1e6)))
      throw Error("Enter a nonnegative cost with at most six decimal places");
    if (cost > 0 && fields.length !== 4)
      throw Error("A maximum cost requires all four model prices");
    body.max_cost_microusd = Math.round(cost * 1e6);
  }
  return body;
}
function actions(events) {
  return events.filter((e) => e.event?.type === "action_completed");
}
function freshness(events) {
  const writes = actions(events).filter((e) =>
    (e.event.data.observed_effects || []).some((x) =>
      /^fs\.(changed|write|remove):/.test(x),
    ),
  );
  const evidence = events.filter((e) => e.event?.type === "evidence_recorded");
  const lastWrite = writes.length
    ? Math.max(...writes.map((e) => e.sequence))
    : null;
  const lastEvidence = evidence.length
    ? Math.max(...evidence.map((e) => e.sequence))
    : null;
  return {
    stale:
      lastWrite !== null && lastEvidence !== null && lastWrite > lastEvidence,
    lastWrite,
    lastEvidence,
  };
}
function provenance(events, path) {
  let turn = null;
  const found = [];
  for (const e of events) {
    const a = e.event?.type === "action_completed" ? e.event.data : null;
    if (!a) continue;
    if (a.actor?.startsWith("model:") && a.action === "invoke")
      turn = numeric(a.attributes?.turn);
    if (
      (a.observed_effects || []).some(
        (x) => x === `fs.changed:${path}` || x === `fs.write:${path}`,
      )
    )
      found.push({ sequence: e.sequence, turn });
  }
  return {
    actions: found.length,
    turns: [...new Set(found.map((x) => x.turn).filter((x) => x !== null))],
    sequences: found.map((x) => x.sequence),
  };
}
function parseUnifiedDiff(text, changes = []) {
  const files = [];
  let file = null,
    hunk = null,
    old = 0,
    next = 0;
  const cleanPath = (s) => s.replace(/^[ab]\//, "").replace(/\t.*$/, "");
  for (const raw of String(text || "").split("\n")) {
    if (raw.startsWith("diff --git ")) {
      file = null;
      hunk = null;
      continue;
    }
    if (
      raw.startsWith("--- ") &&
      (!hunk ||
        (old >= hunk.oldStart + hunk.oldCount &&
          next >= hunk.newStart + hunk.newCount))
    ) {
      file = {
        before: cleanPath(raw.slice(4)),
        path: "",
        hunks: [],
        added: 0,
        removed: 0,
      };
      files.push(file);
      hunk = null;
      continue;
    }
    if (raw.startsWith("+++ ") && file && !hunk) {
      file.path =
        raw.slice(4) === "/dev/null" ? file.before : cleanPath(raw.slice(4));
      continue;
    }
    const match = raw.match(/^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(.*)$/);
    if (match && file) {
      old = Number(match[1]);
      next = Number(match[3]);
      hunk = {
        header: raw,
        oldStart: old,
        newStart: next,
        oldCount: Number(match[2] ?? 1),
        newCount: Number(match[4] ?? 1),
        lines: [],
      };
      file.hunks.push(hunk);
      continue;
    }
    if (!hunk) continue;
    const sign = raw[0];
    if (![" ", "+", "-", "\\"].includes(sign)) continue;
    if (sign === "\\") {
      hunk.lines.push({ kind: "note", text: raw, old: null, new: null });
      continue;
    }
    const kind = sign === "+" ? "add" : sign === "-" ? "del" : "context";
    hunk.lines.push({
      kind,
      text: raw.slice(1),
      old: sign === "+" ? null : old++,
      new: sign === "-" ? null : next++,
    });
    if (kind === "add") file.added++;
    if (kind === "del") file.removed++;
  }
  for (const change of changes) {
    let f = files.find((f) => f.path === change.path);
    if (!f) {
      f = { path: change.path, hunks: [], added: 0, removed: 0 };
      files.push(f);
    }
    f.change = change;
    f.status =
      change.before_digest == null
        ? "A"
        : change.after_digest == null
          ? "D"
          : "M";
    f.modeChanged = change.before_unix_mode !== change.after_unix_mode;
  }
  return files;
}
function usageProjection(events) {
  const turns = actions(events).filter(
    (e) =>
      e.event.data.actor?.startsWith("model:") &&
      e.event.data.action === "invoke",
  );
  let total = 0,
    reported = 0,
    input = 0,
    output = 0,
    cached = 0,
    creation = 0;
  const counts = { input: 0, output: 0, cached: 0, creation: 0 };
  for (const e of turns) {
    const a = e.event.data.attributes || {};
    const i = numeric(a.input_tokens),
      o = numeric(a.output_tokens);
    if (i !== null || o !== null) total += (i ?? 0) + (o ?? 0);
    if (i !== null && o !== null) reported++;
    for (const [key, field] of [
      ["input", "input_tokens"],
      ["output", "output_tokens"],
      ["cached", "cached_input_tokens"],
      ["creation", "cache_creation_input_tokens"],
    ]) {
      const n = numeric(a[field]);
      if (n !== null) {
        counts[key]++;
        if (key === "input") input += n;
        if (key === "output") output += n;
        if (key === "cached") cached += n;
        if (key === "creation") creation += n;
      }
    }
  }
  return {
    turns: turns.length,
    total: counts.input + counts.output ? total : null,
    reported,
    input,
    output,
    cached,
    creation,
    counts,
  };
}
function evidenceSummary(evidence) {
  const counts = { passed: 0, failed: 0, inconclusive: 0, skipped: 0 };
  const grades = ["unverified", "model_assessed", "observed", "deterministic"];
  let grade = null;
  for (const e of evidence || []) {
    if (e.status in counts) counts[e.status]++;
    if (grades.indexOf(e.grade) > grades.indexOf(grade)) grade = e.grade;
  }
  return {
    ...counts,
    grade,
    deterministicPass: (evidence || []).some(
      (e) => e.grade === "deterministic" && e.status === "passed",
    ),
  };
}
if (typeof module !== "undefined")
  module.exports = {
    formatUsage,
    pricingToEngine,
    parseUnifiedDiff,
    freshness,
    provenance,
    usageProjection,
    evidenceSummary,
  };
if (typeof document !== "undefined") {
  const $ = (id) => document.getElementById(id);
  function el(tag, cls, text, attrs = {}) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined && text !== null) e.textContent = String(text);
    for (const [k, v] of Object.entries(attrs))
      if (v !== undefined && v !== null) e.setAttribute(k, String(v));
    return e;
  }
  function append(parent, ...children) {
    parent.append(...children.filter(Boolean));
    return parent;
  }
  function button(text, fn, cls = "button", attrs = {}) {
    const b = el("button", cls, text, { type: "button", ...attrs });
    if (
      /^(?:Apply \d+ files?|Discard(?: candidate)?|Stop(?: it| run)?|Resume run|Dispatch run)$/.test(
        text,
      )
    )
      b.dataset.networkAction = "true";
    if (fn) b.addEventListener("click", fn);
    return b;
  }
  function link(text, href, cls = "") {
    const a = el("a", cls, text, { href });
    a.dataset.link = "";
    return a;
  }
  function icon(name, cls = "", label) {
    const s = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    s.setAttribute("class", "icon " + cls);
    if (label) {
      s.setAttribute("role", "img");
      s.setAttribute("aria-label", label);
    } else s.setAttribute("aria-hidden", "true");
    const u = document.createElementNS(s.namespaceURI, "use");
    u.setAttribute("href", "/icons.svg#" + name);
    s.append(u);
    return s;
  }
  function iconButton(name, label, fn) {
    const b = button("", fn, "icon-button", {
      "aria-label": label,
      title: label,
    });
    b.append(icon(name));
    return b;
  }
  function disabled(control, why) {
    control.disabled = !!why;
    if (why) control.title = why;
    else control.removeAttribute("title");
    return control;
  }
  const memory = new Map();
  let storageFailed = false;
  function read(key, fallback, validate) {
    try {
      const value = JSON.parse(localStorage.getItem("pactrail." + key));
      return value !== null && (!validate || validate(value))
        ? value
        : fallback;
    } catch {
      storageFailed = true;
      return memory.get(key) ?? fallback;
    }
  }
  function save(key, value) {
    memory.set(key, value);
    try {
      localStorage.setItem("pactrail." + key, JSON.stringify(value));
    } catch {
      storageFailed = true;
    }
  }
  const object = (v) => v && typeof v === "object" && !Array.isArray(v);
  const preferenceDefaults = {
    theme: "system",
    density: "comfortable",
    motion: "system",
    timestamps: "relative",
    clock: "24h",
    diffMode: "auto",
    wrapDiff: false,
    wrapTrace: true,
    railCollapsed: false,
    filesPaneWidth: 300,
  };
  const allowedPreferences = {
    theme: ["system", "light", "dark"],
    density: ["comfortable", "compact"],
    motion: ["system", "reduce"],
    timestamps: ["relative", "absolute"],
    clock: ["24h", "12h"],
    diffMode: ["auto", "split", "unified"],
  };
  const prefs = Object.fromEntries(
    Object.entries(preferenceDefaults).map(([k, v]) => [
      k,
      read(k, v, (x) =>
        allowedPreferences[k]
          ? allowedPreferences[k].includes(x)
          : typeof x === typeof v,
      ),
    ]),
  );
  const state = {
    bootstrap: null,
    runs: [],
    jobs: [],
    offline: false,
    connected: false,
    backoff: 0,
    lastUpdated: null,
    refreshing: false,
    refreshTimer: null,
    stream: null,
    routeToken: 0,
    current: null,
    search: "",
    filter: "all",
    railRows: new Map(),
    railGroups: new Map(),
    titles: read("titles", {}, object),
    pricing: read("pricing", {}, object),
    draft: read("draft", { goal: "", title: "" }, object),
    config: read("config", {}, object),
    notes: read("discardNotes", {}, object),
    viewed: read("viewedFiles", {}, object),
    recent: read(
      "recentModels",
      [],
      (value) =>
        Array.isArray(value) &&
        value.every(
          (r) =>
            object(r) &&
            typeof r.model === "string" &&
            typeof r.provider === "string",
        ),
    ),
    pending: null,
    busy: false,
    runCache: new Map(),
    runJobs: new Map(),
    scrolls: new Map(),
    stopTimes: new Map(),
    phaseCollapsed: new Map(),
  };
  const providers = {
    ollama: "Ollama",
    "open-ai-compatible": "OpenAI-compatible",
    "open-ai": "OpenAI",
    "open-ai-responses": "OpenAI Responses",
    anthropic: "Anthropic",
    gemini: "Gemini",
  };
  function announce(text, urgent = false) {
    const region = $(urgent ? "alert" : "live");
    region.textContent = "";
    setTimeout(() => (region.textContent = text), 20);
  }
  function toast(text) {
    $("toast").textContent = text;
    $("toast").hidden = false;
    clearTimeout(state.toastTimer);
    state.toastTimer = setTimeout(() => ($("toast").hidden = true), 3000);
    announce(text);
  }
  async function copy(text, b) {
    try {
      await navigator.clipboard.writeText(String(text));
      if (b) {
        const children = [...b.childNodes];
        b.replaceChildren(icon("check"));
        setTimeout(() => {
          if (b.isConnected) b.replaceChildren(...children);
        }, 1200);
      }
      announce("Copied");
    } catch {
      showPopover(b || $("shortcuts"), "Clipboard unavailable", [
        el("p", "", "Select the text and use your browser’s copy command."),
      ]);
    }
  }
  function copyText(text, label = "Copy", cls = "mono") {
    const span = el("span", "copy-text");
    span.append(
      el("span", cls, text),
      iconButton("copy", label, () => copy(text, span.lastChild)),
    );
    return span;
  }
  function hashText(value, label = "hash") {
    if (!value)
      return usageNode(null, { reason: label + " was not recorded." });
    const v = String(value);
    return button(
      v.length > 20 ? v.slice(0, 10) + "…" + v.slice(-6) : v,
      (e) => copy(v, e.currentTarget),
      "hash",
      { title: v, "aria-label": "Copy " + label + ": " + v },
    );
  }
  function usageNode(value, options = {}, cls = "mono") {
    const f = formatUsage(value, options);
    return el("span", cls + " " + f.state, f.text, {
      title: f.reason,
      "aria-label":
        f.state === "absent" ? "not reported: " + f.reason : undefined,
    });
  }
  function clampText(text, cls = "", pre = false) {
    const wrap = el("div", cls),
      body = el(pre ? "pre" : "div", "", text),
      lines = String(text).split("\n").length;
    wrap.append(body);
    if (lines > 12 || String(text).length > 1200) {
      body.classList.add("clamped");
      const toggle = button(
        lines > 12 ? `Show all ${lines} lines` : "Show full text",
        () => {
          const collapsed = body.classList.toggle("clamped");
          toggle.textContent = collapsed
            ? lines > 12
              ? `Show all ${lines} lines`
              : "Show full text"
            : "Show less";
          toggle.setAttribute("aria-expanded", String(!collapsed));
        },
        "clamp-button",
        { "aria-expanded": "false" },
      );
      wrap.append(toggle);
    }
    return wrap;
  }
  function codeBlock(text, label = "code") {
    const b = clampText(text, "code-block", true);
    b.append(
      iconButton("copy", "Copy " + label, (e) => copy(text, e.currentTarget)),
    );
    b.lastChild.classList.add("copy-button");
    return b;
  }
  function banner(kind, title, body, actions = []) {
    const b = el("section", "banner " + kind);
    append(
      b,
      icon(
        kind === "warn"
          ? "triangle-alert"
          : kind === "fail"
            ? "octagon-alert"
            : kind === "info"
              ? "info"
              : "file-check",
        "banner-icon",
      ),
    );
    const content = el("div", "banner-body");
    append(
      content,
      el("h2", "", title, { tabindex: "-1" }),
      typeof body === "string" ? el("p", "", body) : body,
    );
    if (actions.length)
      append(content, append(el("div", "actions"), ...actions));
    b.append(content);
    return b;
  }
  function errorBlock(title, error, retry) {
    const block = el("section", "error-block");
    append(
      block,
      el("h2", "", title, { tabindex: "-1" }),
      codeBlock(error.message || String(error), "engine error"),
    );
    if (retry) block.append(button("Retry", retry));
    const technical = el("details"),
      summary = el("summary", "caption", "Technical details");
    append(
      technical,
      summary,
      el(
        "p",
        "mono-meta",
        `${error.status ?? "—"} · ${error.endpoint || "local browser"}`,
      ),
    );
    block.append(technical);
    return block;
  }
  async function api(path, options = {}) {
    let response;
    try {
      response = await fetch(path, {
        ...options,
        headers: {
          ...(options.body ? { "Content-Type": "application/json" } : {}),
          ...options.headers,
        },
        signal: options.signal || AbortSignal.timeout(30000),
      });
    } catch {
      const e = Error("The Pactrail engine is unreachable.");
      e.network = true;
      e.endpoint = path;
      throw e;
    }
    let value;
    try {
      value = await response.json();
    } catch {
      const e = Error("The engine returned an unexpected response.");
      e.status = response.status;
      e.endpoint = path;
      throw e;
    }
    if (!response.ok) {
      const e = Error(value.error || `Request failed (${response.status})`);
      e.status = response.status;
      e.endpoint = path;
      throw e;
    }
    return value;
  }
  function baseName(path) {
    return (
      String(path || "")
        .replace(/\/$/, "")
        .split(/[\\/]/)
        .pop() || "Workspace"
    );
  }
  function runTitle(run) {
    return (
      (typeof state.titles[run?.run_id] === "string" &&
        state.titles[run.run_id]) ||
      String(run?.goal || run?.contract?.goal || run?.run_id || "Run")
        .split("\n")
        .find((x) => x.trim()) ||
      "Run"
    );
  }
  function runState(run) {
    const value = run?.outcome || run?.state;
    const map = {
      ready_to_apply: "review",
      awaiting_apply: "review",
      completed: "answered",
      answered: "answered",
      cancelled: "stopped",
      applied: "applied",
      discarded: "discarded",
      failed: "failed",
    };
    if (map[value]) return map[value];
    const nonterminal = state.runs.filter(
      (r) => !TERMINAL.has(r.state) && r.state !== "awaiting_apply",
    );
    const active = state.jobs.filter((j) =>
      ["running", "cancelling"].includes(j.state),
    );
    if (
      nonterminal.length === 1 &&
      nonterminal[0].run_id === run?.run_id &&
      active.length === 1
    )
      return active[0].state === "cancelling" ? "stopping" : "running";
    if (state.connected && active.length === 0 && nonterminal.length === 1)
      return "interrupted";
    return state.connected ? "unknown" : "running";
  }
  const stateInfo = {
    running: ["Running", "loader-circle"],
    starting: ["Starting", "loader-circle"],
    review: ["Awaiting review", "file-diff"],
    answered: ["Answered", "message-square-text"],
    applied: ["Applied", "check-check"],
    discarded: ["Discarded", "trash-2"],
    failed: ["Failed", "octagon-alert"],
    stopped: ["Stopped", "square"],
    stopping: ["Stopping", "loader-circle"],
    interrupted: ["Interrupted", "unplug"],
    unknown: ["State unknown", "circle-help"],
  };
  function stateChip(key) {
    const [label, glyph] = stateInfo[key] || stateInfo.unknown;
    const c = el("span", "state-chip state-" + key);
    append(
      c,
      el("span", "sr-only", "State: "),
      icon(
        glyph,
        ["running", "starting", "stopping"].includes(key) ? "spin" : "",
      ),
      el("span", "", label),
    );
    return c;
  }
  function timeText(time, absolute = false) {
    const d = new Date(time);
    if (!Number.isFinite(d.getTime())) return "—";
    const delta = Math.max(0, (Date.now() - d.getTime()) / 1000);
    if (!absolute && prefs.timestamps === "relative" && delta < 86400)
      return delta < 60
        ? `${Math.floor(delta)} s ago`
        : delta < 3600
          ? `${Math.floor(delta / 60)} min ago`
          : `${Math.floor(delta / 3600)} h ago`;
    return d.toLocaleString("en-US", {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
      hour12: prefs.clock === "12h",
    });
  }
  function clock(time) {
    const d = new Date(time);
    return Number.isFinite(d.getTime())
      ? d.toLocaleTimeString("en-US", {
          hour: "2-digit",
          minute: "2-digit",
          second: "2-digit",
          hour12: prefs.clock === "12h",
        })
      : "—";
  }
  function applyPreferences() {
    document.documentElement.dataset.theme =
      prefs.theme === "system" ? "" : prefs.theme;
    document.documentElement.dataset.density = prefs.density;
    document.documentElement.dataset.motion = prefs.motion;
    $("app").classList.toggle("rail-collapsed", prefs.railCollapsed);
    document.documentElement.style.setProperty(
      "--files-width",
      Math.min(480, Math.max(220, prefs.filesPaneWidth)) + "px",
    );
    const svg =
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 20"><path d="M8 2v16" stroke="currentColor"/><g fill="currentColor"><circle cx="8" cy="3" r="2"/><circle cx="8" cy="10" r="2"/><circle cx="8" cy="17" r="2"/></g></svg>';
    let fav = document.querySelector('link[rel="icon"]');
    if (!fav) {
      fav = el("link", "", null, { rel: "icon" });
      document.head.append(fav);
    }
    const ink = getComputedStyle(document.documentElement)
      .getPropertyValue("--ink")
      .trim();
    fav.href =
      "data:image/svg+xml," +
      encodeURIComponent(svg.replaceAll("currentColor", ink));
    const background = getComputedStyle(document.documentElement)
      .getPropertyValue("--bg")
      .trim();
    for (const meta of document.querySelectorAll('meta[name="theme-color"]')) {
      if (prefs.theme !== "system") {
        meta.removeAttribute("media");
        meta.content = background;
      }
    }
  }
  function setPref(key, value) {
    prefs[key] = value;
    save(key, value);
    if (key === "diffMode" && state.current?.view === "changes") {
      const url = new URL(location.href);
      if (value === "auto") url.searchParams.delete("mode");
      else url.searchParams.set("mode", value);
      history.replaceState(null, "", url);
    }
    applyPreferences();
    if (state.current) {
      if (key === "diffMode" || key === "wrapDiff") updateDiffMode();
      if (key === "wrapTrace")
        state.current.trace?.scroller.classList.toggle("no-wrap", !value);
    }
  }
  function closeRail() {
    const wasOpen = $("rail").classList.contains("open");
    $("rail").classList.remove("open");
    $("app").classList.remove("rail-overlay");
    $("rail-scrim").hidden = true;
    if (wasOpen && $("rail").contains(document.activeElement))
      $("open-rail").focus();
  }
  function openRail(search = false) {
    $("rail").classList.add("open");
    $("app").classList.add("rail-overlay");
    $("rail-scrim").hidden = false;
    if (search) $("rail-search").focus();
    else $("rail-toggle").focus();
  }
  function navigate(path, replace = false) {
    saveScroll();
    history[replace ? "replaceState" : "pushState"]({}, "", path);
    route();
  }
  function saveScroll() {
    const current = state.current;
    if (current && current.panel)
      state.scrolls.set(
        current.id + ":" + current.view,
        current.trace && current.view === "trace"
          ? current.trace.scroller.scrollTop
          : current.panel.scrollTop,
      );
  }
  function pageBreadcrumb(title) {
    $("breadcrumb").textContent =
      (state.bootstrap ? baseName(state.bootstrap.workspace) + " / " : "") +
      title;
    document.title = title + " · Pactrail";
  }
  function updateRail() {
    const railScroll = $("run-list").scrollTop;
    const railFocus = document.activeElement;
    const list = $("run-list"),
      search = state.search.trim().toLowerCase();
    let rows = state.runs
      .filter((r) => {
        const k = runState(r);
        return (
          state.filter === "all" ||
          (state.filter === "active" &&
            ["running", "stopping", "interrupted", "unknown"].includes(k)) ||
          (state.filter === "review" && k === "review") ||
          (state.filter === "done" &&
            ["answered", "applied", "discarded"].includes(k)) ||
          (state.filter === "failed" && ["failed", "stopped"].includes(k))
        );
      })
      .filter(
        (r) =>
          !search ||
          runTitle(r).toLowerCase().includes(search) ||
          String(r.goal || "")
            .toLowerCase()
            .includes(search) ||
          (search.length >= 4 && r.run_id.toLowerCase().startsWith(search)),
      );
    rows.sort(
      (a, b) =>
        (runState(a) === "review" ? 0 : 1) -
          (runState(b) === "review" ? 0 : 1) ||
        String(b.created_at || b.run_id).localeCompare(
          String(a.created_at || a.run_id),
        ),
    );
    const wanted = [];
    let lastGroup = "";
    const reviews = state.runs.filter((r) => runState(r) === "review").length;
    for (const r of rows) {
      const k = runState(r),
        date = new Date(r.created_at);
      let group =
        k === "review"
          ? `Awaiting review · ${reviews}`
          : date.toDateString() === new Date().toDateString()
            ? "Today"
            : date.toDateString() ===
                new Date(Date.now() - 86400000).toDateString()
              ? "Yesterday"
              : Number.isFinite(date.getTime())
                ? date.toLocaleDateString("en-US", {
                    month: "short",
                    day: "numeric",
                    year: "numeric",
                  })
                : "Date not recorded";
      if (group !== lastGroup) {
        lastGroup = group;
        let g = state.railGroups.get(group);
        if (!g) {
          g = el("div", "rail-group", group);
          state.railGroups.set(group, g);
        }
        wanted.push(g);
      }
      let a = state.railRows.get(r.run_id);
      if (!a) {
        a = link("", "/runs/" + r.run_id, "run-row");
        const symbol = el("span", "state-symbol"),
          body = el("div", "run-row-body"),
          title = el("div", "run-row-title"),
          meta = el("div", "run-row-meta");
        append(body, title, meta);
        append(a, symbol, body);
        a.parts = { symbol, title, meta };
        state.railRows.set(r.run_id, a);
      }
      const p = a.parts,
        info = stateInfo[k];
      if (a.dataset.state !== k) {
        p.symbol.replaceChildren(
          icon(info[1], ["running", "stopping"].includes(k) ? "spin" : ""),
        );
        a.dataset.state = k;
      }
      const title = runTitle(r);
      if (p.title.textContent !== title) p.title.textContent = title;
      const files = Array.isArray(r.changes)
        ? r.changes.length
        : numeric(r.changes);
      const meta =
        info[0] +
        (files !== null ? ` · ${files} files` : "") +
        " · " +
        timeText(r.created_at);
      if (p.meta.textContent !== meta) p.meta.textContent = meta;
      a.title = title + " · " + timeText(r.created_at, true);
      a.classList.toggle("selected", location.pathname === "/runs/" + r.run_id);
      if (a.classList.contains("selected"))
        a.setAttribute("aria-current", "page");
      else a.removeAttribute("aria-current");
      wanted.push(a);
    }
    if (!rows.length) {
      let empty = list.querySelector(".rail-empty");
      if (!empty) empty = el("p", "muted rail-empty");
      empty.textContent = state.runs.length
        ? "No runs match your search and filter."
        : "No runs yet — Runs you dispatch appear here, with their receipts.";
      wanted.push(empty);
    }
    /* Reconcile without replacing existing focused rows or their text nodes. */
    let cursor = list.firstChild;
    for (const item of wanted) {
      if (item === cursor) cursor = cursor.nextSibling;
      else list.insertBefore(item, cursor);
    }
    const keep = new Set(wanted);
    for (const item of [...list.children]) if (!keep.has(item)) item.remove();
    list.scrollTop = railScroll;
    if (railFocus?.isConnected && document.activeElement !== railFocus)
      railFocus.focus({ preventScroll: true });
    for (const b of $("rail-filters").children) {
      b.setAttribute("aria-pressed", String(b.dataset.filter === state.filter));
      b.tabIndex = b.dataset.filter === state.filter ? 0 : -1;
      const count = b.querySelector(".count");
      if (count)
        count.textContent =
          b.dataset.filter === "all" ? state.runs.length : reviews;
    }
  }
  function initRail() {
    for (const [key, label] of [
      ["all", "All"],
      ["active", "Active"],
      ["review", "Review"],
      ["done", "Done"],
      ["failed", "Failed"],
    ]) {
      const b = button(
        label,
        () => {
          state.filter = key;
          updateRail();
        },
        "",
        { "aria-pressed": key === "all" },
      );
      b.dataset.filter = key;
      if (key === "all" || key === "review") b.append(el("span", "count", "0"));
      $("rail-filters").append(b);
      const o = el("option", "", label, { value: key });
      $("rail-filter-select").append(o);
    }
    roving($("rail-filters"), "button");
    $("rail-search").oninput = (e) => {
      state.search = e.target.value;
      updateRail();
    };
    $("rail-filter-select").onchange = (e) => {
      state.filter = e.target.value;
      updateRail();
    };
    $("workspace-copy").onclick = (e) =>
      copy(state.bootstrap?.workspace || "", e.currentTarget);
    $("open-rail").onclick = () => openRail();
    $("rail-toggle").onclick = () => {
      if ($("rail").classList.contains("open")) closeRail();
      else setPref("railCollapsed", !prefs.railCollapsed);
    };
    $("rail-scrim").onclick = closeRail;
  }
  function roving(container, selector) {
    container.addEventListener("keydown", (e) => {
      if (
        ![
          "ArrowRight",
          "ArrowLeft",
          "ArrowDown",
          "ArrowUp",
          "Home",
          "End",
        ].includes(e.key)
      )
        return;
      const items = [...container.querySelectorAll(selector)];
      let i = items.indexOf(document.activeElement);
      if (i < 0) return;
      e.preventDefault();
      i =
        e.key === "Home"
          ? 0
          : e.key === "End"
            ? items.length - 1
            : (i +
                (["ArrowRight", "ArrowDown"].includes(e.key) ? 1 : -1) +
                items.length) %
              items.length;
      items[i].focus();
      items[i].click();
    });
  }
  function field(label, value = "", options = {}) {
    const wrap = el("div", "field"),
      id = options.id || "field-" + Math.random().toString(36).slice(2),
      l = el("label", "", label, { for: id });
    const input = el(
      options.tag || "input",
      options.mono ? "mono" : "",
      undefined,
      {
        id,
        type: options.type || "text",
        placeholder: options.placeholder,
        ...options.attrs,
      },
    );
    input.value =
      typeof value === "string" || typeof value === "number" ? value : "";
    append(wrap, l, input);
    return { wrap, input };
  }
  function providerListbox(value, onchange) {
    const wrap = el("div", "provider-control"),
      trigger = button(
        "",
        () => {
          menu.hidden = !menu.hidden;
          trigger.setAttribute("aria-expanded", String(!menu.hidden));
          if (!menu.hidden)
            menu.querySelector('[aria-selected="true"]')?.focus();
        },
        "button provider-button",
        {
          "aria-haspopup": "listbox",
          "aria-expanded": "false",
          "aria-label": "Provider",
        },
      ),
      menu = el("div", "provider-menu", null, {
        role: "listbox",
        "aria-label": "Model provider",
        hidden: "",
      });
    let current = value in providers ? value : "ollama";
    const update = () => {
      trigger.replaceChildren(
        el("span", "", providers[current]),
        el("span", "mono-meta", current),
        el("span", "", "⌄"),
      );
      for (const b of menu.children) {
        b.setAttribute("aria-selected", String(b.dataset.value === current));
        b.tabIndex = b.dataset.value === current ? 0 : -1;
      }
    };
    for (const [key, name] of Object.entries(providers)) {
      const b = button(
        "",
        () => {
          current = key;
          update();
          menu.hidden = true;
          trigger.setAttribute("aria-expanded", "false");
          trigger.focus();
          onchange(key);
        },
        "provider-option",
        { role: "option" },
      );
      append(b, el("span", "", name), el("small", "", key));
      b.dataset.value = key;
      menu.append(b);
    }
    menu.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        menu.hidden = true;
        trigger.focus();
      }
      if (["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) {
        e.preventDefault();
        const items = [...menu.children];
        let i = items.indexOf(document.activeElement);
        i =
          e.key === "Home"
            ? 0
            : e.key === "End"
              ? items.length - 1
              : (i + (e.key === "ArrowDown" ? 1 : -1) + items.length) %
                items.length;
        items[i].focus();
      }
    });
    update();
    append(wrap, trigger, menu);
    return { wrap, value: () => current };
  }
  function configSection(title, summaryText, open = true) {
    const details = el(
        "details",
        "config-section",
        null,
        open ? { open: "" } : {},
      ),
      summary = el("summary"),
      titleNode = el("span", "", title),
      value = el("span", "caption", summaryText),
      body = el("div", "config-section-body");
    append(summary, titleNode, value);
    append(details, summary, body);
    return { details, body, summary: value };
  }
  function matchingCard(provider, model) {
    const card = state.pricing[provider + "::" + model];
    return object(card) ? card : null;
  }
  function completeCard(card) {
    return card && PRICE_FIELDS.every((k) => numeric(card[k]) !== null);
  }
  function renderComposer() {
    state.current = null;
    const main = $("main");
    main.replaceChildren();
    const page = el("section", "composer"),
      heading = el("header", "page-heading");
    append(
      heading,
      el("span", "workspace-eyebrow", "Your local coding workspace"),
      el("h1", "", "New task"),
      el("p", "hero-description", "What should we work on?"),
    );
    const ws = el("div", "workspace-row");
    append(
      ws,
      el("strong", "", baseName(state.bootstrap?.workspace)),
      el(
        "span",
        "workspace-path mono",
        state.bootstrap?.workspace || "Connecting…",
        { title: state.bootstrap?.workspace },
      ),
      iconButton("copy", "Copy workspace path", (e) =>
        copy(state.bootstrap?.workspace || "", e.currentTarget),
      ),
    );
    append(
      heading,
      ws,
      el("p", "caption", "Workspace set when Pactrail was started."),
    );
    page.append(heading);
    const form = el("form", "", null, { id: "composer-form" }),
      grid = el("div", "composer-grid"),
      briefCol = el("div", "brief-column"),
      configCol = el("div", "config-column");
    const intro = el("div", "intro");
    if (!state.runs.length) {
      append(
        intro,
        el(
          "p",
          "",
          `Work happens in an isolated copy. Nothing is written to ${baseName(state.bootstrap?.workspace)} until you apply a candidate.`,
        ),
        el(
          "p",
          "",
          "Every run leaves a receipt: contract, evidence, changed files, integrity hash.",
        ),
        el("p", "", "Start read-only if you’re unsure:"),
      );
      const chips = el("div", "starter-chips");
      for (const text of [
        "Explain this repository’s architecture",
        "List risky areas of this codebase",
      ]) {
        chips.append(
          button(
            text,
            () => {
              goal.value = text;
              onDraft();
              goal.focus();
            },
            "button",
          ),
        );
      }
      intro.append(chips);
    } else
      intro.append(
        el(
          "p",
          "",
          "Changes stay isolated until you review and apply a candidate.",
        ),
      );

    const goalField = field(
        "What should change?",
        typeof state.draft.goal === "string" ? state.draft.goal : "",
        {
          id: "task-goal",
          tag: "textarea",
          placeholder:
            "Describe the change you want. Include the failing symptom, the file or area if you know it, and what 'done' looks like.",
          attrs: { maxlength: 16000, required: "" },
        },
      ),
      goal = goalField.input;
    goal.className = "brief";
    goalField.wrap.firstChild.className = "field-label";
    const counter = el("div", "brief-counter");
    append(briefCol, goalField.wrap, counter);
    const titleDetails = el("details", "title-disclosure"),
      title = field("Title", state.draft.title || "", {
        id: "task-title",
        attrs: { maxlength: 200 },
      });
    append(
      titleDetails,
      el("summary", "", "Add a title (kept in this browser)"),
      title.wrap,
    );
    if (title.input.value) titleDetails.open = true;
    briefCol.append(titleDetails);
    const config = { ...state.bootstrap?.defaults, ...state.config };
    let selectedProvider = config.provider || "ollama",
      mode = ["disabled", "oci", "native"].includes(config.process_backend)
        ? config.process_backend
        : "disabled";
    const modelSection = configSection("Model", providers[selectedProvider]);
    const provider = providerListbox(selectedProvider, (v) => {
      selectedProvider = v;
      modelSection.summary.textContent = providers[v];
      sync();
    });
    const model = field("Model ID", config.model || "", {
      id: "model",
      mono: true,
      placeholder: "provider/model-id",
    });
    append(modelSection.body, provider.wrap, model.wrap);
    const recent = button("Recent", (e) =>
      showPopover(
        e.currentTarget,
        "Recent models (this browser)",
        state.recent.slice(0, 8).map((r) =>
          button(
            r.model,
            () => {
              model.input.value = r.model;
              closePopover();
              sync();
            },
            "button mono",
          ),
        ),
      ),
    );
    modelSection.body.append(recent);
    const pricingLine = el("p", "caption");
    modelSection.body.append(pricingLine);
    configCol.append(modelSection.details);
    const commands = configSection("Commands", "None"),
      seg = el("div", "commands-segments segments", null, {
        role: "radiogroup",
        "aria-label": "Commands",
      }),
      consequence = el("div", "commands-consequence"),
      image = field("Sandbox image", config.sandbox_image || "", {
        id: "sandbox-image",
        mono: true,
        placeholder: "localhost/myimage:tag",
      }),
      ack = el("label", "ack"),
      ackInput = el("input", "", null, { type: "checkbox", id: "host-ack" });
    append(
      ack,
      ackInput,
      el("span", "", "I allow this run to execute commands on this machine."),
    );
    for (const [key, label] of [
      ["disabled", "None"],
      ["oci", "Sandbox"],
      ["native", "Host"],
    ]) {
      const b = button(
        label,
        () => {
          mode = key;
          ackInput.checked = false;
          syncCommands();
          sync();
        },
        "",
        { role: "radio", "aria-checked": mode === key },
      );
      b.dataset.mode = key;
      b.tabIndex = mode === key ? 0 : -1;
      seg.append(b);
    }
    roving(seg, "button");
    append(commands.body, seg, consequence, image.wrap, ack);
    configCol.append(commands.details);
    const limits = configSection("Limits", "Default");
    const turns = field("Max turns", config.max_turns ?? "", {
        id: "max-turns",
        type: "number",
        placeholder: "default",
        attrs: { min: 1, max: 200 },
      }),
      timeout = field("Timeout seconds", config.request_timeout_seconds ?? "", {
        id: "timeout",
        type: "number",
        placeholder: "default",
        attrs: { min: 1, max: 3600 },
      }),
      cost = field("Max cost USD", config.max_cost_usd ?? "", {
        id: "max-cost",
        type: "number",
        placeholder: "No cap",
        attrs: { min: 0, step: "0.000001" },
      });
    const pair = append(el("div", "field-pair"), turns.wrap, timeout.wrap),
      costHint = el("p", "caption");
    append(limits.body, pair, cost.wrap, costHint);
    configCol.append(limits.details);
    const advanced = configSection("Endpoint & key", "Advanced", false),
      base = field("Base URL", config.base_url || "", {
        id: "base-url",
        mono: true,
        placeholder: "Provider default",
      }),
      key = field(
        "API key environment variable or saved reference",
        config.api_key_env || "",
        {
          id: "api-key-env",
          mono: true,
          placeholder: "OPENAI_API_KEY",
        },
      );
    append(
      advanced.body,
      base.wrap,
      key.wrap,
      el(
        "p",
        "caption",
        "Connect from the terminal with pactrail setup. This field accepts an environment variable name or the saved reference supplied by setup. The key itself never reaches this browser.",
      ),
    );
    configCol.append(advanced.details);
    const setup = el("details", "run-setup", null, { open: "" }),
      setupSummary = el("summary", "setup-summary");
    append(
      setupSummary,
      el("span", "", "Run setup"),
      el("span", "caption", "Model, permissions and limits"),
    );
    append(setup, setupSummary, configCol);
    grid.append(briefCol);
    form.append(grid);
    const errors = el("div", "", null, { id: "dispatch-error" }),
      bar = el("div", "dispatch-bar"),
      summary = el("span", "dispatch-summary"),
      dispatchButton = button("Dispatch run", null, "button primary", {
        type: "submit",
        id: "dispatch-button",
      });
    dispatchButton.append(el("kbd", "", "⌘↵ / Ctrl↵"));
    append(bar, summary, dispatchButton);
    append(briefCol, errors, bar);
    append(form, setup);
    page.append(form, intro);
    main.append(page);
    pageBreadcrumb("New task");
    let draftTimer;
    const onDraft = () => {
      state.draft = { goal: goal.value, title: title.input.value };
      clearTimeout(draftTimer);
      draftTimer = setTimeout(() => save("draft", state.draft), 300);
      goal.style.setProperty("height", "auto");
      goal.style.setProperty(
        "height",
        Math.max(144, Math.min(goal.scrollHeight, innerHeight * 0.6)) + "px",
      );
      counter.textContent =
        goal.value.length.toLocaleString("en-US") + " / 16,000";
      counter.className =
        "brief-counter" +
        (goal.value.length >= 16000
          ? " fail"
          : goal.value.length > 14400
            ? " warn"
            : "");
      sync();
    };
    function syncCommands() {
      for (const b of seg.children) {
        b.setAttribute("aria-checked", String(b.dataset.mode === mode));
        b.tabIndex = b.dataset.mode === mode ? 0 : -1;
      }
      commands.summary.textContent = {
        disabled: "None",
        oci: "Sandbox",
        native: "Host",
      }[mode];
      consequence.classList.toggle("warn", mode === "native");
      consequence.textContent = {
        disabled:
          "The agent can read and edit files in an isolated copy but cannot run tests or builds. Evidence will be unverified.",
        oci: "Commands run in an OCI container built from a local image.",
        native:
          "Commands run on your machine with your permissions, in an isolated copy of the workspace. Choose this only for code you trust.",
      }[mode];
      image.wrap.hidden = mode !== "oci";
      image.input.required = mode === "oci";
      ack.hidden = mode !== "native";
    }
    function sync() {
      if (state.bootstrap && !state.busy) {
        state.config = {
          provider: selectedProvider,
          model: model.input.value.trim(),
          process_backend: mode,
          sandbox_image: image.input.value,
          max_turns: turns.input.value,
          request_timeout_seconds: timeout.input.value,
          max_cost_usd: cost.input.value,
          base_url: base.input.value,
          api_key_env: key.input.value,
        };
        save("config", state.config);
      }
      const card = matchingCard(selectedProvider, model.input.value.trim()),
        filled = PRICE_FIELDS.filter((k) => numeric(card?.[k]) !== null).length;
      pricingLine.replaceChildren(
        el(
          "span",
          "",
          filled === 4
            ? `Pricing: $${card.input} input / $${card.output} output — cost cap available. `
            : filled
              ? `Pricing incomplete (${filled} of 4 prices). Cost cap unavailable. `
              : "Pricing: not set. Cost is not reported or capped. ",
        ),
        link(
          "Set pricing →",
          "/settings?provider=" +
            encodeURIComponent(selectedProvider) +
            "&model=" +
            encodeURIComponent(model.input.value.trim()) +
            "#pricing",
        ),
      );
      disabled(
        cost.input,
        filled === 4 && !state.offline
          ? null
          : state.offline
            ? "Offline."
            : "Needs all four prices",
      );
      costHint.textContent =
        filled === 4
          ? "Engine-enforced using this browser’s price card."
          : "Needs all four prices.";
      summary.textContent = `${selectedProvider} · ${model.input.value || "model not set"} · commands ${commands.summary.textContent.toLowerCase()} · ${turns.input.value || "default"} turns · ${cost.input.value && filled === 4 ? "$" + cost.input.value : "no cost cap"}`;
      const active = state.jobs.find((j) =>
        ["running", "cancelling"].includes(j.state),
      );
      const reason = state.offline
        ? "Reconnect to dispatch."
        : state.busy
          ? "Dispatching…"
          : active
            ? "A run is already active in this workspace."
            : !goal.value.trim()
              ? "Write a task first."
              : !model.input.value.trim()
                ? "Enter a model ID."
                : mode === "native" && !ackInput.checked
                  ? "Acknowledge host command execution."
                  : mode === "oci" && !image.input.value.trim()
                    ? "Enter a local image name."
                    : null;
      disabled(dispatchButton, reason);
      dispatchButton.firstChild.textContent = state.busy
        ? "Dispatching…"
        : "Dispatch run";
      let activeBanner = page.querySelector(".active-run-banner");
      if (active && !activeBanner) {
        activeBanner = banner(
          "info",
          "A run is already active in this workspace.",
          "Pactrail runs one at a time.",
          [
            button("Open active run", () => openActiveRun()),
            button(
              "Stop it",
              (e) => stopPopover(active, e.currentTarget),
              "button danger",
            ),
          ],
        );
        activeBanner.classList.add("active-run-banner");
        bar.before(activeBanner);
      }
      if (!active) activeBanner?.remove();
      bar.hidden = !!active;
    }
    state.composerSync = sync;
    state.composerRef = { goal, title: title.input };
    for (const input of [goal, title.input])
      input.addEventListener("input", onDraft);
    for (const input of [
      model.input,
      turns.input,
      timeout.input,
      cost.input,
      base.input,
      key.input,
      image.input,
      ackInput,
    ])
      input.addEventListener("input", sync);
    form.addEventListener("submit", async (e) => {
      e.preventDefault();
      sync();
      if (dispatchButton.disabled) return;
      errors.replaceChildren();
      let body;
      try {
        body = {
          goal: goal.value.trim(),
          provider: selectedProvider,
          model: model.input.value.trim(),
          process_backend: mode,
          ...pricingToEngine(
            completeCard(
              matchingCard(selectedProvider, model.input.value.trim()),
            )
              ? matchingCard(selectedProvider, model.input.value.trim())
              : null,
            cost.input.disabled ? "" : cost.input.value,
          ),
        };
        if (turns.input.value) body.max_turns = Number(turns.input.value);
        if (timeout.input.value)
          body.request_timeout_seconds = Number(timeout.input.value);
        if (base.input.value.trim()) body.base_url = base.input.value.trim();
        if (key.input.value.trim()) body.api_key_env = key.input.value.trim();
        if (mode === "oci") body.sandbox_image = image.input.value.trim();
        state.config = { ...body, max_cost_usd: cost.input.value };
        delete state.config.goal;
        for (const k of Object.keys(state.config))
          if (k.endsWith("_price") || k === "max_cost_microusd")
            delete state.config[k];
        save("config", state.config);
        save("draft", state.draft);
        state.busy = true;
        for (const control of form.querySelectorAll("input,textarea,button"))
          disabled(control, "Dispatching…");
        form.setAttribute("aria-busy", "true");
        const ids = new Set(state.runs.map((r) => r.run_id));
        const job = await api("/api/runs", {
          method: "POST",
          body: JSON.stringify(body),
        });
        state.pending = {
          job: job.id,
          ids,
          start: Date.now(),
          goal: body.goal,
          title: title.input.value,
          resume: null,
        };
        state.recent = [
          { provider: body.provider, model: body.model },
          ...state.recent.filter(
            (r) => !(r.provider === body.provider && r.model === body.model),
          ),
        ].slice(0, 8);
        save("recentModels", state.recent);
        announce("Run started");
        navigate("/runs/starting?job=" + encodeURIComponent(job.id));
      } catch (error) {
        errors.append(errorBlock("Unable to dispatch.", error));
        errors.querySelector("h2").focus();
        if (error.network) connectionLost();
      } finally {
        state.busy = false;
        if (form.isConnected) {
          for (const control of form.querySelectorAll("input,textarea,button"))
            disabled(control, null);
          form.removeAttribute("aria-busy");
          sync();
        }
      }
    });
    syncCommands();
    onDraft();
    if (innerWidth < 768)
      for (const section of [modelSection, limits])
        section.details.open = false;
  }
  function openActiveRun() {
    const active = state.runs.filter(
      (r) => !TERMINAL.has(r.state) && r.state !== "awaiting_apply",
    );
    if (active.length === 1)
      navigate("/runs/" + active[0].run_id + "?view=trace");
    else if (state.pending) navigate("/runs/starting?job=" + state.pending.job);
    else openRail();
  }
  function renderStarting() {
    const params = new URLSearchParams(location.search),
      jobId = params.get("job"),
      pending = state.pending;
    const page = el("section", "starting-page");
    append(
      page,
      el("div", "row", null),
      el("h1", "", pending?.goal || "Starting run", { tabindex: "-1" }),
    );
    page.firstChild.append(stateChip("starting"));
    const list = el("ol", "starting-checklist");
    append(
      list,
      append(
        el("li"),
        el("span", "", "✓"),
        el("span", "", "Request accepted by Pactrail"),
        el("span", "caption", clock(pending?.start)),
      ),
      append(
        el("li"),
        el("span", "spin", "◌"),
        el("span", "", "Waiting for the engine to register the run…"),
        el("span", "caption", null, { id: "starting-elapsed" }),
      ),
      append(
        el("li"),
        el("span", "", "○"),
        el("span", "", "Contract, then investigation"),
      ),
    );
    page.append(list);
    const result = el("div", "", null, { id: "starting-result" });
    page.append(result);
    $("main").replaceChildren(page);
    pageBreadcrumb("Starting");
    page.querySelector("h1").focus();
    state.startingTick = () => {
      const elapsed = pending ? (Date.now() - pending.start) / 1000 : null;
      const clockNode = $("starting-elapsed");
      if (clockNode)
        clockNode.textContent = formatUsage(elapsed, { kind: "duration" }).text;
      const job = state.jobs.find((j) => j.id === jobId);
      const found = pending?.resume
        ? state.runs.find(
            (r) =>
              r.run_id === pending.resume &&
              !TERMINAL.has(r.state) &&
              job?.state !== "failed",
          )
        : state.runs.find((r) => pending && !pending.ids.has(r.run_id));
      if (found) {
        state.runJobs.set(found.run_id, jobId);
        if (pending?.title) {
          state.titles[found.run_id] = pending.title;
          save("titles", state.titles);
        }
        state.pending = null;
        navigate("/runs/" + found.run_id + "?view=trace", true);
        return;
      }
      if (job?.output?.run_id) {
        navigate("/runs/" + job.output.run_id + "?view=trace", true);
        return;
      }
      if (
        job &&
        ["failed", "cancelled"].includes(job.state) &&
        !result.dataset.failed
      ) {
        result.dataset.failed = "true";
        result.replaceChildren(
          errorBlock(
            "The engine exited before creating a run.",
            Error(job.error || "No error was reported."),
          ),
          append(
            el("div", "actions"),
            button("Edit and retry", () => navigate("/")),
            button("Copy error", (e) =>
              copy(job.error || "No error was reported.", e.currentTarget),
            ),
          ),
        );
        announce("Run failed to start", true);
      } else if (elapsed > 20 && !result.children.length)
        result.append(
          banner(
            "warn",
            "This is taking longer than usual.",
            "The model endpoint may be slow to respond.",
            [
              button(
                "Stop",
                (e) => job && stopPopover(job, e.currentTarget),
                "button danger",
              ),
              button("Keep waiting", () => result.replaceChildren()),
            ],
          ),
        );
    };
    state.startingTick();
    clearTimeout(state.refreshTimer);
    state.refreshTimer = setTimeout(() => refresh(), 0);
  }
  function showPopover(anchor, title, content, actions = []) {
    const p = $("popover");
    p.replaceChildren(
      el("h2", "", title),
      ...content,
      append(el("div", "actions"), ...actions),
    );
    p.hidden = false;
    const rect = anchor?.getBoundingClientRect() || { left: 24, bottom: 60 };
    p.style.setProperty(
      "left",
      Math.max(12, Math.min(rect.left, innerWidth - p.offsetWidth - 12)) + "px",
    );
    p.style.setProperty(
      "top",
      Math.max(
        12,
        Math.min(rect.bottom + 8, innerHeight - p.offsetHeight - 12),
      ) + "px",
    );
    state.popoverAnchor = anchor;
    p.querySelector("button,input,textarea")?.focus();
  }
  function closePopover(restore = true) {
    if ($("popover").hidden) return;
    $("popover").hidden = true;
    if (restore) {
      if (state.popoverAnchor?.isConnected) state.popoverAnchor.focus();
      else state.current?.nodes?.title.focus();
    }
  }
  function stopPopover(job, anchor) {
    if (state.offline) return;
    const keep = button("Keep running", closePopover);
    showPopover(
      anchor,
      "Stop this run?",
      [
        el(
          "p",
          "",
          "The engine will wind down and may write a partial receipt.",
        ),
      ],
      [
        button(
          "Stop run",
          async () => {
            try {
              await api("/api/jobs/" + job.id + "/cancel", { method: "POST" });
              state.stopTimes.set(job.id, Date.now());
              job.state = "cancelling";
              closePopover();
              await refresh();
            } catch (error) {
              $("popover").append(errorBlock("Unable to stop.", error));
            }
          },
          "button danger",
        ),
        keep,
      ],
    );
    keep.focus();
  }
  function jobFor(run) {
    const direct = state.jobs.find(
      (j) => j.output?.run_id === run.id || j.id === state.runJobs.get(run.id),
    );
    if (direct) return direct;
    const active = state.jobs.filter((j) =>
        ["running", "cancelling"].includes(j.state),
      ),
      runs = state.runs.filter(
        (r) => !TERMINAL.has(r.state) && r.state !== "awaiting_apply",
      );
    if (active.length === 1 && runs.length === 1 && runs[0].run_id === run.id) {
      state.runJobs.set(run.id, active[0].id);
      return active[0];
    }
    return null;
  }
  async function resumeRun(run, anchor) {
    if (state.offline) return;
    const keep = button("Cancel", closePopover);
    showPopover(
      anchor,
      "Ask the engine to resume?",
      [
        el(
          "p",
          "",
          "Pactrail checks the last durable state. Terminal or unsafe checkpoints may be refused; the engine’s reason will be shown.",
        ),
      ],
      [
        keep,
        button("Resume run", async () => {
          try {
            const job = await api("/api/runs/" + run.id + "/resume", {
              method: "POST",
            });
            state.pending = {
              job: job.id,
              ids: new Set(state.runs.map((r) => r.run_id)),
              start: Date.now(),
              goal: runTitle(run.record),
              resume: run.id,
            };
            closePopover();
            navigate("/runs/starting?job=" + job.id);
          } catch (error) {
            $("popover").append(errorBlock("Resume refused.", error));
          }
        }),
      ],
    );
    keep.focus();
  }
  function receiptOf(detail) {
    return detail?.contract && detail?.outcome
      ? detail
      : detail?.receipt || null;
  }
  function contractOf(run) {
    return (
      run.receipt?.contract ||
      run.events.find((e) => e.event?.type === "contract_registered")?.event
        .data ||
      null
    );
  }
  function liveEvidence(run) {
    return (
      run.receipt?.evidence ||
      run.events
        .filter((e) => e.event?.type === "evidence_recorded")
        .map((e) => e.event.data)
    );
  }
  function currentRecord(run) {
    return {
      ...run.record,
      ...(run.receipt ? { outcome: run.receipt.outcome } : {}),
      state:
        run.events.filter((e) => e.event?.type === "state_changed").at(-1)
          ?.event.data.to || run.record.state,
    };
  }
  async function loadRun(id, token) {
    const main = $("main"),
      loading = setTimeout(() => {
        if (token === state.routeToken)
          main.replaceChildren(
            append(
              el("div", "reading"),
              el("div", "skeleton"),
              el("div", "skeleton"),
            ),
          );
      }, 150);
    const slow = setTimeout(() => {
      if (token === state.routeToken)
        main.append(
          banner("neutral", "Still loading…", "", [
            button("Retry", () => route()),
          ]),
        );
    }, 8000);
    try {
      const result = await Promise.all([
        api(`/api/runs/${id}/inspect`),
        api(`/api/runs/${id}/trace`),
      ]);
      if (token !== state.routeToken) return;
      const run = {
        id,
        record: state.runs.find((r) => r.run_id === id) || {
          run_id: id,
          state: result[0].state,
          goal: result[0].contract?.goal,
        },
        detail: result[0],
        receipt: receiptOf(result[0]),
        events: [],
        seen: new Set(),
        eventQueue: [],
        panels: new Map(),
        view: null,
        diff: null,
        diffError: null,
        completedBanner: false,
        trace: null,
      };
      state.current = run;
      mergeEvents(run, Array.isArray(result[1]) ? result[1] : [], false);
      renderRunShell(run);
      if (run.receipt) loadDiff(run);
      if (!TERMINAL.has(currentRecord(run).state)) openStream(run);
    } catch (error) {
      if (token !== state.routeToken) return;
      main.replaceChildren(
        append(
          el("section", "empty-state"),
          el(
            "h1",
            "",
            error.status === 404
              ? "This run isn’t in this workspace’s state directory."
              : "Unable to load run",
          ),
          errorBlock("Run unavailable.", error, () => route()),
          button("Back to runs", () => {
            navigate("/");
            openRail();
          }),
        ),
      );
      if (error.network) connectionLost();
    } finally {
      clearTimeout(loading);
      clearTimeout(slow);
    }
  }
  function mergeEvents(run, events, render = true) {
    const fresh = [];
    for (const e of events) {
      if (!Number.isInteger(e.sequence) || run.seen.has(e.sequence)) continue;
      run.seen.add(e.sequence);
      run.events.push(e);
      fresh.push(e);
    }
    run.events.sort((a, b) => a.sequence - b.sequence);
    if (!render) return;
    run.eventQueue.push(...fresh);
    if (!run.frame)
      run.frame = requestAnimationFrame(() => appendEventBatch(run));
  }
  function appendEventBatch(run) {
    const batchStart = performance.now();
    run.frame = null;
    if (state.current !== run) return;
    const batch = run.eventQueue.splice(0, 50);
    if (run.trace) {
      for (const e of batch) appendTraceEvent(run, e);
      applyTraceFilter(run, false);
      if (batch.length) {
        if (run.trace.following) followTrace(run, false);
        else {
          run.trace.newEvents += batch.length;
          updateFollowPill(run);
        }
      }
    }
    run.loadedOnce = true;
    updateRunHeader(run);
    if (run.panels.has("evidence")) updateEvidence(run);
    if (batch.some((e) => e.event?.type === "state_changed")) {
      updateRunState(run);
      if (TERMINAL.has(currentRecord(run).state)) {
        closeStream();
        refreshRunDetail(run);
      }
    }
    window.dispatchEvent(
      new CustomEvent("pactrail:trace-batch", {
        detail: {
          count: batch.length,
          durationMs: performance.now() - batchStart,
        },
      }),
    );
    if (run.eventQueue.length)
      run.frame = requestAnimationFrame(() => appendEventBatch(run));
  }
  function openStream(run) {
    closeStream();
    if (state.offline || TERMINAL.has(currentRecord(run).state)) return;
    const stream = new EventSource(`/api/runs/${run.id}/events`);
    state.stream = stream;
    stream.addEventListener("trace", (e) => {
      if (state.current !== run) return;
      try {
        const events = JSON.parse(e.data);
        if (Array.isArray(events)) mergeEvents(run, events);
      } catch {
        /* Invalid batches do not enter the feed. */
      }
    });
    stream.onerror = () => {
      if (state.current === run) refresh();
    };
  }
  function closeStream() {
    state.stream?.close();
    state.stream = null;
  }
  function defaultView(record) {
    const k = runState(record);
    return k === "answered"
      ? "answer"
      : ["review", "applied", "discarded"].includes(k)
        ? "changes"
        : "trace";
  }
  function renderRunShell(run) {
    const page = el("article", "run-page"),
      header = el("header", "run-header"),
      heading = el("div", "run-heading"),
      title = el("h1", "", runTitle(run.record), { tabindex: "-1" }),
      stateSlot = el("span"),
      actionsBox = el("div", "actions"),
      meta = el("div", "run-meta"),
      phase = el("div", "phase-track"),
      mobile = el("div", "phase-mobile"),
      stats = el("div", "ledger-strip"),
      last = el("div", "last-activity"),
      banners = el("div", "run-banners"),
      tabs = el("div", "tabs", null, {
        role: "tablist",
        "aria-label": "Run views",
      }),
      content = el("div", "run-content");
    append(
      heading,
      title,
      iconButton("pencil", "Edit title (this browser)", (e) =>
        renameRun(run, e.currentTarget),
      ),
      stateSlot,
      actionsBox,
    );
    append(header, heading, meta, phase, mobile, stats, last);
    append(page, header, banners, tabs, content);
    $("main").replaceChildren(page);
    run.nodes = {
      page,
      header,
      title,
      stateSlot,
      actions: actionsBox,
      meta,
      phase,
      mobile,
      stats,
      last,
      banners,
      tabs,
      content,
    };
    run.statNodes = {};
    for (const [key, label] of [
      ["elapsed", "Elapsed"],
      ["turns", "Turns"],
      ["tokens", "Tokens"],
      ["cost", "Cost"],
      ["files", "Files"],
    ]) {
      const cell = el("div", "stat"),
        value = el("div", "stat-value"),
        caption = el("div", "stat-label", label),
        meter = el("meter", "", null, {
          min: 0,
          max: 1,
          value: 0,
          hidden: "",
          "aria-label": label + " budget",
        });
      append(cell, value, caption, meter);
      stats.append(cell);
      run.statNodes[key] = { value, caption, meter };
    }
    roving(tabs, '[role="tab"]');
    const detailsButton = button(
      "⌄",
      () => {
        const collapsed = page.classList.toggle("mobile-collapsed");
        detailsButton.setAttribute("aria-expanded", String(!collapsed));
      },
      "icon-button header-details-button",
      {
        "aria-expanded": "true",
        "aria-label": "Show run details",
        title: "Show run details",
      },
    );
    heading.append(detailsButton);
    page.addEventListener(
      "wheel",
      (event) => {
        if (
          innerWidth < 768 &&
          event.deltaY > 0 &&
          !header.contains(event.target)
        ) {
          page.classList.add("mobile-collapsed");
          detailsButton.setAttribute("aria-expanded", "false");
        }
      },
      { passive: true },
    );
    updateRunHeader(run);
    updateRunState(run);
    let requested =
      new URLSearchParams(location.search).get("view") ||
      defaultView(currentRecord(run));
    if (
      !["trace", "changes", "evidence", "receipt", "answer"].includes(
        requested,
      ) ||
      (["changes", "receipt"].includes(requested) && !run.receipt)
    )
      requested = "trace";
    if (innerWidth < 768 && requested === "changes") {
      page.classList.add("mobile-collapsed");
      detailsButton.setAttribute("aria-expanded", "false");
    }
    switchTab(run, requested, false);
    title.focus();
  }
  function renameRun(run, anchor) {
    const input = field("Title (kept in this browser)", runTitle(run.record), {
      attrs: { maxlength: 200 },
    });
    showPopover(
      anchor,
      "Run title",
      [input.wrap],
      [
        button("Save", () => {
          state.titles[run.id] = input.input.value.trim();
          save("titles", state.titles);
          closePopover();
          updateRunHeader(run);
          updateRail();
        }),
      ],
    );
    input.input.focus();
    input.input.select();
  }
  function updateRunHeader(run) {
    if (!run.nodes) return;
    const record = currentRecord(run),
      key = runState(record),
      contract = contractOf(run),
      usage = usageProjection(run.events),
      first = run.events[0]?.timestamp,
      last = run.events.at(-1)?.timestamp,
      live = !TERMINAL.has(record.state) && record.state !== "awaiting_apply";
    if (run.nodes.title.textContent !== runTitle(record))
      run.nodes.title.textContent = runTitle(record);
    pageBreadcrumb(runTitle(record));
    if (run.headerKey !== key) {
      run.nodes.stateSlot.replaceChildren(stateChip(key));
      run.headerKey = key;
    }
    const model = actions(run.events)
      .find((e) => e.event.data.actor?.startsWith("model:"))
      ?.event.data.actor.slice(6);
    const metaText = `${model || "Model: not recorded"} · started ${first ? timeText(first, true) : "—"} · run ${run.id.slice(0, 8)}`;
    if (run.nodes.meta.dataset.text !== metaText) {
      run.nodes.meta.dataset.text = metaText;
      run.nodes.meta.replaceChildren(
        el("span", "", metaText),
        iconButton("copy", "Copy run ID", (e) => copy(run.id, e.currentTarget)),
      );
    }
    const permissions = contract?.permissions;
    const commandsMode = permissions?.deny?.includes("process_spawn")
      ? "none"
      : null;
    if (commandsMode && !run.nodes.meta.querySelector(".commands-meta"))
      run.nodes.meta.append(
        el("span", "commands-meta", "· commands: " + commandsMode),
      );
    const transitions = run.events.filter(
        (e) => e.event?.type === "state_changed",
      ),
      entered = transitions.map((e) => e.event.data.to),
      currentPhase =
        PHASE_STATE[record.state] ??
        PHASE_STATE[transitions.at(-1)?.event.data.from] ??
        0,
      executeCount = entered.filter((x) => x === "executing").length;
    const phaseKey = [currentPhase, key, executeCount, entered.join(",")].join(
      "|",
    );
    if (run.phaseKey !== phaseKey) {
      run.phaseKey = phaseKey;
      run.nodes.phase.replaceChildren();
      PHASES.forEach((name, i) => {
        let cls = "phase";
        if (
          i === currentPhase &&
          ["applied", "discarded", "answered"].includes(key)
        )
          cls += " done";
        else if (i === currentPhase)
          cls +=
            " current" +
            (key === "stopped"
              ? " stopped"
              : key === "stopping"
                ? " stopping"
                : key === "failed"
                  ? " failed"
                  : "");
        else if (i < currentPhase) cls += " done";
        if (i === 2 && !entered.includes("planning") && currentPhase > 2)
          cls = "phase skipped";
        const label =
          name + (i === 3 && executeCount > 1 ? ` ×${executeCount}` : "");
        run.nodes.phase.append(
          el("div", cls, label, {
            title: cls.includes("skipped") ? "Plan was skipped" : name,
          }),
        );
      });
      run.nodes.mobile.replaceChildren(
        el("span", "", `${PHASES[currentPhase]} · ${currentPhase + 1} of 7`),
        el("progress", "", null, {
          max: 7,
          value: currentPhase + 1,
          "aria-label": "Run phase",
        }),
      );
    }
    const elapsed = first
      ? Math.max(
          0,
          ((live ? Date.now() : new Date(last).getTime()) -
            new Date(first).getTime()) /
            1000,
        )
      : null;
    const effects = actions(run.events).flatMap(
      (e) => e.event.data.observed_effects || [],
    );
    const touched = new Set(
      effects
        .filter((x) => /^fs\.(changed|write|remove):/.test(x))
        .map((x) => x.replace(/^fs\.[^:]+:/, "")),
    );
    const values = {
      elapsed: [
        elapsed,
        { kind: "duration" },
        contract?.budget?.wall_time_seconds,
      ],
      turns: [usage.turns, {}, contract?.budget?.max_model_attempts],
      tokens: [
        usage.total,
        {
          kind: "tokens",
          coverage: { reported: usage.reported, total: usage.turns },
        },
        contract?.budget?.model_tokens,
      ],
      cost: [
        run.detail?.web_result?.cost_microusd,
        {
          kind: "cost",
          reason:
            "No cost reported. Set model pricing to enable cost tracking.",
        },
        contract?.budget?.cost_microusd,
      ],
      files: [
        run.receipt ? run.receipt.changes.length : touched.size,
        {},
        null,
      ],
    };
    for (const [key, [value, options, budget]] of Object.entries(values)) {
      const parts = run.statNodes[key],
        f = formatUsage(value, options);
      parts.value.textContent = f.text;
      parts.value.className = "stat-value " + f.state;
      parts.value.title = f.reason;
      if (f.state === "absent")
        parts.value.setAttribute("aria-label", "not reported: " + f.reason);
      else parts.value.removeAttribute("aria-label");
      parts.meter.hidden = !(budget > 0 && numeric(value) !== null);
      if (!parts.meter.hidden) {
        parts.meter.max = budget;
        parts.meter.value = Math.min(value, budget);
        parts.meter.title =
          formatUsage(value).text + " / " + formatUsage(budget).text;
      }
    }
    const usageText = [
      ...["input", "cached", "creation", "output"].map(
        (k) =>
          `${k}: ${formatUsage(usage.counts[k] ? usage[k] : null, { coverage: { reported: usage.counts[k], total: usage.turns } }).text}`,
      ),
      formatUsage(usage.total, {
        coverage: { reported: usage.reported, total: usage.turns },
      }).reason,
    ].join(" · ");
    run.statNodes.tokens.value.title = usageText;
    run.statNodes.files.caption.textContent = run.receipt
      ? "Files"
      : "Files touched";
    const latest = run.events.at(-1),
      summary =
        latest?.event?.data?.summary ||
        latest?.event?.data?.message ||
        latest?.event?.type?.replaceAll("_", " ") ||
        "No events reported.";
    run.nodes.last.textContent = latest
      ? `Last event ${timeText(latest.timestamp)} · ${summary}`
      : "No events reported.";
    const actionKey =
      key + ":" + state.offline + ":" + (jobFor(run)?.state || "");
    if (run.actionKey !== actionKey) {
      run.actionKey = actionKey;
      run.nodes.actions.replaceChildren();
      const job = jobFor(run);
      if (job && ["running", "cancelling"].includes(job.state)) {
        const stop = button(
          job.state === "cancelling" ? "Stopping…" : "Stop",
          (e) => stopPopover(job, e.currentTarget),
          "button danger",
        );
        disabled(
          stop,
          state.offline
            ? "Offline."
            : job.state === "cancelling"
              ? "Waiting for the engine to wind down."
              : null,
        );
        run.nodes.actions.append(stop);
      } else if (["interrupted", "failed", "stopped"].includes(key)) {
        run.nodes.actions.append(
          disabled(
            button("Resume run", (e) => resumeRun(run, e.currentTarget)),
            state.offline ? "Offline." : null,
          ),
        );
      }
    }
    const delta = latest
      ? (Date.now() - new Date(latest.timestamp).getTime()) / 1000
      : 0;
    if (
      key === "running" &&
      delta > 90 &&
      !run.nodes.banners.querySelector(".silence-banner")
    ) {
      const b = banner(
        "warn",
        `No new events for ${Math.floor(delta)} s.`,
        "The model or a command may be slow.",
      );
      b.classList.add("silence-banner");
      run.nodes.banners.append(b);
    } else if (key !== "running" || delta <= 90)
      run.nodes.banners.querySelector(".silence-banner")?.remove();
  }
  function updateRunState(run) {
    if (!run.nodes) return;
    const record = currentRecord(run),
      key = runState(record);
    const jobError = jobFor(run)?.error || "";
    if (run.bannerKey === key && run.bannerError === jobError) return;
    run.bannerError = jobError;
    const previous = run.bannerKey;
    run.bannerKey = key;
    run.nodes.banners.replaceChildren();
    const changes = run.receipt?.changes || [],
      when = run.events
        .filter(
          (e) =>
            e.event?.type === "state_changed" &&
            [
              "applied",
              "discarded",
              "failed",
              "cancelled",
              "completed",
            ].includes(e.event.data.to),
        )
        .at(-1)?.timestamp;
    const newTask = button("New run from this brief", () => {
      state.draft = {
        goal: contractOf(run)?.goal || run.record.goal || "",
        title: "",
      };
      save("draft", state.draft);
      navigate("/");
    });
    let b;
    if (key === "stopping") {
      const job = jobFor(run),
        requested = state.stopTimes.get(job?.id);
      b = banner(
        "warn",
        `Stop requested${requested ? " at " + clock(requested) : ""}.`,
        "Waiting for the engine to finish its current action. A partial receipt may be written.",
      );
    } else if (key === "stopped") {
      const from = run.events
        .filter(
          (e) =>
            e.event?.type === "state_changed" &&
            e.event.data.to === "cancelled",
        )
        .at(-1)?.event.data.from;
      b = banner(
        "warn",
        `Stopped${from ? " during " + from : ""}${when ? " at " + clock(when) : ""}.`,
        run.receipt
          ? `Partial receipt: ${changes.length} file${changes.length === 1 ? "" : "s"} changed, ${run.receipt.evidence.length} evidence records. Stopped runs can’t be applied.`
          : "No receipt was written.",
        [
          ...(run.receipt
            ? [button("Open partial changes", () => switchTab(run, "changes"))]
            : []),
          button("Resume run", (e) => resumeRun(run, e.currentTarget)),
          newTask,
        ],
      );
    } else if (key === "failed") {
      const job = jobFor(run),
        from = run.events
          .filter(
            (e) =>
              e.event?.type === "state_changed" && e.event.data.to === "failed",
          )
          .at(-1)?.event.data.from;
      b = banner(
        "fail",
        `Failed${from ? " during " + from : ""}${when ? " at " + clock(when) : ""}.`,
        job?.error
          ? codeBlock(job.error, "failure reason")
          : el(
              "p",
              "",
              "The engine didn’t record a reason after this Pactrail session ended. Last events: " +
                run.events
                  .slice(-5)
                  .map((e) => e.event.data?.summary || e.event.type)
                  .join(" · "),
            ),
        [
          button("Jump to failure in trace", () => {
            switchTab(run, "trace");
            const e =
              run.events.findLast(
                (e) =>
                  e.event.type === "action_completed" &&
                  e.event.data.succeeded === false,
              ) || run.events.at(-1);
            jumpToEvent(run, e?.sequence);
          }),
          button("Resume run", (e) => resumeRun(run, e.currentTarget)),
          newTask,
        ],
      );
      announce("Run failed", true);
    } else if (key === "interrupted")
      b = banner(
        "warn",
        "This run is not currently being executed.",
        "The process may have been stopped or Pactrail restarted.",
        [button("Resume run", (e) => resumeRun(run, e.currentTarget))],
      );
    else if (key === "unknown")
      b = banner(
        "warn",
        "Execution state unknown.",
        "The current jobs cannot be associated with this run unambiguously.",
      );
    else if (key === "applied")
      b = banner(
        "neutral",
        `Applied ${changes.length} file${changes.length === 1 ? "" : "s"} to ${baseName(state.bootstrap?.workspace)}${when ? " at " + clock(when) : ""}.`,
        "",
        [
          button("Open receipt", () => switchTab(run, "receipt")),
          button("New run", () => navigate("/")),
        ],
      );
    else if (key === "discarded")
      b = banner(
        "neutral",
        `Candidate discarded${when ? " at " + clock(when) : ""}.`,
        "Workspace files were not changed." +
          (state.notes[run.id]
            ? " Your note (this browser only): " + state.notes[run.id]
            : ""),
        [button("Open receipt", () => switchTab(run, "receipt")), newTask],
      );
    else if (key === "review" && previous && previous !== "review")
      b = banner(
        "info",
        `Run finished — ${changes.length} file${changes.length === 1 ? "" : "s"} ready for review.`,
        "",
        [button("Open changes", () => switchTab(run, "changes"))],
      );
    if (b) run.nodes.banners.append(b);
    if (previous && key !== previous)
      announce("Run finished: " + (stateInfo[key]?.[0] || key));
    updateTabs(run);
  }
  function updateTabs(run) {
    const keys =
      runState(currentRecord(run)) === "answered"
        ? ["answer", "trace", "evidence", "receipt"]
        : ["trace", "changes", "evidence", "receipt"];
    for (const key of keys) {
      let tab = run.nodes.tabs.querySelector(`[data-view="${key}"]`);
      if (!tab) {
        tab = button(
          key[0].toUpperCase() + key.slice(1),
          () => {
            if (tab.getAttribute("aria-disabled") !== "true")
              switchTab(run, key);
          },
          "tab",
          { role: "tab", id: "tab-" + key, "aria-controls": "panel-" + key },
        );
        tab.dataset.view = key;
        run.nodes.tabs.append(tab);
      }
      const unavailable = ["changes", "receipt"].includes(key) && !run.receipt;
      tab.setAttribute("aria-disabled", String(unavailable));
      tab.title = unavailable ? "Available when the run finishes" : "";
      tab.setAttribute("aria-selected", String(run.view === key));
      tab.tabIndex = run.view === key || unavailable ? 0 : -1;
      if (key === "evidence") {
        const n = new Set(liveEvidence(run).map((e) => e.obligation_id)).size,
          m = contractOf(run)?.obligations?.length;
        tab.textContent = "Evidence" + (m !== undefined ? ` ${n}/${m}` : "");
      }
      if (key === "changes" && run.receipt)
        tab.textContent = `Changes (${run.receipt.changes.length})`;
    }
    for (const tab of [...run.nodes.tabs.children])
      if (!keys.includes(tab.dataset.view)) tab.remove();
  }
  function switchTab(run, view, write = true) {
    saveScroll();
    run.view = view;
    if (write) {
      const url = new URL(location.href);
      url.searchParams.set("view", view);
      history.replaceState({}, "", url.pathname + url.search);
    }
    let panel = run.panels.get(view);
    if (!panel) {
      panel = el("section", "tab-panel", null, {
        role: "tabpanel",
        id: "panel-" + view,
        "aria-labelledby": "tab-" + view,
      });
      run.panels.set(view, panel);
      run.nodes.content.append(panel);
      if (view === "trace") renderTrace(run, panel);
      if (view === "changes") renderChanges(run, panel);
      if (view === "evidence") renderEvidence(run, panel);
      if (view === "receipt") renderReceipt(run, panel);
      if (view === "answer") renderAnswer(run, panel);
    }
    for (const [key, p] of run.panels) p.hidden = key !== view;
    run.panel = panel;
    updateTabs(run);
    const offset = state.scrolls.get(run.id + ":" + view);
    if (offset !== undefined)
      (view === "trace" ? run.trace.scroller : panel).scrollTop = offset;
    if (view === "changes")
      requestAnimationFrame(() => {
        updateDiffMode();
        const file = new URLSearchParams(location.search).get("file");
        if (file) selectFile(run, file, false);
      });
  }
  function traceKind(event) {
    const type = event.event?.type,
      a = event.event?.data || {};
    if (
      ["checkpoint_created", "effect_prepared", "effect_completed"].includes(
        type,
      )
    )
      return "low";
    if (type === "evidence_recorded" || a.actor === "verifier")
      return "evidence";
    if (
      (type === "action_completed" &&
        (a.succeeded === false ||
          WARNING_ACTIONS.has(a.action) ||
          WARNING_TEXT.test(a.summary || ""))) ||
      (type === "note_recorded" && WARNING_TEXT.test(a.message || ""))
    )
      return "warnings";
    if (a.actor?.startsWith("model:")) return "models";
    if (a.actor?.startsWith("tool:")) return "tools";
    return "other";
  }
  function renderTrace(run, panel) {
    const toolbar = el("div", "trace-toolbar"),
      chips = el("div", "segments", null, { "aria-label": "Trace filters" }),
      search = el("input", "", null, {
        type: "search",
        placeholder: "Search trace /",
        "aria-label": "Search trace",
      }),
      low = el("label"),
      lowInput = el("input", "", null, { type: "checkbox" }),
      wrap = button(
        "Wrap",
        () => {
          setPref("wrapTrace", !prefs.wrapTrace);
          wrap.setAttribute("aria-pressed", String(prefs.wrapTrace));
        },
        "button",
        { "aria-pressed": prefs.wrapTrace },
      ),
      follow = el("span", "caption", "Following"),
      exportButton = button(
        "Copy visible rows",
        (e) => {
          const rows = [...run.trace.rows.values()]
            .filter((r) => !r.hidden)
            .map((r) => r.dataset.export);
          copy(rows.join("\n"), e.currentTarget);
        },
        "button",
      ),
      layout = el("div", "trace-layout"),
      scroller = el("div", "trace-scroller", null, {
        tabindex: "0",
        "aria-label": "Trace events",
      }),
      feed = el("div", "trace-feed"),
      index = el("nav", "turn-index", null, { "aria-label": "Turn index" }),
      pill = button("", () => followTrace(run, true), "button follow-pill", {
        hidden: "",
      });
    append(low, lowInput, el("span", "", "Low-level events"));
    append(toolbar, chips, search, low, wrap, follow, exportButton);
    scroller.append(feed);
    append(layout, scroller, index, pill);
    append(panel, toolbar, layout);
    const trace = (run.trace = {
      toolbar,
      chips,
      search,
      lowInput,
      scroller,
      feed,
      index,
      pill,
      followLabel: follow,
      rows: new Map(),
      phases: [],
      turns: new Map(),
      pending: new Map(),
      filter: "all",
      query: "",
      following: true,
      newEvents: 0,
      currentPhase: null,
      currentTurn: null,
      lastAnnounced: 0,
    });
    scroller.classList.toggle("no-wrap", !prefs.wrapTrace);
    for (const [key, label] of [
      ["all", "All"],
      ["models", "Model turns"],
      ["tools", "Tools"],
      ["warnings", "Warnings & failures"],
      ["evidence", "Evidence"],
    ]) {
      const b = button(
        label,
        () => {
          trace.filter = key;
          applyTraceFilter(run);
        },
        "",
        { "aria-pressed": key === "all" },
      );
      b.dataset.kind = key;
      chips.append(b);
    }
    roving(chips, "button");
    search.oninput = () => {
      trace.query = search.value.toLowerCase();
      applyTraceFilter(run);
    };
    search.onkeydown = (e) => {
      if (e.key === "Escape") {
        search.value = "";
        trace.query = "";
        applyTraceFilter(run);
      }
    };
    lowInput.onchange = () => applyTraceFilter(run);
    const unfollow = () => {
      trace.following = false;
      updateFollowPill(run);
    };
    scroller.addEventListener(
      "wheel",
      (e) => {
        if (e.deltaY < 0) unfollow();
      },
      { passive: true },
    );
    scroller.addEventListener("touchmove", unfollow, { passive: true });
    scroller.addEventListener("pointerdown", (e) => {
      if (e.offsetX >= scroller.clientWidth - 18) unfollow();
    });
    scroller.addEventListener("keydown", (e) => {
      if (["PageUp", "Home", "ArrowUp"].includes(e.key)) unfollow();
      if (e.key === "End") {
        e.preventDefault();
        followTrace(run, true);
      }
    });
    scroller.addEventListener("focusin", (e) => {
      if (e.target !== scroller) unfollow();
    });
    scroller.addEventListener("scroll", () => {
      if (
        scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight <=
          48 &&
        !window.getSelection()?.toString()
      ) {
        trace.following = true;
        trace.newEvents = 0;
        updateFollowPill(run);
      }
    });
    trace.selectionListener = () => {
      const s = window.getSelection();
      if (s?.toString() && feed.contains(s.anchorNode)) unfollow();
    };
    document.addEventListener("selectionchange", trace.selectionListener);
    for (const event of run.events) appendTraceEvent(run, event);
    applyTraceFilter(run, false);
    requestAnimationFrame(() => followTrace(run, false));
  }
  function makePhase(run, name, event) {
    const trace = run.trace,
      section = el("section", "trace-phase"),
      id = "phase-" + event.sequence,
      head = button(
        "",
        () => {
          const collapsed = !body.hidden;
          body.hidden = collapsed;
          head.setAttribute("aria-expanded", String(!collapsed));
          state.phaseCollapsed.set(run.id + ":" + event.sequence, collapsed);
          trace.following = false;
          updateFollowPill(run);
        },
        "phase-heading",
        { id, "aria-expanded": "true" },
      ),
      body = el("div");
    append(
      head,
      el("span", "", name),
      el("span", "caption", offsetText(run, event)),
    );
    section.setAttribute("aria-labelledby", id);
    append(section, head, body);
    trace.feed.append(section);
    const phase = { name, section, head, body, event, rows: 0 };
    trace.phases.push(phase);
    trace.currentPhase = phase;
    trace.currentTurn = null;
    const remembered = state.phaseCollapsed.get(run.id + ":" + event.sequence);
    if (remembered) {
      body.hidden = true;
      head.setAttribute("aria-expanded", "false");
    }
    return phase;
  }
  function offsetText(run, event) {
    const first = new Date(run.events[0]?.timestamp).getTime(),
      now = new Date(event.timestamp).getTime();
    return Number.isFinite(first) && Number.isFinite(now)
      ? "+" +
          formatUsage(Math.max(0, (now - first) / 1000), { kind: "duration" })
            .text
      : "—";
  }
  function traceContainer(run, event) {
    const trace = run.trace;
    if (!trace.currentPhase) makePhase(run, "Contract", event);
    if (trace.currentTurn) return trace.currentTurn.list;
    let list = trace.currentPhase.body.querySelector(":scope > ol");
    if (!list) {
      list = el("ol", "trace-rows");
      trace.currentPhase.body.append(list);
    }
    return list;
  }
  function appendTraceEvent(run, event) {
    const trace = run.trace;
    if (trace.rows.has(event.sequence)) return;
    const type = event.event?.type,
      data = event.event?.data || {},
      kind = traceKind(event);
    if (type === "state_changed") {
      const phase = PHASE_STATE[data.to];
      if (phase !== undefined && trace.currentPhase?.name !== PHASES[phase]) {
        const old = trace.currentPhase;
        if (
          old &&
          trace.following &&
          !window.getSelection()?.toString() &&
          old.rows > 12 &&
          !state.phaseCollapsed.has(run.id + ":" + old.event.sequence)
        ) {
          old.body.hidden = true;
          old.head.setAttribute("aria-expanded", "false");
          const count = run.events.filter(
            (e) =>
              e.sequence >= old.event.sequence &&
              e.sequence < event.sequence &&
              e.event?.data?.actor?.startsWith("model:"),
          ).length;
          old.head.firstChild.textContent = old.name + ` · ${count} turns`;
          old.head.lastChild.textContent = formatUsage(
            (new Date(event.timestamp) - new Date(old.event.timestamp)) / 1000,
            { kind: "duration" },
          ).text;
        }
        makePhase(run, PHASES[phase], event);
      }
    }
    if (
      type === "action_completed" &&
      data.actor?.startsWith("model:") &&
      data.action === "invoke"
    ) {
      const turn = data.attributes?.turn ?? trace.turns.size + 1,
        section = el("section", "turn-group", null, {
          "aria-labelledby": "turn-" + event.sequence,
        }),
        heading = el("div", "turn-heading", null, {
          id: "turn-" + event.sequence,
        }),
        usage = el("span", "turn-usage"),
        list = el("ol", "trace-rows");
      append(
        heading,
        el("span", "", `Turn ${turn} · ${data.actor.slice(6)}`),
        usage,
      );
      const attrs = data.attributes || {};
      usage.textContent = `${formatUsage(attrs.input_tokens).text} in · ${formatUsage(attrs.cached_input_tokens).text} cached · ${formatUsage(attrs.output_tokens).text} out · ${formatUsage(numeric(attrs["provider.time_to_first_byte_ms"]) === null ? null : Number(attrs["provider.time_to_first_byte_ms"]) / 1000, { kind: "duration" }).text} to first byte`;
      if (attrs.finish_reason)
        heading.append(el("span", "risk", attrs.finish_reason));
      append(section, heading, list);
      if (!trace.currentPhase) makePhase(run, "Contract", event);
      trace.currentPhase.body.append(section);
      trace.currentTurn = { section, list, turn, event };
      trace.turns.set(event.sequence, trace.currentTurn);
      const jump = button(
        "T" + turn,
        () => jumpToEvent(run, event.sequence),
        "",
        { title: "Jump to turn " + turn },
      );
      jump.append(
        el("meter", "", null, {
          min: 0,
          max: 300,
          value: Math.min(300, data.duration_ms / 1000),
          "aria-label": "Turn duration",
        }),
      );
      trace.index.append(jump);
    }
    if (type === "effect_prepared") {
      const row = el("li", "trace-row in-flight");
      append(
        row,
        append(
          el("div", "row"),
          icon("loader-circle", "spin"),
          el("span", "mono", data.tool),
          el("span", "caption", "In flight — elapsed locally"),
          el("span", "mono-meta", null, { "aria-hidden": "true" }),
        ),
      );
      row.dataset.started = String(Date.now());
      trace.pending.set(data.call_id, row);
      traceContainer(run, event).append(row);
    }
    if (type === "effect_completed") {
      trace.pending.get(data.call_id)?.remove();
      trace.pending.delete(data.call_id);
    }
    const row = el(
      "li",
      "trace-row" +
        (kind === "low" ? " low-event" : "") +
        (data.succeeded === false
          ? " failed"
          : kind === "warnings"
            ? " warning"
            : data.actor === "context"
              ? " context"
              : ""),
    );
    row.dataset.kind = kind;
    row.dataset.sequence = event.sequence;
    row.dataset.search = JSON.stringify([
      data.summary,
      data.message,
      data.actor,
      data.action,
      data.attributes,
      data.declared_effects,
      data.observed_effects,
    ]).toLowerCase();
    const summary =
      data.summary ||
      data.message ||
      (type === "state_changed"
        ? `${data.from} → ${data.to}`
        : type === "contract_registered"
          ? "Task contract registered"
          : type === "evidence_recorded"
            ? data.summary
            : type.replaceAll("_", " "));
    row.dataset.search += " " + String(summary).toLowerCase();
    row.dataset.export = `${offsetText(run, event)} [${data.actor || type}] ${data.action || ""} ${summary}`;
    if (kind === "low")
      row.append(
        el(
          "span",
          "",
          `${type.replaceAll("_", " ")} · ${String(event.hash || "").slice(0, 10)}`,
        ),
      );
    else {
      const head = button(
          "",
          () => {
            details.hidden = !details.hidden;
            head.setAttribute("aria-expanded", String(!details.hidden));
            trace.following = false;
            updateFollowPill(run);
          },
          "trace-row-head",
          { "aria-expanded": "false" },
        ),
        details = el("div", "trace-details", null, { hidden: "" });
      let glyph = data.actor?.startsWith("model:")
        ? "cpu"
        : data.actor?.startsWith("tool:")
          ? toolGlyph(data.actor.slice(5))
          : kind === "evidence"
            ? "shield-check"
            : kind === "warnings"
              ? "triangle-alert"
              : type.includes("policy") || type.includes("approval")
                ? "shield-check"
                : "circle";
      if (data.succeeded === false) glyph = "octagon-alert";
      append(
        head,
        icon(glyph, "inline"),
        el(
          "span",
          "tool-name",
          data.actor?.startsWith("tool:")
            ? data.actor.slice(5)
            : data.actor?.startsWith("model:")
              ? "invoke"
              : data.action || type.replaceAll("_", " "),
        ),
        el("span", "trace-row-summary", summary),
        usageNode(
          data.duration_ms > 0 ? data.duration_ms / 1000 : null,
          {
            kind: "duration",
            reason: "Timing was not recorded for this event.",
          },
          "trace-duration",
        ),
      );
      if (data.succeeded === false)
        head.append(el("span", "sr-only", "failed"));
      const risk = data.attributes?.risk;
      if (risk && risk !== "readonly")
        head.append(
          el(
            "span",
            "risk",
            risk === "workspacemutation"
              ? "✎ writes"
              : risk === "hostexecution"
                ? "▣ host exec"
                : risk,
          ),
        );
      const attrs = { ...(data.attributes || {}) };
      if (data.arguments_digest) attrs.arguments_digest = data.arguments_digest;
      const table = el("dl", "attributes");
      for (const [key, value] of Object.entries(attrs))
        append(table, el("dt", "", key), el("dd", "", value));
      details.append(table);
      for (const [label, effects] of [
        ["Declared effects", data.declared_effects],
        ["Observed effects", data.observed_effects],
      ])
        if (effects?.length) {
          details.append(el("p", "caption", label));
          const chips = el("div", "chips");
          for (const effect of effects) {
            const path = String(effect).match(
              /^fs\.(?:changed|write):(.+)$/,
            )?.[1];
            chips.append(
              path && run.receipt?.changes.some((c) => c.path === path)
                ? button(
                    effect,
                    () => {
                      switchTab(run, "changes");
                      selectFile(run, path);
                    },
                    "chip",
                  )
                : el("span", "chip", effect),
            );
          }
          details.append(chips);
        }
      if (attrs.output_truncated === "true")
        details.append(
          el("p", "caption", "Output was truncated by the engine."),
        );
      if (kind === "models")
        details.append(
          el(
            "p",
            "caption",
            "The trace records model activity and byte counts. It contains no model reasoning text or response body.",
          ),
        );
      if (kind === "tools")
        details.append(
          el(
            "p",
            "caption",
            "Tool output bodies are not present in this trace.",
          ),
        );
      if (kind === "evidence") {
        if (type === "evidence_recorded")
          details.append(
            gradeMeter(data.grade),
            el("span", "evidence-status", data.status),
          );
        details.append(
          button(
            "View in Evidence",
            () => switchTab(run, "evidence"),
            "button link",
          ),
        );
        if (/not authorized|process permission/i.test(summary))
          details.append(
            el("p", "caption", "Commands are disabled for this run."),
          );
      }
      if (!table.children.length && !details.children.length)
        details.append(
          el("p", "caption", "No additional fields were recorded."),
        );
      append(row, head, details);
    }
    traceContainer(run, event).append(row);
    trace.rows.set(event.sequence, row);
    if (trace.currentPhase) trace.currentPhase.rows++;
    if (run.loadedOnce) row.classList.add("flash");
  }
  function toolGlyph(tool) {
    return /read|list/.test(tool)
      ? "file-text"
      : /search_code_graph|search_change_impact/.test(tool)
        ? "network"
        : /search/.test(tool)
          ? "search"
          : /edit|write|replace|patch/.test(tool)
            ? "pencil"
            : /remove/.test(tool)
              ? "file-x"
              : /process|shell/.test(tool)
                ? "terminal"
                : /memory/.test(tool)
                  ? "database"
                  : "wrench";
  }
  function applyTraceFilter(run, notify = true) {
    const t = run.trace;
    if (!t) return;
    let matches = 0;
    const counts = {};
    for (const row of t.rows.values()) {
      counts[row.dataset.kind] = (counts[row.dataset.kind] || 0) + 1;
      const visible =
        (row.dataset.kind !== "low" || t.lowInput.checked) &&
        (t.filter === "all" || row.dataset.kind === t.filter) &&
        (!t.query || row.dataset.search.includes(t.query));
      row.hidden = !visible;
      if (visible) matches++;
      const summary = row.querySelector(".trace-row-summary");
      if (summary && summary.dataset.query !== t.query) {
        const raw = summary.dataset.original ?? summary.textContent;
        summary.dataset.original = raw;
        summary.dataset.query = t.query;
        summary.replaceChildren();
        if (t.query) {
          let index = 0,
            at;
          const lower = raw.toLowerCase();
          while ((at = lower.indexOf(t.query, index)) !== -1) {
            summary.append(
              document.createTextNode(raw.slice(index, at)),
              el("mark", "", raw.slice(at, at + t.query.length)),
            );
            index = at + t.query.length;
          }
          summary.append(document.createTextNode(raw.slice(index)));
        } else summary.textContent = raw;
      }
    }
    for (const b of t.chips.children) {
      b.setAttribute("aria-pressed", String(b.dataset.kind === t.filter));
      b.tabIndex = b.dataset.kind === t.filter ? 0 : -1;
      let badge = b.querySelector(".count");
      if (!badge) {
        badge = el("span", "count");
        b.append(badge);
      }
      badge.textContent =
        b.dataset.kind === "all" ? t.rows.size : counts[b.dataset.kind] || 0;
    }
    if (notify) announce(matches + " rows match");
  }
  function followTrace(run, user) {
    const t = run.trace;
    if (!t) return;
    t.following = true;
    t.newEvents = 0;
    updateFollowPill(run);
    t.scroller.scrollTo({
      top: t.scroller.scrollHeight,
      behavior:
        user &&
        prefs.motion !== "reduce" &&
        !matchMedia("(prefers-reduced-motion: reduce)").matches
          ? "smooth"
          : "instant",
    });
  }
  function updateFollowPill(run) {
    const t = run.trace;
    t.pill.hidden = t.following || !t.newEvents;
    t.pill.textContent = `↓ ${t.newEvents} new events · Follow`;
    t.followLabel.textContent = t.following ? "Following" : "Follow paused";
  }
  function jumpToEvent(run, sequence) {
    const t = run.trace;
    if (!t) return;
    const row = t.rows.get(sequence) || t.turns.get(sequence)?.section;
    if (!row) return;
    for (const phase of t.phases)
      if (phase.section.contains(row)) {
        phase.body.hidden = false;
        phase.head.setAttribute("aria-expanded", "true");
      }
    row.hidden = false;
    t.following = false;
    row.scrollIntoView({ block: "center", behavior: "instant" });
    row.classList.remove("flash");
    requestAnimationFrame(() => row.classList.add("flash"));
    updateFollowPill(run);
  }
  function gradeMeter(grade) {
    const label = grade ? grade.replaceAll("_", " ") : "not recorded",
      n =
        ["unverified", "model_assessed", "observed", "deterministic"].indexOf(
          grade,
        ) + 1,
      span = el("span", "grade"),
      bars = el("span", "grade-bars", null, { "aria-hidden": "true" });
    for (let i = 0; i < 4; i++) bars.append(el("i", i < n ? "filled" : ""));
    append(span, bars, el("span", "", label));
    return span;
  }
  async function loadDiff(run) {
    try {
      const diff = await api(`/api/runs/${run.id}/diff`);
      if (state.current !== run) return;
      run.diff = diff;
      run.diffError = null;
      if (run.panels.has("changes")) populateChanges(run);
      if (run.panels.has("receipt"))
        renderReceipt(run, run.panels.get("receipt"));
    } catch (error) {
      if (state.current !== run) return;
      run.diffError = error;
      if (run.panels.has("changes")) populateChanges(run);
    }
  }
  function freshnessNotice(run) {
    const f = freshness(run.events),
      e = liveEvidence(run),
      s = evidenceSummary(e);
    if (f.stale)
      return `Evidence predates the last file change (last write at event #${f.lastWrite}, latest evidence at #${f.lastEvidence}). Derived from trace order.`;
    if (s.failed)
      return `${s.failed} check${s.failed === 1 ? "" : "s"} failed. See Evidence.`;
    const checks = e.filter((x) =>
      ["test", "build", "diagnostic", "process_observation"].includes(x.kind),
    );
    if (
      !checks.length ||
      checks.every((x) => ["inconclusive", "skipped"].includes(x.status))
    ) {
      const disabled =
        e.some((x) => /process permission|not authorized/i.test(x.summary)) ||
        actions(run.events).some(
          (x) =>
            x.event.data.actor === "verifier" &&
            /not authorized/i.test(x.event.data.summary),
        );
      return (
        "No check ran against this candidate." +
        (disabled ? " Commands were disabled for this run." : "")
      );
    }
    return "";
  }
  function verdictStrip(run) {
    const strip = el("div", "verdict-strip"),
      main = el("div", "verdict-main"),
      files = parseUnifiedDiff(
        run.diff?.unified_diff,
        run.receipt?.changes || [],
      ),
      added = files.reduce((n, f) => n + f.added, 0),
      removed = files.reduce((n, f) => n + f.removed, 0),
      s = evidenceSummary(liveEvidence(run));
    append(
      main,
      el(
        "span",
        "mono",
        `${run.receipt?.changes.length ?? 0} files${run.diff ? ` +${added} −${removed}` : " · line counts not loaded"}`,
      ),
      el(
        "span",
        "",
        `✓ ${s.passed} passed · ✕ ${s.failed} failed · ? ${s.inconclusive} inconclusive · – ${s.skipped} skipped`,
      ),
      el("span", "", "Highest grade:"),
      gradeMeter(s.grade),
    );
    strip.append(main);
    const notice = freshnessNotice(run);
    if (notice)
      append(
        strip,
        append(
          el("div", "verdict-notice"),
          el("span", "", notice + " "),
          button(
            "Open evidence",
            () => switchTab(run, "evidence"),
            "button link",
          ),
        ),
      );
    return strip;
  }
  function renderChanges(run, panel) {
    const verdict = verdictStrip(run),
      partial = el("div"),
      layout = el("div", "changes-layout"),
      files = el("nav", "files-pane", null, { "aria-label": "Changed files" }),
      separator = el("div", "pane-separator", null, {
        role: "separator",
        tabindex: 0,
        "aria-label": "Resize files pane",
        "aria-orientation": "vertical",
        "aria-valuemin": 220,
        "aria-valuemax": 480,
        "aria-valuenow": prefs.filesPaneWidth,
      }),
      diffPane = el("div", "diff-pane"),
      toolbar = el("div", "diff-toolbar"),
      filesToggle = button(
        "Files ▾",
        () => {
          files.classList.toggle("open");
          filesToggle.setAttribute(
            "aria-expanded",
            String(files.classList.contains("open")),
          );
        },
        "button files-mobile-toggle",
        { "aria-expanded": "false" },
      ),
      mode = button(
        "Split",
        () =>
          setPref(
            "diffMode",
            effectiveDiffMode(run) === "split" ? "unified" : "split",
          ),
        "button",
        { id: "diff-mode" },
      ),
      wrap = button(
        "Wrap",
        () => setPref("wrapDiff", !prefs.wrapDiff),
        "button",
        { "aria-pressed": prefs.wrapDiff },
      ),
      body = el("div", "diff-body"),
      decision = el("div", "decision-bar");
    append(toolbar, filesToggle, el("span", "spacer"), mode, wrap);
    append(diffPane, toolbar, body);
    append(layout, files, separator, diffPane);
    append(panel, partial, verdict, layout, decision);
    run.changesUI = {
      panel,
      verdict,
      partial,
      files,
      separator,
      diffPane,
      mode,
      wrap,
      body,
      decision,
      fileNodes: new Map(),
      rows: new Map(),
      observer: null,
    };
    const resize = (value) => {
      setPref("filesPaneWidth", Math.max(220, Math.min(480, value)));
      separator.setAttribute("aria-valuenow", String(prefs.filesPaneWidth));
      updateDiffMode();
    };
    separator.addEventListener("keydown", (e) => {
      if (["ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key)) {
        e.preventDefault();
        resize(
          e.key === "Home"
            ? 220
            : e.key === "End"
              ? 480
              : prefs.filesPaneWidth + (e.key === "ArrowRight" ? 16 : -16),
        );
      }
    });
    separator.ondblclick = () => resize(300);
    separator.onpointerdown = (e) => {
      separator.setPointerCapture(e.pointerId);
      const start = e.clientX,
        width = prefs.filesPaneWidth;
      const move = (e) => resize(width + e.clientX - start),
        up = () => {
          separator.removeEventListener("pointermove", move);
          separator.removeEventListener("pointerup", up);
        };
      separator.addEventListener("pointermove", move);
      separator.addEventListener("pointerup", up);
    };
    run.changesUI.resizeObserver = new ResizeObserver(() => updateDiffMode());
    run.changesUI.resizeObserver.observe(diffPane);
    populateChanges(run);
  }
  function changeStatus(c) {
    return c.before_digest == null
      ? "Added"
      : c.after_digest == null
        ? "Deleted"
        : "Modified";
  }
  function changesTable(changes) {
    const scroll = el("div", "table-scroll"),
      table = el("table", "data-table"),
      head = el("tr");
    for (const label of ["Path", "Status", "Bytes", "Before / after", "Mode"])
      head.append(el("th", "", label, { scope: "col" }));
    const thead = el("thead"),
      tbody = el("tbody");
    thead.append(head);
    for (const c of changes) {
      const row = el("tr");
      append(
        row,
        append(el("td", ""), copyText(c.path, "Copy path")),
        el("td", "", changeStatus(c)),
        el("td", "mono", `+${c.bytes_added} −${c.bytes_removed}`),
        append(
          el("td", ""),
          hashText(c.before_digest, "before digest"),
          el("span", "", " → "),
          hashText(c.after_digest, "after digest"),
        ),
        el(
          "td",
          "mono",
          c.before_unix_mode !== c.after_unix_mode
            ? `${c.before_unix_mode ?? "—"} → ${c.after_unix_mode ?? "—"}`
            : "—",
          {
            title:
              c.before_unix_mode === c.after_unix_mode
                ? "No mode change"
                : "Recorded Unix modes",
          },
        ),
      );
      tbody.append(row);
    }
    append(table, thead, tbody);
    scroll.append(table);
    return scroll;
  }
  function viewedKey(run, file) {
    return (
      run.id +
      ":" +
      file.path +
      ":" +
      String(file.change?.after_digest || "deleted").slice(0, 8)
    );
  }
  function markViewed(run, file, value) {
    state.viewed[viewedKey(run, file)] = value;
    save("viewedFiles", state.viewed);
    const row = run.changesUI.rows.get(file.path);
    if (row) {
      row.querySelector(".viewed-marker").textContent = value ? "✓" : "";
      row.setAttribute("aria-label", fileLabel(run, file));
    }
    updateDecision(run);
  }
  function fileLabel(run, file) {
    return `${file.path}, ${changeStatus(file.change || {})}, ${file.added} additions, ${file.removed} deletions${state.viewed[viewedKey(run, file)] ? ", viewed" : ""}`;
  }
  function populateChanges(run) {
    const ui = run.changesUI;
    if (!ui) return;
    ui.verdict.replaceWith((ui.verdict = verdictStrip(run)));
    const key = runState(currentRecord(run));
    ui.partial.replaceChildren();
    if (["stopped", "failed"].includes(key))
      ui.partial.append(
        banner(
          "warn",
          "Partial — this run did not finish.",
          "These changes cannot be applied from this run.",
        ),
      );
    ui.observer?.disconnect();
    ui.files.replaceChildren();
    ui.body.replaceChildren();
    ui.fileNodes.clear();
    ui.rows.clear();
    if (!run.diff) {
      if (run.diffError) {
        ui.body.append(
          errorBlock(
            key === "discarded"
              ? "The engine can no longer render this diff."
              : "Unable to load the diff.",
            run.diffError,
            () => loadDiff(run),
          ),
          el(
            "p",
            "diff-empty",
            `The receipt still lists ${run.receipt?.changes.length || 0} changed files.`,
          ),
          changesTable(run.receipt?.changes || []),
        );
      } else ui.body.append(el("p", "diff-empty", "Loading candidate diff…"));
      updateDecision(run);
      return;
    }
    const files = (run.files = parseUnifiedDiff(
        run.diff.unified_diff,
        run.receipt?.changes || [],
      )),
      added = files.reduce((n, f) => n + f.added, 0),
      removed = files.reduce((n, f) => n + f.removed, 0);
    ui.files.append(
      el(
        "div",
        "files-pane-head",
        `${files.length} files · +${added} −${removed}`,
      ),
    );
    let filter;
    if (files.length > 8) {
      filter = el("input", "", null, {
        type: "search",
        placeholder: "Filter files",
        "aria-label": "Filter changed files",
      });
      ui.files.append(filter);
      filter.oninput = () => {
        for (const [path, row] of ui.rows)
          row.hidden = !path.toLowerCase().includes(filter.value.toLowerCase());
      };
    }
    const selected =
        new URLSearchParams(location.search).get("file") || files[0]?.path,
      totalLines = files.reduce(
        (n, f) => n + f.hunks.reduce((n, h) => n + h.lines.length, 0),
        0,
      );
    let directory = null;
    for (const file of files) {
      const dir = file.path.includes("/")
        ? file.path.slice(0, file.path.lastIndexOf("/"))
        : ".";
      if (directory !== dir) {
        directory = dir;
        const dirButton = button(
          dir,
          () => {
            for (const [path, row] of ui.rows)
              if (
                (path.includes("/")
                  ? path.slice(0, path.lastIndexOf("/"))
                  : ".") === dir
              )
                row.hidden = !row.hidden;
          },
          "files-directory",
          { title: dir },
        );
        ui.files.append(dirButton);
      }
      const row = button("", () => selectFile(run, file.path), "file-row", {
        "aria-label": fileLabel(run, file),
      });
      row.tabIndex = file.path === selected ? 0 : -1;
      const basename = baseName(file.path);
      append(
        row,
        el("span", "status-letter", file.status || "M"),
        el(
          "span",
          "file-basename",
          basename.length > 40
            ? basename.slice(0, 20) + "…" + basename.slice(-16)
            : basename,
          { title: file.path },
        ),
        miniBar(file.added, file.removed),
        el(
          "span",
          "viewed-marker",
          state.viewed[viewedKey(run, file)] ? "✓" : "",
        ),
      );
      ui.files.append(row);
      ui.rows.set(file.path, row);
      const section = el("section", "file-diff", null, {
          "aria-label": file.path,
        }),
        header = el("header", "file-header"),
        toggle = button(
          "⌄",
          () => {
            body.hidden = !body.hidden;
            toggle.textContent = body.hidden ? "›" : "⌄";
            toggle.setAttribute("aria-expanded", String(!body.hidden));
            if (!body.hidden && !body.dataset.built)
              buildFileDiff(run, file, body);
          },
          "icon-button",
          {
            "aria-label": "Collapse diff for " + file.path,
            "aria-expanded": "true",
          },
        ),
        label = el("label"),
        checkbox = el("input", "", null, { type: "checkbox" });
      checkbox.checked = !!state.viewed[viewedKey(run, file)];
      checkbox.onchange = () => markViewed(run, file, checkbox.checked);
      append(label, checkbox, el("span", "", "Viewed"));
      append(
        header,
        toggle,
        el("span", "path", file.path),
        iconButton("copy", "Copy path " + file.path, (e) =>
          copy(file.path, e.currentTarget),
        ),
        el("span", "status-letter", file.status || "M"),
        el("span", "file-counts", `+${file.added} −${file.removed}`),
        label,
      );
      const body = el("div", "file-diff-body"),
        prov = provenance(run.events, file.path);
      section.append(header);
      if (prov.actions) {
        const line = el(
          "div",
          "file-provenance",
          `Written by ${prov.actions} action${prov.actions === 1 ? "" : "s"}${prov.turns.length ? " in turns " + prov.turns.join(" and ") : ""} · `,
        );
        line.append(
          button(
            "Show in trace",
            () => {
              switchTab(run, "trace");
              jumpToEvent(run, prov.sequences[0]);
            },
            "button link",
          ),
        );
        section.append(line);
      }
      section.append(body);
      const lineCount = file.hunks.reduce((n, h) => n + h.lines.length, 0),
        collapsed =
          (file.added + file.removed > 400 || totalLines > 3000) &&
          file.path !== selected;
      if (collapsed) {
        body.hidden = true;
        toggle.textContent = "›";
        toggle.setAttribute("aria-expanded", "false");
        const note = el(
          "div",
          "diff-empty",
          `Large diff · ${lineCount.toLocaleString("en-US")} lines. `,
        );
        note.append(
          button("Show", () => {
            body.hidden = false;
            toggle.setAttribute("aria-expanded", "true");
            toggle.textContent = "⌄";
            note.remove();
            buildFileDiff(run, file, body);
          }),
        );
        section.append(note);
      } else buildFileDiff(run, file, body);
      ui.body.append(section);
      ui.fileNodes.set(file.path, { section, body, file, checkbox, toggle });
    }
    ui.files.addEventListener("keydown", (e) => {
      const rows = [...ui.rows.values()].filter((r) => !r.hidden),
        i = rows.indexOf(document.activeElement);
      if (i < 0) return;
      if (["ArrowDown", "ArrowUp"].includes(e.key)) {
        e.preventDefault();
        const next =
          rows[
            Math.max(
              0,
              Math.min(rows.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)),
            )
          ];
        next.focus();
        next.click();
      }
      if (e.key === " ") {
        e.preventDefault();
        const file = files.find(
          (f) => ui.rows.get(f.path) === document.activeElement,
        );
        if (file) {
          const val = !state.viewed[viewedKey(run, file)];
          markViewed(run, file, val);
          ui.fileNodes.get(file.path).checkbox.checked = val;
        }
      }
    });
    ui.observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries)
          if (entry.isIntersecting) {
            const file = files.find(
              (f) => ui.fileNodes.get(f.path).section === entry.target,
            );
            if (file) highlightFile(run, file.path);
          }
      },
      { root: ui.diffPane, rootMargin: "0px 0px -75% 0px", threshold: 0 },
    );
    for (const x of ui.fileNodes.values()) ui.observer.observe(x.section);
    updateDiffMode();
    highlightFile(run, selected);
    updateDecision(run);
  }
  function miniBar(added, removed) {
    const span = el("span", "mini-bar", null, { "aria-hidden": "true" }),
      total = added + removed;
    for (let i = 0; i < 5; i++)
      span.append(
        el(
          "i",
          total ? (i < Math.round((added / total) * 5) ? "add" : "del") : "",
        ),
      );
    return span;
  }
  function diffCell(text, cls, aria) {
    const cell = el("div", "diff-cell " + cls);
    if (cls.includes("gutter") || cls.includes("sign"))
      cell.setAttribute("aria-hidden", "true");
    if (cls.includes("code")) {
      if (aria) cell.append(el("span", "sr-only", aria));
      cell.append(el("code", "", text));
    } else if (text !== null && text !== undefined)
      cell.textContent = String(text);
    return cell;
  }
  function differingMiddle(a, b) {
    let start = 0,
      end = 0;
    while (start < Math.min(a.length, b.length) && a[start] === b[start])
      start++;
    while (
      end < Math.min(a.length, b.length) - start &&
      a[a.length - 1 - end] === b[b.length - 1 - end]
    )
      end++;
    if ((start + end) / Math.max(1, a.length, b.length) < 0.3) return null;
    return { start, end };
  }
  function highlightedCode(cell, text, middle, kind) {
    if (!middle) return;
    const code = cell.querySelector("code");
    code.replaceChildren(
      document.createTextNode(text.slice(0, middle.start)),
      el(
        "span",
        "diff-word " + kind,
        text.slice(
          middle.start,
          middle.end ? text.length - middle.end : undefined,
        ),
      ),
      document.createTextNode(middle.end ? text.slice(-middle.end) : ""),
    );
  }
  function buildFileDiff(run, file, body) {
    body.dataset.built = "true";
    body.replaceChildren();
    if (!file.hunks.length) {
      append(
        body,
        el("p", "diff-empty", "No textual diff."),
        append(
          el("div", "diff-empty"),
          el("span", "", "Before "),
          hashText(file.change?.before_digest, "before digest"),
          el("span", "", " → after "),
          hashText(file.change?.after_digest, "after digest"),
          file.modeChanged
            ? el(
                "p",
                "",
                `Mode changed: ${file.change?.before_unix_mode ?? "—"} → ${file.change?.after_unix_mode ?? "—"}`,
              )
            : null,
        ),
      );
      return;
    }
    const scroll = el("div", "diff-scroll"),
      grid = el("div", "diff-grid");
    scroll.append(grid);
    body.append(scroll);
    const split = effectiveDiffMode(run) === "split";
    grid.classList.toggle("split", split);
    grid.classList.toggle("wrap", prefs.wrapDiff);
    let previous = null;
    for (const hunk of file.hunks) {
      if (previous) {
        const gap = hunk.oldStart - (previous.oldStart + previous.oldCount);
        if (gap > 0)
          grid.append(el("div", "diff-gap", `⋯ ${gap} unchanged lines`));
      }
      grid.append(el("div", "hunk-heading", hunk.header));
      const lines = hunk.lines;
      for (let i = 0; i < lines.length; ) {
        const line = lines[i];
        if (line.kind === "note") {
          grid.append(el("div", "diff-gap", line.text));
          i++;
          continue;
        }
        if (line.kind === "context") {
          if (split)
            append(
              grid,
              diffCell(line.old, "gutter old"),
              diffCell(line.text, "code"),
              diffCell(line.new, "gutter new"),
              diffCell(line.text, "code"),
            );
          else
            append(
              grid,
              diffCell(line.old, "gutter old"),
              diffCell(line.new, "gutter new"),
              diffCell(" ", "sign"),
              diffCell(line.text, "code"),
            );
          i++;
          continue;
        }
        const dels = [],
          adds = [];
        while (i < lines.length && lines[i].kind === "del")
          dels.push(lines[i++]);
        while (i < lines.length && lines[i].kind === "add")
          adds.push(lines[i++]);
        if (!dels.length && !adds.length) {
          i++;
          continue;
        }
        for (let j = 0; j < Math.max(dels.length, adds.length); j++) {
          const d = dels[j],
            a = adds[j],
            middle = d && a ? differingMiddle(d.text, a.text) : null;
          if (split) {
            const dc = diffCell(
                d?.text || "",
                "code" + (d ? " del" : ""),
                d ? "removed: " : null,
              ),
              ac = diffCell(
                a?.text || "",
                "code" + (a ? " add" : ""),
                a ? "added: " : null,
              );
            append(
              grid,
              diffCell(d?.old, "gutter old" + (d ? " del" : "")),
              dc,
              diffCell(a?.new, "gutter new" + (a ? " add" : "")),
              ac,
            );
            if (d) highlightedCode(dc, d.text, middle, "del");
            if (a) highlightedCode(ac, a.text, middle, "add");
          } else {
            if (d) {
              const code = diffCell(d.text, "code del", "removed: ");
              append(
                grid,
                diffCell(d.old, "gutter old del"),
                append(
                  diffCell(null, "gutter new del", null),
                  el("span", "deleted-mobile-number", d.old),
                ),
                diffCell("−", "sign del"),
                code,
              );
              highlightedCode(code, d.text, middle, "del");
            }
            if (a) {
              const code = diffCell(a.text, "code add", "added: ");
              append(
                grid,
                diffCell(null, "gutter old add"),
                diffCell(a.new, "gutter new add"),
                diffCell("+", "sign add"),
                code,
              );
              highlightedCode(code, a.text, middle, "add");
            }
          }
        }
      }
      previous = hunk;
    }
    scroll.addEventListener("copy", (e) => {
      const selection = window.getSelection();
      if (!selection?.rangeCount) return;
      const fragment = selection.getRangeAt(0).cloneContents();
      for (const node of fragment.querySelectorAll(
        ".gutter,.sign,.sr-only,.hunk-heading,.diff-gap",
      ))
        node.remove();
      const codes = [...fragment.querySelectorAll("code")];
      if (codes.length) {
        e.preventDefault();
        e.clipboardData.setData(
          "text/plain",
          codes.map((c) => c.textContent).join("\n"),
        );
      }
    });
  }
  function effectiveDiffMode(run) {
    const width = run.changesUI?.diffPane.clientWidth || 0;
    if (width < 720) return "unified";
    const requested = new URL(location.href).searchParams.get("mode");
    const preferred = ["split", "unified"].includes(requested)
      ? requested
      : prefs.diffMode;
    return preferred === "auto"
      ? width >= 960
        ? "split"
        : "unified"
      : preferred;
  }
  function updateDiffMode() {
    const run = state.current,
      ui = run?.changesUI;
    if (!ui) return;
    const mode = effectiveDiffMode(run),
      width = ui.diffPane.clientWidth;
    disabled(ui.mode, width < 720 ? "Needs a wider pane" : null);
    ui.mode.textContent = mode === "split" ? "Unified" : "Split";
    ui.wrap.setAttribute("aria-pressed", String(prefs.wrapDiff));
    const changed = ui.mode.dataset.mode !== mode;
    ui.mode.dataset.mode = mode;
    for (const x of ui.fileNodes.values()) {
      if (changed && !x.body.hidden && x.body.dataset.built)
        buildFileDiff(run, x.file, x.body);
      for (const grid of x.body.querySelectorAll(".diff-grid"))
        grid.classList.toggle("wrap", prefs.wrapDiff);
    }
  }
  function highlightFile(run, path) {
    const ui = run.changesUI;
    if (!ui?.rows.has(path)) return;
    run.selectedFile = path;
    for (const [p, row] of ui.rows) {
      row.classList.toggle("selected", p === path);
      row.tabIndex = p === path ? 0 : -1;
    }
    const url = new URL(location.href);
    if (run.view === "changes" && url.searchParams.get("file") !== path) {
      url.searchParams.set("file", path);
      history.replaceState({}, "", url.pathname + url.search);
    }
  }
  function selectFile(run, path, scroll = true) {
    const node = run.changesUI?.fileNodes.get(path);
    if (!node) return;
    highlightFile(run, path);
    if (node.body.hidden) {
      node.body.hidden = false;
      node.toggle.setAttribute("aria-expanded", "true");
      if (!node.body.dataset.built) buildFileDiff(run, node.file, node.body);
    }
    if (scroll)
      node.section.scrollIntoView({ block: "start", behavior: "instant" });
    if (innerWidth < 900) run.changesUI.files.classList.remove("open");
  }
  function updateDecision(run) {
    const ui = run.changesUI;
    if (!ui) return;
    const key = runState(currentRecord(run));
    ui.decision.hidden = key !== "review";
    if (key !== "review") return;
    const changes = run.receipt?.changes || [],
      viewed = (run.files || []).filter(
        (f) => state.viewed[viewedKey(run, f)],
      ).length;
    ui.decision.replaceChildren();
    const copy = append(
      el("div", "decision-copy"),
      el(
        "div",
        "",
        `${changes.length} file${changes.length === 1 ? "" : "s"} · Candidate is isolated from ${state.bootstrap?.workspace || "the workspace"} until applied.`,
      ),
      el(
        "div",
        "caption",
        `${viewed} of ${changes.length} marked viewed (informational)`,
      ),
    );
    const controls = append(
      el("div", "actions"),
      disabled(
        button(
          "Discard",
          () => confirmRunAction(run, "discard"),
          "button danger",
        ),
        state.offline ? "Offline." : null,
      ),
      disabled(
        button(
          `Apply ${changes.length} file${changes.length === 1 ? "" : "s"}`,
          () => confirmRunAction(run, "apply"),
          "button primary",
        ),
        state.offline ? "Offline." : null,
      ),
    );
    append(ui.decision, copy, controls);
  }
  function renderEvidence(run, panel) {
    const reading = el("div", "reading"),
      risks = el("div", "evidence-risks"),
      notice = el("div"),
      heading = el("h2", "", "Obligations & evidence"),
      list = el("div", "evidence-list");
    append(reading, risks, notice, heading, list);
    panel.append(reading);
    run.evidenceUI = {
      risks,
      notice,
      list,
      obligations: new Map(),
      evidence: new Map(),
    };
    updateEvidence(run);
  }
  function evidenceRecord(record) {
    const block = el(
      "article",
      "evidence" +
        (record.grade === "deterministic" && record.status === "passed"
          ? " deterministic-passed"
          : ""),
    );
    const glyph =
      { passed: "✓", failed: "✕", inconclusive: "?", skipped: "–" }[
        record.status
      ] || "?";
    append(
      block,
      append(
        el("div", "row"),
        gradeMeter(record.grade),
        el(
          "span",
          "chip",
          record.kind?.replaceAll("_", " ") || "Kind not recorded",
        ),
        el("span", "evidence-status", glyph + " " + record.status),
      ),
      clampText(record.summary || "No summary recorded.", "evidence-summary"),
    );
    if (record.reproduction)
      append(
        block,
        el("h3", "", "Reproduction"),
        codeBlock(record.reproduction, "reproduction command"),
      );
    if (record.artifact_digest)
      append(
        block,
        el("p", "caption", "Artifact digest"),
        hashText(record.artifact_digest, "artifact digest"),
        el(
          "p",
          "caption",
          "Full output is stored by the engine; not viewable in the browser yet.",
        ),
      );
    return block;
  }
  function updateEvidence(run) {
    const ui = run.evidenceUI;
    if (!ui) return;
    const records = liveEvidence(run),
      contract = contractOf(run),
      risks = run.receipt?.unresolved_risks || [],
      riskKey = JSON.stringify(risks);
    if (ui.risks.dataset.key !== riskKey) {
      ui.risks.dataset.key = riskKey;
      ui.risks.replaceChildren(el("h2", "", "Unresolved risks"));
      if (risks.length)
        for (const risk of risks)
          ui.risks.append(banner("warn", String(risk), ""));
      else ui.risks.append(el("p", "caption", "No unresolved risks recorded."));
    }
    const notice = freshnessNotice(run);
    if (ui.notice.dataset.text !== notice) {
      ui.notice.dataset.text = notice;
      ui.notice.replaceChildren();
      if (notice) ui.notice.append(banner("warn", notice, ""));
      if (["stopped", "failed"].includes(runState(currentRecord(run))))
        ui.notice.append(
          banner("warn", "Partial — this run did not finish.", ""),
        );
    }
    if (!contract) {
      if (!ui.list.children.length)
        ui.list.append(
          el("p", "caption", "Waiting for the contract to be recorded."),
        );
      return;
    }
    ui.list.querySelector(":scope > p")?.remove();
    const rank = (o) => {
      const e = records.filter((e) => e.obligation_id === o.id);
      return e.some((e) => e.status === "failed")
        ? 0
        : e.some((e) => e.status === "inconclusive")
          ? 1
          : !e.length || e.some((e) => e.grade === "unverified")
            ? 2
            : 3;
    };
    const obligations = [...(contract.obligations || [])].sort(
      (a, b) => rank(a) - rank(b),
    );
    for (const o of obligations) {
      let item = ui.obligations.get(o.id);
      if (!item) {
        const details = el("details"),
          summary = el("summary"),
          status = el("span", "state-symbol"),
          copy = el("div", "obligation-copy"),
          labels = el("div", "chips");
        append(
          labels,
          el(
            "span",
            "chip",
            o.kind
              ? o.kind[0].toUpperCase() + o.kind.slice(1)
              : "Kind not recorded",
          ),
          o.required ? el("span", "chip", "required") : null,
        );
        append(copy, labels, el("p", "obligation-description", o.description));
        append(summary, status, copy);
        const body = el("div");
        append(details, summary, body);
        item = { details, status, body };
        ui.obligations.set(o.id, item);
        ui.list.append(details);
      }
      const evidence = records.filter((e) => e.obligation_id === o.id);
      item.status.textContent = evidence.some((e) => e.status === "failed")
        ? "✕"
        : evidence.some((e) => e.status === "inconclusive")
          ? "?"
          : evidence.length && evidence.every((e) => e.status === "passed")
            ? "✓"
            : "○";
      item.status.title = evidence.length
        ? evidence.map((e) => e.status).join(", ")
        : "Unverified";
      for (const record of evidence)
        if (!ui.evidence.has(record.id)) {
          const block = evidenceRecord(record);
          item.body.querySelector(".no-evidence")?.remove();
          item.body.append(block);
          ui.evidence.set(record.id, block);
        }
      if (!evidence.length && !item.body.children.length)
        item.body.append(
          el(
            "p",
            "caption no-evidence",
            "No evidence has been recorded for this obligation.",
          ),
        );
    }
    updateTabs(run);
  }
  function receiptSection(title, ...content) {
    return append(
      el("section", "receipt-section"),
      el("h2", "", title),
      ...content,
    );
  }
  function renderReceipt(run, panel) {
    const signature =
      (run.receipt?.integrity_hash || "none") +
      ":" +
      (run.diff?.receipt_integrity || "unchecked");
    if (panel.dataset.signature === signature) return;
    panel.dataset.signature = signature;
    const rawWasOpen = panel.querySelector("details")?.open;
    panel.replaceChildren();
    const r = run.receipt;
    if (!r) {
      panel.append(el("p", "reading", "Available when the run finishes."));
      return;
    }
    const reading = el("div", "reading");
    const integrity =
      run.diff?.receipt_integrity === "verified"
        ? "Verified by engine when this diff was loaded"
        : "Not checked in this view.";
    append(
      reading,
      receiptSection(
        "Outcome & integrity",
        stateChip(runState(currentRecord(run))),
        el("p", "caption", "Created " + timeText(r.created_at, true)),
        el("p", "", integrity),
        run.diff?.receipt_integrity === "verified"
          ? null
          : codeBlock("pactrail inspect " + run.id, "verification command"),
      ),
    );
    const c = r.contract;
    const budget = el("dl", "attributes");
    for (const [key, value] of Object.entries(c.budget || {}))
      append(
        budget,
        el("dt", "", key.replaceAll("_", " ")),
        el(
          "dd",
          "",
          value === 0
            ? "no limit"
            : formatUsage(value, {
                kind: key === "cost_microusd" ? "cost" : "number",
              }).text,
        ),
      );
    const paths = append(
      el("div", "chips"),
      ...(c.allowed_write_paths || []).map((p) =>
        copyText(p, "Copy allowed path"),
      ),
    );
    const out = append(
      el("div", "chips"),
      ...(c.out_of_scope || []).map((p) => el("span", "chip", p)),
    );
    append(
      reading,
      receiptSection(
        "Contract",
        clampText(c.goal),
        el("p", "caption", "Workspace"),
        copyText(c.workspace_root, "Copy contract workspace"),
        el("p", "caption", "Allowed write paths"),
        paths,
        el("p", "caption", "Out of scope"),
        out.children.length ? out : el("p", "caption", "None recorded."),
        budget,
        el("p", "caption", "Base commit: not recorded"),
      ),
    );
    const obligations = el("div", "stack");
    for (const o of c.obligations || [])
      obligations.append(
        button(
          o.description,
          () => {
            switchTab(run, "evidence");
            const row = run.evidenceUI?.obligations.get(o.id);
            if (row) {
              row.details.open = true;
              row.details.scrollIntoView({ block: "center" });
            }
          },
          "button link",
        ),
      );
    reading.append(
      receiptSection("Obligations", obligations),
      receiptSection("Changes", changesTable(r.changes || [])),
    );
    const policy = run.events.filter((e) =>
        ["policy_evaluated", "approval_decided"].includes(e.event?.type),
      ),
      approvals = el("div");
    if (!(r.approvals || []).length && !policy.length)
      approvals.append(
        el("p", "caption", "No approvals or policy decisions were recorded."),
      );
    else
      for (const item of [
        ...r.approvals,
        ...policy.map((e) => ({ sequence: e.sequence, ...e.event.data })),
      ])
        approvals.append(
          codeBlock(JSON.stringify(item, null, 2), "approval or policy record"),
        );
    reading.append(receiptSection("Approvals & policy", approvals));
    const chain = el("dl", "attributes");
    for (const key of ["baseline_digest", "final_event_hash", "integrity_hash"])
      append(
        chain,
        el("dt", "", key.replaceAll("_", " ")),
        append(el("dd"), hashText(r[key], key)),
      );
    reading.append(receiptSection("Chain", chain));
    reading.append(
      receiptSection(
        "Metadata",
        Object.keys(r.metadata || {}).length
          ? codeBlock(JSON.stringify(r.metadata, null, 2), "metadata")
          : el("p", "caption", "No metadata recorded."),
      ),
    );
    const raw = el("details"),
      rawText = JSON.stringify(r, null, 2);
    append(
      raw,
      el("summary", "", "Raw receipt.json"),
      codeBlock(rawText, "receipt JSON"),
      button("Download receipt.json", () =>
        download(rawText, "receipt-" + run.id + ".json"),
      ),
    );
    raw.open = Boolean(rawWasOpen);
    reading.append(receiptSection("Raw record", raw));
    panel.append(reading);
  }
  function download(text, name) {
    const url = URL.createObjectURL(
        new Blob([text], { type: "application/json" }),
      ),
      a = el("a", "", null, { href: url, download: name });
    document.body.append(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  function inlineMarkdown(text, parent) {
    const pattern = /(\*\*([^*]+)\*\*|`([^`]+)`)/g;
    let at = 0,
      match;
    while ((match = pattern.exec(text))) {
      parent.append(
        document.createTextNode(text.slice(at, match.index)),
        el(match[2] ? "strong" : "code", "", match[2] || match[3]),
      );
      at = pattern.lastIndex;
    }
    parent.append(document.createTextNode(text.slice(at)));
  }
  function markdown(text) {
    const root = el("div", "answer-prose"),
      lines = String(text).split("\n");
    let paragraph = [],
      list = null;
    const flush = () => {
      if (paragraph.length) {
        const p = el("p");
        inlineMarkdown(paragraph.join(" "), p);
        root.append(p);
        paragraph = [];
      }
    };
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      if (line.startsWith("```")) {
        flush();
        list = null;
        const code = [];
        while (++i < lines.length && !lines[i].startsWith("```"))
          code.push(lines[i]);
        root.append(codeBlock(code.join("\n"), "answer code"));
        continue;
      }
      const h = line.match(/^(#{1,3})\s+(.+)$/);
      if (h) {
        flush();
        list = null;
        const heading = el("h" + Math.min(3, h[1].length + 1));
        inlineMarkdown(h[2], heading);
        root.append(heading);
        continue;
      }
      const li = line.match(/^\s*(?:[-*]|\d+\.)\s+(.+)$/);
      if (li) {
        flush();
        if (!list) {
          list = el(/^\s*\d/.test(line) ? "ol" : "ul");
          root.append(list);
        }
        const item = el("li");
        inlineMarkdown(li[1], item);
        list.append(item);
        continue;
      }
      list = null;
      if (!line.trim()) flush();
      else paragraph.push(line);
    }
    flush();
    return root;
  }
  function renderAnswer(run, panel) {
    panel.replaceChildren();
    const reading = el("div", "reading"),
      summary = run.detail?.web_result?.summary,
      u = usageProjection(run.events),
      toolbar = el("div", "answer-toolbar");
    if (typeof summary === "string" && summary) {
      append(
        toolbar,
        button("Copy answer", (e) => copy(summary, e.currentTarget)),
        el(
          "span",
          "caption",
          `${timeText(run.events.at(-1)?.timestamp, true)} · ${u.turns} turns · ${formatUsage(u.total, { kind: "tokens", coverage: { reported: u.reported, total: u.turns } }).text} tokens`,
        ),
      );
      append(reading, toolbar, markdown(summary));
    } else {
      append(
        reading,
        banner(
          "neutral",
          "Answer text unavailable.",
          "The answer text wasn’t stored for runs started outside the browser. The trace shows what the agent did.",
        ),
      );
      for (const e of run.events.slice(-10))
        reading.append(
          el(
            "p",
            "caption",
            e.event.data?.summary ||
              e.event.data?.message ||
              e.event.type.replaceAll("_", " "),
          ),
        );
    }
    const consulted = new Set(
      actions(run.events)
        .flatMap((e) => e.event.data.observed_effects || [])
        .filter((x) => x.startsWith("fs.read:"))
        .map((x) => x.slice(8)),
    );
    if (consulted.size)
      append(
        reading,
        el("h2", "", "Files consulted"),
        append(
          el("div", "chips"),
          ...[...consulted].map((p) => copyText(p, "Copy consulted path")),
        ),
      );
    panel.append(reading);
  }
  function showDialog(title, body, footer, initial) {
    const d = $("dialog");
    state.dialogInvoker = document.activeElement;
    d.replaceChildren(
      el("h2", "", title, { id: "dialog-title" }),
      body,
      footer,
    );
    d.showModal();
    (initial || footer.querySelector("button"))?.focus();
    d.oncancel = (e) => {
      if (state.dialogBusy) e.preventDefault();
    };
    d.onclose = () => {
      state.dialogBusy = false;
      const target = state.dialogInvoker;
      if (target?.isConnected) target.focus();
      else state.current?.nodes?.title.focus();
    };
    d.onclick = (e) => {
      if (e.target === d && !state.dialogBusy) {
        const r = d.getBoundingClientRect();
        if (
          e.clientX < r.left ||
          e.clientX > r.right ||
          e.clientY < r.top ||
          e.clientY > r.bottom
        )
          d.close();
      }
    };
  }
  function confirmRunAction(run, action) {
    if (state.offline || runState(currentRecord(run)) !== "review") return;
    const changes = run.receipt.changes,
      count = changes.length,
      s = evidenceSummary(run.receipt.evidence),
      f = freshness(run.events),
      body = el("div", "dialog-body"),
      footer = el("div", "dialog-footer"),
      cancel = button("Cancel", () => $("dialog").close()),
      confirm = button(
        action === "apply"
          ? `Apply ${count} file${count === 1 ? "" : "s"}`
          : "Discard candidate",
        execute,
        "button " + (action === "apply" ? "primary" : "danger"),
      ),
      errors = el("div"),
      acks = [];
    let note;
    if (action === "apply") {
      append(body, el("p", "dialog-path", state.bootstrap?.workspace));
      const list = el("ul");
      for (const c of changes.slice(0, 6))
        list.append(
          el(
            "li",
            "mono-meta",
            `${changeStatus(c)[0]} ${c.path} · +${c.bytes_added} −${c.bytes_removed} bytes`,
          ),
        );
      if (count > 6) list.append(el("li", "caption", `+ ${count - 6} more`));
      append(
        body,
        list,
        el(
          "p",
          "",
          `Evidence: ${s.grade?.replaceAll("_", " ") || "not recorded"} · ${s.passed} passed · ${s.inconclusive} inconclusive`,
        ),
      );
      const cautions = [];
      if (!s.deterministicPass)
        cautions.push(
          "I understand this candidate is not backed by deterministic checks.",
        );
      if (s.failed)
        cautions.push(
          `I understand ${s.failed} checks failed and want to apply anyway.`,
        );
      if (f.stale)
        cautions.push(
          "I understand the evidence predates the last file change.",
        );
      for (const text of cautions) {
        const label = el("label", "ack"),
          input = el("input", "", null, { type: "checkbox" });
        acks.push(input);
        append(label, input, el("span", "", text));
        body.append(label);
        input.onchange = () =>
          disabled(
            confirm,
            state.offline
              ? "Offline."
              : acks.every((x) => x.checked)
                ? null
                : "Acknowledge each caution before applying.",
          );
      }
      if (cautions.length)
        disabled(confirm, "Acknowledge each caution before applying.");
      body.append(
        el(
          "p",
          "caption dialog-warning",
          "The web app has no undo for this action. If the workspace has changed since this run started, the engine may refuse to apply and the reason will be shown here.",
        ),
      );
    } else {
      append(
        body,
        el(
          "p",
          "",
          `The isolated copy is removed. Files in ${baseName(state.bootstrap?.workspace)} are not touched.`,
        ),
      );
      note = field(
        "Note (optional) — saved in this browser only; Pactrail doesn’t record discard reasons yet.",
        "",
        { tag: "textarea", attrs: { maxlength: 4000, rows: 4 } },
      );
      body.append(note.wrap);
    }
    body.append(errors);
    append(footer, cancel, confirm);
    showDialog(
      action === "apply"
        ? `Apply ${count} file${count === 1 ? "" : "s"} to ${baseName(state.bootstrap?.workspace)}?`
        : "Discard this candidate?",
      body,
      footer,
      action === "apply" ? cancel : note.input,
    );
    async function execute() {
      if (state.offline || state.dialogBusy || confirm.disabled) return;
      state.dialogBusy = true;
      errors.replaceChildren();
      for (const control of $("dialog").querySelectorAll(
        "button,input,textarea",
      ))
        disabled(control, "Applying changes; wait for the engine.");
      confirm.textContent = action === "apply" ? "Applying…" : "Discarding…";
      $("dialog").setAttribute("aria-busy", "true");
      try {
        const result = await api(`/api/runs/${run.id}/${action}`, {
          method: "POST",
        });
        if (note) {
          state.notes[run.id] = note.input.value;
          save("discardNotes", state.notes);
        }
        run.receipt = receiptOf(result) || result;
        run.detail = { ...run.receipt, web_result: run.detail?.web_result };
        await refreshRunDetail(run);
        state.dialogBusy = false;
        $("dialog").removeAttribute("aria-busy");
        $("dialog").close();
        updateDecision(run);
        const heading = run.nodes.banners.querySelector("h2");
        heading?.focus();
        toast(
          action === "apply"
            ? `Applied ${count} file${count === 1 ? "" : "s"}.`
            : "Candidate discarded.",
        );
      } catch (error) {
        state.dialogBusy = false;
        $("dialog").removeAttribute("aria-busy");
        for (const control of $("dialog").querySelectorAll(
          "button,input,textarea",
        ))
          disabled(control, null);
        confirm.textContent = "Try again";
        cancel.textContent = "Close";
        errors.append(
          errorBlock(
            action === "apply" ? "Apply failed." : "Discard failed.",
            error,
          ),
        );
        errors.querySelector("h2").focus();
        announce(action + " failed", true);
        if (error.network) connectionLost();
      }
    }
  }
  const SHORTCUTS = [
    ["N", "New run"],
    ["Ctrl / ⌘ K", "Search runs (also inside fields)"],
    ["Ctrl / ⌘ Enter", "Dispatch task"],
    ["\\", "Toggle run history"],
    ["/", "Focus current filter"],
    [
      "g then t / c / e / r / a",
      "Trace / Changes / Evidence / Receipt / Answer",
    ],
    ["j / k", "Next / previous turn or file"],
    ["] / [", "Next / previous hunk"],
    ["v", "Toggle Viewed"],
    ["s", "Switch diff layout"],
    ["w", "Toggle wrapping"],
    ["F / End", "Resume trace follow"],
    ["Shift A / Shift D", "Open Apply / Discard dialog"],
    ["Esc", "Close dialog, popover, drawer, or search"],
    ["?", "Show shortcuts"],
  ];
  function shortcutsTable() {
    const table = el("table", "data-table");
    for (const [key, description] of SHORTCUTS)
      append(
        table,
        append(
          el("tr"),
          append(el("td"), el("kbd", "", key)),
          el("td", "", description),
        ),
      );
    return table;
  }
  function confirmLocal(anchor, title, action) {
    showPopover(
      anchor,
      title,
      [el("p", "", "This affects only data stored in this browser.")],
      [
        button("Cancel", closePopover),
        button("Confirm", () => {
          action();
          closePopover();
          toast("Browser data updated.");
        }),
      ],
    );
  }
  function renderSettings() {
    state.current = null;
    const page = el("section", "settings-page"),
      nav = el("nav", "settings-nav", null, {
        "aria-label": "Settings sections",
      }),
      content = el("div", "settings-content");
    for (const [id, label] of [
      ["appearance", "Appearance"],
      ["pricing", "Model pricing"],
      ["engine", "Engine"],
      ["keyboard", "Keyboard shortcuts"],
      ["browser-data", "Data in this browser"],
    ])
      nav.append(el("a", "", label, { href: "#" + id }));
    content.append(el("h1", "", "Settings"));
    const appearance = el("section", "settings-section", null, {
      id: "appearance",
    });
    append(
      appearance,
      el("h2", "", "Appearance"),
      el("p", "caption", "Stored in this browser. Changes apply immediately."),
    );
    for (const [key, label, choices] of [
      [
        "theme",
        "Theme",
        [
          ["system", "System"],
          ["light", "Light"],
          ["dark", "Dark"],
        ],
      ],
      [
        "density",
        "Density",
        [
          ["comfortable", "Comfortable"],
          ["compact", "Compact"],
        ],
      ],
      [
        "motion",
        "Motion",
        [
          ["system", "System"],
          ["reduce", "Reduce"],
        ],
      ],
      [
        "timestamps",
        "Timestamps",
        [
          ["relative", "Relative"],
          ["absolute", "Absolute"],
        ],
      ],
      [
        "clock",
        "Clock",
        [
          ["24h", "24 hour"],
          ["12h", "12 hour"],
        ],
      ],
      [
        "diffMode",
        "Diff layout",
        [
          ["auto", "Auto"],
          ["split", "Split"],
          ["unified", "Unified"],
        ],
      ],
    ]) {
      const row = el("div", "settings-row"),
        select = el("select", "", null, { id: "pref-" + key });
      for (const [value, text] of choices)
        select.append(el("option", "", text, { value }));
      select.value = prefs[key];
      select.onchange = () => setPref(key, select.value);
      append(row, el("label", "", label, { for: "pref-" + key }), select);
      appearance.append(row);
    }
    for (const [key, label] of [
      ["wrapTrace", "Wrap code in trace"],
      ["wrapDiff", "Wrap diff lines"],
    ]) {
      const row = el("label", "settings-row"),
        check = el("input", "", null, { type: "checkbox" });
      check.checked = prefs[key];
      check.onchange = () => setPref(key, check.checked);
      append(row, el("span", "", label), check);
      appearance.append(row);
    }
    content.append(appearance);
    const pricing = el("section", "settings-section", null, { id: "pricing" }),
      cards = el("div", "pricing-grid");
    append(
      pricing,
      el("h2", "", "Model pricing"),
      el(
        "p",
        "caption",
        "Pactrail doesn’t fetch prices. Copy them from your provider’s pricing page. Cards are stored in this browser.",
      ),
      cards,
      button("＋ Add model pricing", (e) => addPricing(e.currentTarget)),
    );
    content.append(pricing);
    state.pricingCards = cards;
    const engine = el("section", "settings-section", null, { id: "engine" });
    append(
      engine,
      el("h2", "", "Engine"),
      el("p", "caption", "Change defaults by restarting Pactrail."),
    );
    for (const [label, value] of [
      ["Version", state.bootstrap?.version],
      ["Workspace", state.bootstrap?.workspace],
      [
        "Connection",
        state.offline ? "Offline" : "Connected to " + location.host,
      ],
      ...Object.entries(state.bootstrap?.defaults || {}).map(([k, v]) => [
        k.replaceAll("_", " "),
        v || "Not configured",
      ]),
    ])
      engine.append(
        append(
          el("div", "settings-row"),
          el("span", "", label),
          value
            ? copyText(value, "Copy " + label, "mono-meta")
            : usageNode(null),
        ),
      );
    content.append(engine);
    append(
      content,
      append(
        el("section", "settings-section", null, { id: "keyboard" }),
        el("h2", "", "Keyboard shortcuts"),
        shortcutsTable(),
      ),
    );
    const data = el("section", "settings-section", null, {
      id: "browser-data",
    });
    append(
      data,
      el("h2", "", "Data in this browser"),
      el(
        "p",
        "caption",
        `${Object.keys(state.titles).length} run titles · ${Object.keys(state.pricing).length} pricing cards · ${Object.keys(state.viewed).length} viewed marks · ${Object.keys(state.notes).length} discard notes`,
      ),
    );
    if (storageFailed)
      data.append(
        banner(
          "warn",
          "Browser storage is unavailable.",
          "Changes are kept in memory for this session. They may be lost when you reload.",
        ),
      );
    append(
      data,
      append(
        el("div", "actions"),
        button("Clear run titles", (e) =>
          confirmLocal(e.currentTarget, "Clear run titles?", () => {
            state.titles = {};
            save("titles", {});
            updateRail();
          }),
        ),
        button("Clear draft", (e) =>
          confirmLocal(e.currentTarget, "Clear draft?", () => {
            state.draft = { goal: "", title: "" };
            save("draft", state.draft);
          }),
        ),
        button("Reset preferences", (e) =>
          confirmLocal(e.currentTarget, "Reset preferences?", () => {
            for (const [k, v] of Object.entries(preferenceDefaults))
              setPref(k, v);
            renderSettings();
          }),
        ),
      ),
    );
    content.append(data);
    append(page, nav, content);
    $("main").replaceChildren(page);
    pageBreadcrumb("Settings");
    renderPricingCards();
    const params = new URLSearchParams(location.search);
    if (params.has("model") && params.get("model")) {
      const p = params.get("provider") || "ollama",
        m = params.get("model"),
        key = p + "::" + m;
      if (!state.pricing[key]) {
        state.pricing[key] = {
          provider: p,
          model: m,
          input: "",
          cached_input: "",
          cache_creation: "",
          output: "",
        };
        save("pricing", state.pricing);
        renderPricingCards();
      }
    }
    if (location.hash)
      requestAnimationFrame(() => $(location.hash.slice(1))?.scrollIntoView());
  }
  function renderPricingCards() {
    const grid = state.pricingCards;
    if (!grid?.isConnected) return;
    grid.replaceChildren();
    for (const [key, card] of Object.entries(state.pricing)) {
      if (!object(card)) continue;
      const split = key.indexOf("::"),
        provider = card.provider || key.slice(0, split),
        model = card.model || key.slice(split + 2),
        article = el("article", "pricing-card", null, {
          id:
            "price-" +
            btoa(unescape(encodeURIComponent(key))).replace(
              /[^a-zA-Z0-9]/g,
              "",
            ),
        }),
        status = el("p", "pricing-status"),
        fields = {};
      append(
        article,
        el("h3", "mono", model),
        el("p", "caption", providers[provider] || provider),
        el("p", "caption", "Prices in USD per million tokens"),
      );
      const pairs = [el("div", "field-pair"), el("div", "field-pair")];
      PRICE_FIELDS.forEach((name, i) => {
        const fieldNode = field(
          {
            input: "Input",
            cached_input: "Cached input",
            cache_creation: "Cache creation",
            output: "Output",
          }[name],
          card[name] ?? "",
          { type: "number", attrs: { min: 0, step: "0.000001" } },
        );
        fields[name] = fieldNode.input;
        pairs[Math.floor(i / 2)].append(fieldNode.wrap);
        fieldNode.input.oninput = () => {
          const v = fieldNode.input.value;
          if (v && !/^\d+(?:\.\d{0,6})?$/.test(v)) {
            fieldNode.input.classList.add("invalid");
            fieldNode.input.setCustomValidity(
              "Use a nonnegative price with at most six decimal places.",
            );
            status.textContent =
              "Use a nonnegative price with at most six decimal places.";
            return;
          }
          fieldNode.input.setCustomValidity("");
          fieldNode.input.classList.remove("invalid");
          card[name] = v === "" ? "" : Number(v);
          save("pricing", state.pricing);
          updateStatus();
        };
      });
      append(article, ...pairs, status);
      function updateStatus() {
        const n = PRICE_FIELDS.filter((k) => numeric(card[k]) !== null).length;
        status.className =
          "pricing-status" + (n > 0 && n < 4 ? " incomplete" : "");
        status.textContent =
          n === 4
            ? "✓ Complete — cost caps enabled for this model"
            : n
              ? "Incomplete — cost caps are unavailable until all four are set"
              : "No prices — cost is not reported.";
      }
      updateStatus();
      append(
        article,
        append(
          el("div", "actions"),
          button("Duplicate", (e) =>
            addPricing(e.currentTarget, {
              ...card,
              provider,
              model: model + "-copy",
            }),
          ),
          button("Use in new run", () => {
            state.config = { ...state.config, provider, model };
            save("config", state.config);
            navigate("/");
          }),
          button(
            "Delete",
            (e) =>
              showPopover(
                e.currentTarget,
                "Delete pricing for " + model + "?",
                [el("p", "", "Runs already started are unaffected.")],
                [
                  button("Cancel", closePopover),
                  button(
                    "Delete",
                    () => {
                      delete state.pricing[key];
                      save("pricing", state.pricing);
                      closePopover();
                      renderPricingCards();
                    },
                    "button danger",
                  ),
                ],
              ),
            "button danger",
          ),
        ),
      );
      grid.append(article);
    }
    if (!grid.children.length)
      grid.append(el("p", "caption", "No price cards in this browser yet."));
  }
  function addPricing(anchor, initial = {}) {
    let provider =
      initial.provider ||
      state.config.provider ||
      state.bootstrap?.defaults.provider ||
      "ollama";
    const select = providerListbox(provider, (v) => (provider = v)),
      model = field("Model ID", initial.model || "", { mono: true }),
      error = el("div");
    showPopover(
      anchor,
      "Add model pricing",
      [select.wrap, model.wrap, error],
      [
        button("Cancel", closePopover),
        button("Add card", () => {
          const id = model.input.value.trim(),
            key = provider + "::" + id;
          if (!id) {
            error.replaceChildren(el("p", "field-error", "Enter a model ID."));
            return;
          }
          if (state.pricing[key]) {
            error.replaceChildren(
              el("p", "field-error", "A card for this model already exists."),
              button("Open it", () => {
                closePopover();
                renderPricingCards();
              }),
            );
            return;
          }
          state.pricing[key] = {
            provider,
            model: id,
            ...Object.fromEntries(
              PRICE_FIELDS.map((k) => [k, initial[k] ?? ""]),
            ),
          };
          save("pricing", state.pricing);
          closePopover();
          renderPricingCards();
        }),
      ],
    );
    model.input.focus();
  }
  function syncNetworkControls() {
    for (const control of document.querySelectorAll("[data-network-action]")) {
      if (state.offline) {
        if (!control.dataset.offlineDisabled) {
          control.dataset.offlineDisabled = "true";
          control.dataset.priorDisabled = String(control.disabled);
          control.dataset.priorTitle = control.title;
        }
        disabled(control, "Offline.");
      } else if (control.dataset.offlineDisabled) {
        control.disabled = control.dataset.priorDisabled === "true";
        control.title = control.dataset.priorTitle || "";
        delete control.dataset.offlineDisabled;
      }
    }
  }
  function connectionLost() {
    if (!state.offline) {
      state.offline = true;
      state.connected = false;
      state.backoff = 0;
      closeStream();
      announce("Lost connection to the Pactrail engine", true);
    }
    const wait = [1, 2, 4, 8, 15][Math.min(state.backoff, 4)];
    $("connection-banner").replaceChildren(
      banner(
        "fail",
        `Lost connection to the Pactrail engine at ${location.host}.`,
        `A run already started keeps going only if the engine process is alive. Retrying in ${wait} s… Last updated ${state.lastUpdated ? clock(state.lastUpdated) : "—"}.`,
        [button("Retry now", () => refresh(true))],
      ),
    );
    $("connection-banner").hidden = false;
    $("connection-dot").classList.add("offline");
    $("connection-dot").title = "Offline";
    $("connection-dot").setAttribute("aria-label", "Offline");
    $("engine-indicator").textContent = "Engine disconnected";
    state.composerSync?.();
    syncNetworkControls();
    if (state.current) {
      updateRunHeader(state.current);
      updateDecision(state.current);
    }
  }
  async function refresh(force = false) {
    if (state.refreshing && !force) return;
    clearTimeout(state.refreshTimer);
    state.refreshing = true;
    let delay = state.pending
      ? 800
      : state.jobs.some((j) => ["running", "cancelling"].includes(j.state))
        ? 5000
        : 30000;
    try {
      if (!state.bootstrap) state.bootstrap = await api("/api/bootstrap");
      const [runs, jobs] = await Promise.all([
        api("/api/runs"),
        api("/api/jobs"),
      ]);
      const reconnect = state.offline,
        first = !state.connected && !state.lastUpdated;
      state.runs = Array.isArray(runs) ? runs : [];
      state.jobs = Array.isArray(jobs) ? jobs : [];
      state.offline = false;
      state.connected = true;
      state.backoff = 0;
      state.lastUpdated = Date.now();
      $("connection-dot").classList.remove("offline");
      $("connection-dot").title = "Connected to localhost";
      $("connection-dot").setAttribute("aria-label", "Connected to localhost");
      $("engine-indicator").textContent =
        `● Engine v${state.bootstrap.version} · connected`;
      $("workspace-copy").textContent = baseName(state.bootstrap.workspace);
      $("workspace-copy").title =
        state.bootstrap.workspace + " · click to copy";
      if (reconnect) {
        $("connection-banner").replaceChildren(
          banner("info", "Reconnected.", ""),
        );
        setTimeout(() => {
          if (!state.offline) $("connection-banner").hidden = true;
        }, 3000);
        announce("Reconnected");
      } else $("connection-banner").hidden = true;
      updateRail();
      if (
        first &&
        (location.pathname === "/" || location.pathname === "/settings")
      )
        route();
      else state.composerSync?.();
      state.startingTick?.();
      if (state.current) {
        const run = state.current;
        run.record = state.runs.find((r) => r.run_id === run.id) || run.record;
        updateRunHeader(run);
        updateRunState(run);
        if (
          reconnect ||
          (!run.receipt &&
            ["review", "answered", "failed", "stopped"].includes(
              runState(currentRecord(run)),
            ))
        )
          await refreshRunDetail(run);
        if (reconnect && !TERMINAL.has(currentRecord(run).state))
          openStream(run);
      }
      delay = state.pending
        ? 800
        : state.jobs.some((j) => ["running", "cancelling"].includes(j.state))
          ? 5000
          : 30000;
    } catch (error) {
      if (error.network || error.status >= 500) {
        connectionLost();
        delay = [1000, 2000, 4000, 8000, 15000][Math.min(state.backoff++, 4)];
      } else {
        let block = $("refresh-error");
        if (!block) {
          block = errorBlock("Unable to refresh engine data.", error, () =>
            refresh(true),
          );
          block.id = "refresh-error";
          $("connection-banner").replaceChildren(block);
          $("connection-banner").hidden = false;
        }
        delay = 5000;
      }
    } finally {
      state.refreshing = false;
      state.refreshTimer = setTimeout(refresh, delay);
    }
  }
  async function refreshRunDetail(run) {
    if (run.detailRefreshing) return;
    run.detailRefreshing = true;
    try {
      const [detail, trace] = await Promise.all([
        api(`/api/runs/${run.id}/inspect`),
        api(`/api/runs/${run.id}/trace`),
      ]);
      if (state.current !== run) return;
      const had = !!run.receipt;
      run.detail = detail;
      run.receipt = receiptOf(detail);
      mergeEvents(run, Array.isArray(trace) ? trace : []);
      updateRunHeader(run);
      run.bannerKey = null;
      updateRunState(run);
      if (run.receipt && !had) {
        loadDiff(run);
        if (run.panels.has("changes")) populateChanges(run);
      }
      if (run.evidenceUI) updateEvidence(run);
      if (run.panels.has("receipt"))
        renderReceipt(run, run.panels.get("receipt"));
      if (run.panels.has("answer") && run.receipt)
        renderAnswer(run, run.panels.get("answer"));
      updateDecision(run);
    } catch (error) {
      if (error.network) connectionLost();
      else {
        const block = errorBlock("Unable to refresh this run.", error, () =>
          refreshRunDetail(run),
        );
        run.nodes.banners.append(block);
      }
    } finally {
      run.detailRefreshing = false;
    }
  }
  function route() {
    const token = ++state.routeToken;
    closeRail();
    closeStream();
    state.startingTick = null;
    state.composerSync = null;
    state.composerRef = null;
    if (state.current?.frame) cancelAnimationFrame(state.current.frame);
    state.current?.changesUI?.observer?.disconnect();
    state.current?.changesUI?.resizeObserver?.disconnect();
    if (state.current?.trace?.selectionListener)
      document.removeEventListener(
        "selectionchange",
        state.current.trace.selectionListener,
      );
    state.current = null;
    const path = location.pathname;
    if (path === "/") renderComposer();
    else if (path === "/settings") renderSettings();
    else if (path === "/runs/starting") renderStarting();
    else if (/^\/runs\/[0-9a-f-]{36}$/i.test(path))
      loadRun(path.split("/")[2], token);
    else {
      history.replaceState({}, "", "/");
      renderComposer();
    }
    updateRail();
  }
  let gKey = false;
  function onKey(e) {
    const typing = e.target.closest(
      "input,textarea,select,[contenteditable=true]",
    );
    if (
      (e.ctrlKey || e.metaKey) &&
      e.key.toLowerCase() === "f" &&
      state.current?.view === "trace"
    ) {
      for (const phase of state.current.trace.phases) {
        phase.body.hidden = false;
        phase.head.setAttribute("aria-expanded", "true");
      }
      state.current.trace.following = false;
      updateFollowPill(state.current);
      return;
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
      e.preventDefault();
      openRail(true);
      return;
    }
    if ($("dialog").open) {
      if (e.key === "Tab") {
        const controls = [
          ...$("dialog").querySelectorAll(
            'button:not(:disabled),input:not(:disabled),textarea:not(:disabled),select:not(:disabled),a[href],[tabindex="0"]',
          ),
        ].filter((n) => n.getClientRects().length);
        if (controls.length) {
          if (e.shiftKey && document.activeElement === controls[0]) {
            e.preventDefault();
            controls.at(-1).focus();
          } else if (
            !e.shiftKey &&
            document.activeElement === controls.at(-1)
          ) {
            e.preventDefault();
            controls[0].focus();
          }
        }
      }
      return;
    }
    if (e.key === "Escape") {
      closePopover();
      closeRail();
      if (typing && e.target.type === "search") {
        e.target.value = "";
        e.target.dispatchEvent(new Event("input"));
      }
      return;
    }
    if (
      (e.ctrlKey || e.metaKey) &&
      e.key === "Enter" &&
      location.pathname === "/"
    ) {
      e.preventDefault();
      $("composer-form")?.requestSubmit();
      return;
    }
    if (typing || e.ctrlKey || e.metaKey || e.altKey) return;
    const run = state.current;
    if (gKey) {
      gKey = false;
      const view = {
        t: "trace",
        c: "changes",
        e: "evidence",
        r: "receipt",
        a: "answer",
      }[e.key];
      if (
        view &&
        run &&
        (!["changes", "receipt"].includes(view) || run.receipt)
      ) {
        e.preventDefault();
        switchTab(run, view);
      }
      return;
    }
    if (e.key === "g") {
      gKey = true;
      setTimeout(() => (gKey = false), 1200);
      return;
    }
    if (e.key === "n" || e.key === "N") {
      e.preventDefault();
      navigate("/");
      $("task-goal")?.focus();
    } else if (e.key === "\\") {
      e.preventDefault();
      if (innerWidth < 1280)
        $("rail").classList.contains("open") ? closeRail() : openRail();
      else setPref("railCollapsed", !prefs.railCollapsed);
    } else if (e.key === "/") {
      e.preventDefault();
      if (run?.view === "trace") run.trace.search.focus();
      else if (
        run?.view === "changes" &&
        run.changesUI.files.querySelector("input")
      )
        run.changesUI.files.querySelector("input").focus();
      else openRail(true);
    } else if (e.key === "?") {
      showShortcuts();
    } else if (run) {
      if (
        e.shiftKey &&
        e.key === "A" &&
        runState(currentRecord(run)) === "review"
      ) {
        e.preventDefault();
        confirmRunAction(run, "apply");
      } else if (
        e.shiftKey &&
        e.key === "D" &&
        runState(currentRecord(run)) === "review"
      ) {
        e.preventDefault();
        confirmRunAction(run, "discard");
      } else if (["F", "End"].includes(e.key) && run.view === "trace") {
        e.preventDefault();
        followTrace(run, true);
      } else if (e.key === "w") {
        e.preventDefault();
        setPref(
          run.view === "changes" ? "wrapDiff" : "wrapTrace",
          !(run.view === "changes" ? prefs.wrapDiff : prefs.wrapTrace),
        );
      } else if (e.key === "s" && run.view === "changes") {
        e.preventDefault();
        if (run.changesUI.diffPane.clientWidth >= 720)
          setPref(
            "diffMode",
            effectiveDiffMode(run) === "split" ? "unified" : "split",
          );
      } else if (e.key === "v" && run.view === "changes") {
        const f = run.files?.find((f) => f.path === run.selectedFile);
        if (f) {
          const val = !state.viewed[viewedKey(run, f)];
          markViewed(run, f, val);
          run.changesUI.fileNodes.get(f.path).checkbox.checked = val;
        }
      } else if (["j", "k"].includes(e.key)) {
        e.preventDefault();
        if (run.view === "trace") {
          const keys = [...run.trace.turns.keys()],
            index = Math.max(
              0,
              keys.indexOf(run.selectedTurn) + (e.key === "j" ? 1 : -1),
            );
          run.selectedTurn = keys[Math.min(index, keys.length - 1)];
          jumpToEvent(run, run.selectedTurn);
        } else if (run.view === "changes") {
          const i = run.files.findIndex((f) => f.path === run.selectedFile),
            f =
              run.files[
                Math.max(
                  0,
                  Math.min(run.files.length - 1, i + (e.key === "j" ? 1 : -1)),
                )
              ];
          if (f) selectFile(run, f.path);
        }
      } else if (["[", "]"].includes(e.key) && run.view === "changes") {
        const hunks = [...run.changesUI.body.querySelectorAll(".hunk-heading")],
          i = (run.hunkIndex ?? -1) + (e.key === "]" ? 1 : -1);
        run.hunkIndex = Math.max(0, Math.min(hunks.length - 1, i));
        hunks[run.hunkIndex]?.scrollIntoView({ block: "center" });
      }
    }
  }
  function showShortcuts() {
    const body = append(el("div", "dialog-body"), shortcutsTable()),
      close = button("Close", () => $("dialog").close());
    showDialog(
      "Keyboard shortcuts",
      body,
      append(el("div", "dialog-footer"), close),
      close,
    );
  }
  document.addEventListener("click", (e) => {
    const a = e.target.closest("a[data-link]");
    if (
      a &&
      a.origin === location.origin &&
      !e.ctrlKey &&
      !e.metaKey &&
      !e.shiftKey &&
      e.button === 0
    ) {
      e.preventDefault();
      navigate(a.pathname + a.search + a.hash);
    }
    if (
      !$("popover").hidden &&
      !$("popover").contains(e.target) &&
      !state.popoverAnchor?.contains(e.target)
    )
      closePopover(false);
  });
  document.addEventListener("keydown", onKey);
  window.addEventListener("pagehide", () => save("draft", state.draft));
  window.addEventListener("popstate", route);
  window.addEventListener("focus", () => refresh());
  $("shortcuts").onclick = showShortcuts;
  window.addEventListener("error", (e) => {
    if (e.error)
      $("main").append(
        errorBlock("An interface error occurred.", e.error, () =>
          location.reload(),
        ),
      );
  });
  window.addEventListener("unhandledrejection", (e) =>
    $("main").append(
      errorBlock("An interface operation failed.", e.reason, () =>
        location.reload(),
      ),
    ),
  );
  setInterval(() => {
    if (state.startingTick) state.startingTick();
    const run = state.current;
    if (run) {
      updateRunHeader(run);
      const job = jobFor(run),
        time = state.stopTimes.get(job?.id);
      if (
        job?.state === "cancelling" &&
        time &&
        Date.now() - time > 30000 &&
        !run.nodes.banners.querySelector(".stop-delay")
      ) {
        const p = el(
          "p",
          "caption stop-delay",
          "The engine is taking longer than expected. It may be inside a long-running command.",
        );
        run.nodes.banners.append(p);
      }
      if (run.trace)
        for (const row of run.trace.pending.values())
          row.querySelector("[aria-hidden=true]:last-child").textContent =
            formatUsage((Date.now() - Number(row.dataset.started)) / 1000, {
              kind: "duration",
            }).text;
    }
  }, 1000);
  applyPreferences();
  initRail();
  route();
  refresh();
}
