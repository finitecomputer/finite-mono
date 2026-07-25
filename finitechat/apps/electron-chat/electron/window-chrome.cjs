const navigationToolbarHeight = 44;
const navigationToolbarWidth = 62;
const navigationCommandScheme = "finitechat-navigation:";
const macTrafficLightInset = 14;

function electronDashboardChromeCss(platform = process.platform) {
  const macWindowChrome = platform === "darwin"
    ? `
      .ocean-app-header,
      .finite-chat__sidebar-top,
      .finite-chat__topbar {
        -webkit-app-region: drag;
        app-region: drag;
      }

      .ocean-app-header :where(a, button, input, select, textarea, [role="button"]),
      .finite-chat__desktop-collapse-button,
      .finite-chat__mobile-collapse-button,
      .finite-chat__sidebar-toggle,
      .finite-chat__rename-button,
      .finite-chat__topbar-actions > button {
        -webkit-app-region: no-drag;
        app-region: no-drag;
      }

      .finite-chat__sidebar-top {
        min-height: 40px;
        padding-top: 4px;
        padding-bottom: 4px;
      }

      .finite-chat__topbar {
        min-height: 40px;
        padding-top: 4px;
        padding-bottom: 4px;
      }

      .finite-agent-shell.is-sidebar-collapsed .finite-chat__desktop-collapse-button {
        display: none;
      }

      .finite-agent-shell.is-sidebar-collapsed .finite-chat__sidebar-toggle {
        display: inline-flex;
      }
    `
    : "";

  return `
    .ocean-app-header__brand .ocean-brand,
    .finite-chat__brand .ocean-brand {
      visibility: hidden !important;
    }

    ${macWindowChrome}
  `;
}

function fullBleedWindowOptions(platform = process.platform) {
  if (platform !== "darwin") {
    return {};
  }
  return {
    titleBarStyle: "hiddenInset",
    trafficLightPosition: { x: macTrafficLightInset, y: macTrafficLightInset },
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
  electronDashboardChromeCss,
  fullBleedWindowOptions,
  navigationActionForUrl,
  navigationToolbarBounds,
  navigationToolbarHeight,
  navigationToolbarWidth,
};
