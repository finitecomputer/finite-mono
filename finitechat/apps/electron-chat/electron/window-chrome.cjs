const navigationToolbarHeight = 44;
const navigationToolbarWidth = 62;
const navigationCommandScheme = "finitechat-navigation:";
const hiddenElectronDashboardBrandCss = `
  .ocean-app-header__brand .ocean-brand,
  .finite-chat__brand .ocean-brand {
    visibility: hidden !important;
  }

  .ocean-app-header,
  .finite-chat__sidebar-top {
    -webkit-app-region: drag;
    app-region: drag;
  }

  .ocean-app-header :where(a, button, input, select, textarea, [role="button"]),
  .finite-chat__sidebar-top :where(a, button, input, select, textarea, [role="button"]) {
    -webkit-app-region: no-drag;
    app-region: no-drag;
  }
`;

function fullBleedWindowOptions(platform = process.platform) {
  if (platform !== "darwin") {
    return {};
  }
  return {
    titleBarStyle: "hiddenInset",
    trafficLightPosition: { x: 14, y: 14 },
  };
}

function navigationToolbarBounds(contentBounds) {
  const width = Number.isFinite(contentBounds?.width)
    ? Math.max(0, Math.floor(contentBounds.width))
    : 0;
  const toolbarWidth = Math.min(width, navigationToolbarWidth);
  return {
    x: Math.max(0, Math.floor((width - toolbarWidth) / 2)),
    y: 0,
    width: toolbarWidth,
    height: navigationToolbarHeight,
  };
}

function navigationActionForUrl(value) {
  let url;
  try {
    url = new URL(value);
  } catch {
    return null;
  }
  if (url.protocol !== navigationCommandScheme || url.username || url.password) {
    return null;
  }
  if (url.port || url.search || url.hash || url.pathname) {
    return null;
  }
  if (url.hostname === "back") {
    return "back";
  }
  if (url.hostname === "forward") {
    return "forward";
  }
  return null;
}

module.exports = {
  fullBleedWindowOptions,
  hiddenElectronDashboardBrandCss,
  navigationActionForUrl,
  navigationToolbarBounds,
  navigationToolbarHeight,
  navigationToolbarWidth,
};
