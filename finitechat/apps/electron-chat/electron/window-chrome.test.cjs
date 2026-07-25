const assert = require("node:assert/strict");
const test = require("node:test");
const {
  electronDashboardChromeCss,
  fullBleedWindowOptions,
  navigationActionForUrl,
  navigationToolbarBounds,
  navigationToolbarHeight,
  navigationToolbarWidth,
} = require("./window-chrome.cjs");

test("macOS windows use a full-bleed hidden title bar", () => {
  assert.deepEqual(fullBleedWindowOptions("darwin"), {
    titleBarStyle: "hiddenInset",
    trafficLightPosition: { x: 20, y: 20 },
  });
  assert.deepEqual(fullBleedWindowOptions("linux"), {});
  assert.deepEqual(fullBleedWindowOptions("win32"), {});
});

test("Electron dashboard CSS hides only its two wordmark placements", () => {
  const css = electronDashboardChromeCss("darwin");
  assert.match(css, /\.ocean-app-header__brand \.ocean-brand/u);
  assert.match(css, /\.finite-chat__brand \.ocean-brand/u);
  assert.match(css, /visibility:\s*hidden\s*!important/u);
  assert.doesNotMatch(
    css,
    /\.finite-chat__brand \.ocean-brand\s*\{[^}]*display:\s*none/su
  );
});

test("macOS dashboard CSS reserves traffic-light space and exposes draggable chrome", () => {
  const css = electronDashboardChromeCss("darwin");
  assert.match(css, /\.finite-chat__topbar/u);
  assert.match(css, /app-region:\s*drag/u);
  assert.match(
    css,
    /\.finite-chat__desktop-collapse-button,\s*.finite-chat__mobile-collapse-button/u
  );
  assert.match(css, /\.finite-chat__topbar-actions > button/u);
  assert.match(css, /app-region:\s*no-drag/u);
  assert.match(
    css,
    /\.finite-agent-shell\.is-sidebar-collapsed \.finite-chat__desktop-collapse-button/u
  );
  assert.match(
    css,
    /\.finite-agent-shell\.is-sidebar-collapsed \.finite-chat__sidebar-toggle/u
  );
  assert.match(css, /display:\s*none/u);
  assert.match(css, /display:\s*inline-flex/u);
  assert.match(css, /\.finite-chat__sidebar-top\s*\{[^}]*min-height:\s*40px/su);
  assert.match(css, /\.finite-chat__topbar\s*\{[^}]*min-height:\s*40px/su);
  assert.doesNotMatch(css, /position:\s*fixed/u);
});

test("non-macOS dashboard CSS does not alter native window chrome", () => {
  const css = electronDashboardChromeCss("win32");
  assert.match(css, /\.finite-chat__brand \.ocean-brand/u);
  assert.doesNotMatch(css, /app-region/u);
  assert.doesNotMatch(css, /is-sidebar-collapsed/u);
});

test("navigation toolbar stays centered without covering page or window controls", () => {
  assert.deepEqual(navigationToolbarBounds({ width: 1280, height: 860 }), {
    x: (1280 - navigationToolbarWidth) / 2,
    y: 0,
    width: navigationToolbarWidth,
    height: navigationToolbarHeight,
  });
  assert.deepEqual(navigationToolbarBounds({ width: -1 }), {
    x: 0,
    y: 0,
    width: 0,
    height: navigationToolbarHeight,
  });
});

test("only exact app-owned history commands are accepted", () => {
  assert.equal(navigationActionForUrl("finitechat-navigation://back"), "back");
  assert.equal(navigationActionForUrl("finitechat-navigation://forward"), "forward");
  assert.equal(navigationActionForUrl("finitechat-navigation://reload"), null);
  assert.equal(navigationActionForUrl("finitechat-navigation://back/path"), null);
  assert.equal(navigationActionForUrl("finitechat-navigation://back?then=https://evil.example"), null);
  assert.equal(navigationActionForUrl("https://finitechat-navigation/back"), null);
  assert.equal(navigationActionForUrl("not a url"), null);
});
