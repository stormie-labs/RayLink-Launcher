// RayLink prototypes: inline SVG sprite (24-unit grid, 1.75 stroke, square caps) + the screenshot state switch.
// Usage: <svg class="i"><use href="#i-play"/></svg>. ?state=update -> <html class="is-update">.
(function () {
  const params = new URLSearchParams(location.search);
  if (params.get("state") === "update") document.documentElement.classList.add("is-update");

  const sym = (id, body, vb = "0 0 24 24") => `<symbol id="${id}" viewBox="${vb}">${body}</symbol>`;
  const sprite =
    '<svg xmlns="http://www.w3.org/2000/svg" style="position:absolute;width:0;height:0" aria-hidden="true">' +
    // logo: three links bound into one network, laid out like the trefoil (one node down, two up)
    sym("qn-mark",
      '<circle cx="16" cy="16" r="10" fill="none" stroke="currentColor" stroke-width="1.6" opacity=".55"/>' +
      '<path d="M16 16V26M16 16L7.34 11M16 16L24.66 11" stroke="currentColor" stroke-width="2.2"/>' +
      '<circle cx="16" cy="16" r="2.9" fill="currentColor"/>' +
      '<circle cx="16" cy="26" r="3.6" fill="currentColor"/>' +
      '<circle cx="7.34" cy="11" r="3.6" fill="currentColor"/>' +
      '<circle cx="24.66" cy="11" r="3.6" fill="currentColor"/>', "0 0 32 32") +
    sym("i-play", '<path d="M7 4.5v15l12.5-7.5z" fill="currentColor" stroke="none"/>') +
    sym("i-host", '<circle cx="12" cy="12" r="2"/><path d="M7.8 7.8a6 6 0 0 0 0 8.4M16.2 7.8a6 6 0 0 1 0 8.4M4.9 4.9a10 10 0 0 0 0 14.2M19.1 4.9a10 10 0 0 1 0 14.2"/>') +
    sym("i-join", '<path d="M14 4h6v16h-6M3 12h12M11 8l4 4-4 4"/>') +
    sym("i-check", '<path d="M4.5 12.5l5 5 10-11"/>') +
    sym("i-download", '<path d="M12 3v12M7 10l5 5 5-5M4 20h16"/>') +
    sym("i-verify", '<path d="M4 8V4h4M16 4h4v4M20 16v4h-4M8 20H4v-4M8 12l3 3 5-6"/>') +
    sym("i-install", '<path d="M12 3l9 5-9 5-9-5zM3 13l9 5 9-5"/>') +
    sym("i-rollback", '<path d="M9 5L4 10l5 5M4 10h10a5 5 0 0 1 0 10h-3"/>') +
    sym("i-logs", '<path d="M5 3h10l4 4v14H5zM15 3v4h4M12 18v-7M9 14l3-3 3 3"/>') +
    sym("i-sliders", '<path d="M4 7h9M17 7h3M15 5v4M4 17h3M11 17h9M9 15v4"/>') +
    sym("i-folder", '<path d="M3 6h6l2 2h10v11H3z"/>') +
    sym("i-user", '<circle cx="12" cy="8" r="4"/><path d="M4 21c1-4 4-6 8-6s7 2 8 6"/>') +
    sym("i-warn", '<path d="M12 3l10 18H2zM12 10v5M12 18v.5"/>') +
    sym("i-min", '<path d="M6 12h12"/>') +
    sym("i-close", '<path d="M6 6l12 12M18 6L6 18"/>') +
    sym("i-chevron", '<path d="M6 9l6 6 6-6"/>') +
    sym("i-signal", '<path d="M5 20v-3M10 20v-7M15 20v-11M20 20V4"/>') +
    sym("i-rebuild", '<path d="M20 11a8 8 0 0 0-14-5L4 8M4 4v4h4M4 13a8 8 0 0 0 14 5l2-2M20 20v-4h-4"/>') +
    sym("i-copy", '<path d="M8 8h12v12H8zM4 16V4h12"/>') +
    sym("i-clock", '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>') +
    "</svg>";
  document.addEventListener("DOMContentLoaded", () => document.body.insertAdjacentHTML("afterbegin", sprite));
})();
