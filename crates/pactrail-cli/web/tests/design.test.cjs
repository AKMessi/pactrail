const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const root = path.join(__dirname, "..");
const css = fs.readFileSync(path.join(root, "app.css"), "utf8");
function tokens(selector) {
  const start = css.indexOf(selector),
    end = css.indexOf("}", start);
  return Object.fromEntries(
    [...css.slice(start, end).matchAll(/--([\w-]+):\s*(#[\da-f]{6})/gi)].map(
      (m) => [m[1], m[2]],
    ),
  );
}
function luminance(hex) {
  const c = hex
    .slice(1)
    .match(/../g)
    .map((v) => parseInt(v, 16) / 255)
    .map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
  return c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722;
}
function contrast(a, b) {
  const values = [luminance(a), luminance(b)].sort((a, b) => b - a);
  return (values[0] + 0.05) / (values[1] + 0.05);
}
test("all designed text surfaces and focus/control borders meet contrast", () => {
  const light = tokens(":root {"),
    dark = { ...light, ...tokens(':root[data-theme="dark"]') };
  assert.ok(Object.keys(light).length > 20);
  assert.ok(Object.keys(dark).length > 20);
  for (const [theme, t] of [
    ["light", light],
    ["dark", dark],
  ]) {
    const surfaces = [
      "bg",
      "surface",
      "surface-2",
      "code-bg",
      "accent-soft",
      "warn-soft",
      "fail-soft",
      "add-bg",
      "del-bg",
      "add-gutter",
      "del-gutter",
    ];
    for (const ink of ["ink", "ink-2", "ink-3"])
      for (const surface of surfaces)
        assert.ok(
          contrast(t[ink], t[surface]) >= 4.5,
          `${theme} ${ink}/${surface}: ${contrast(t[ink], t[surface]).toFixed(2)}`,
        );
    for (const surface of ["add-word", "del-word"])
      assert.ok(contrast(t.ink, t[surface]) >= 4.5, `${theme} ink/${surface}`);
    for (const ink of ["accent", "fail", "warn"])
      for (const surface of ["bg", "surface", "surface-2"])
        assert.ok(
          contrast(t[ink], t[surface]) >= 4.5,
          `${theme} ${ink}/${surface}`,
        );
    assert.ok(
      contrast(t.pass, t["pass-soft"]) >= 4.5,
      `${theme} evidence pass`,
    );
    assert.ok(
      contrast(t["on-accent"], t.accent) >= 4.5,
      `${theme} accent text`,
    );
    for (const border of ["control", "accent"])
      for (const surface of ["bg", "surface", "surface-2"])
        assert.ok(
          contrast(t[border], t[surface]) >= 3,
          `${theme} ${border}/${surface} border`,
        );
  }
});
test("markup and rendering respect CSP and text-only engine data", () => {
  const html = fs.readFileSync(path.join(root, "index.html"), "utf8"),
    js = fs.readFileSync(path.join(root, "app.js"), "utf8");
  assert.doesNotMatch(html, /\sstyle\s*=/i);
  assert.doesNotMatch(html, /<script(?![^>]*\bsrc=)[^>]*>/i);
  assert.doesNotMatch(js, /innerHTML|outerHTML|insertAdjacentHTML|\beval\s*\(/);
  assert.doesNotMatch(css, /text-transform:\s*uppercase/);
  assert.doesNotMatch(css, /gradient\(/);
  const afterTokens = css.slice(css.indexOf("\n* {"));
  assert.doesNotMatch(afterTokens, /#(?:[\da-f]{3,8})\b|rgba?\(/i);
  const passRules = [
    ...css.matchAll(/([^{}]+)\{([^{}]*var\(--pass(?:-soft)?\)[^{}]*)\}/g),
  ];
  assert.ok(passRules.length > 0);
  for (const match of passRules)
    assert.ok(match[1].includes(".deterministic-passed"), match[1]);
});
