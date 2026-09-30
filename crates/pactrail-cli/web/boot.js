"use strict";
(() => {
  try {
    const get = (key) => JSON.parse(localStorage.getItem("pactrail." + key));
    const theme = get("theme");
    if (["light", "dark"].includes(theme))
      document.documentElement.dataset.theme = theme;
    if (get("density") === "compact")
      document.documentElement.dataset.density = "compact";
    if (get("motion") === "reduce")
      document.documentElement.dataset.motion = "reduce";
  } catch {
    /* Storage is optional. The OS theme remains the default. */
  }
})();
