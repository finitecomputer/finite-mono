const assert = require("node:assert/strict");
const test = require("node:test");
const {
  fullBleedWindowOptions,
  hiddenElectronDashboardBrandCss,
  navigationActionForUrl,
  navigationToolbarBounds,
  navigationToolbarHeight,
  navigationToolbarWidth,
} = require("./window-chrome.cjs");

test("macOS windows use a full-bleed hidden title bar", () => {
  assert.deepEqual(fullBleedWindowOptions("darwin"), {
    titleBarStyle: "hiddenInset",
    trafficLightPosition: { x: 14, y: 14 },
  });
  assert.deepEqual(fullBleedWindowOptions("linux"), {});
  assert.deepEqual(fullBleedWindowOptions("win32"), {});
});

test("Electron dashboard CSS hides only its two wordmark placements", () => {
  assert.match(hiddenElectronDashboardBrandCss, /\.ocean-app-header__brand \.ocean-brand/u);
  assert.match(hiddenElectronDashboardBrandCss, /\.finite-chat__brand \.ocean-brand/u);
  assert.doesNotMatch(hiddenElectronDashboardBrandCss, /display:\s*none/u);
  assert.match(hiddenElectronDashboardBrandCss, /\.ocean-app-header,\s*.finite-chat__sidebar-top/u);
  assert.match(hiddenElectronDashboardBrandCss, /app-region:\s*drag/u);
  assert.match(
    hiddenElectronDashboardBrandCss,
    /\.ocean-app-header :where\(a, button, input, select, textarea, \[role="button"\]\)/u
  );
  assert.match(hiddenElectronDashboardBrandCss, /app-region:\s*no-drag/u);
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
