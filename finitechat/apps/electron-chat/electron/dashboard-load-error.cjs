function shouldReplaceFailedDashboardDocument(
  { resourceType, statusCode, url, webContentsId } = {},
  { currentUrl, dashboardWebContentsId } = {}
) {
  return (
    resourceType === "mainFrame"
    && Number.isInteger(statusCode)
    && statusCode >= 500
    && statusCode <= 599
    && Number.isInteger(webContentsId)
    && webContentsId === dashboardWebContentsId
    && typeof url === "string"
    && url === currentUrl
  );
}

function dashboardLoadErrorScript(logoDataUrl) {
  if (
    typeof logoDataUrl !== "string"
    || !logoDataUrl.startsWith("data:image/svg+xml;base64,")
  ) {
    throw new TypeError("Finite dashboard failure UI requires its packaged logo");
  }
  return `(() => {
    const logoDataUrl = ${JSON.stringify(logoDataUrl)};
    const head = document.createElement("head");
    const meta = document.createElement("meta");
    meta.name = "color-scheme";
    meta.content = "light dark";
    head.append(meta);

    const style = document.createElement("style");
    style.textContent = \`
      * { box-sizing: border-box; }
      html, body { width: 100%; height: 100%; min-height: 100vh; margin: 0; }
      body {
        display: grid;
        place-items: center;
        padding: 32px;
        color: CanvasText;
        background:
          radial-gradient(circle at 50% 38%, rgba(10, 132, 255, 0.08), transparent 34%),
          Canvas;
        font: 15px/1.45 -apple-system, BlinkMacSystemFont, sans-serif;
      }
      main { width: min(420px, 100%); text-align: center; }
      .mark {
        display: block;
        width: 78px;
        height: 78px;
        margin: 0 auto 24px;
      }
      h1 { margin: 0 0 10px; font-size: 25px; letter-spacing: -0.02em; }
      p { margin: 0 auto 24px; max-width: 36ch; color: GrayText; }
      button {
        min-width: 132px;
        min-height: 42px;
        padding: 0 20px;
        border: 0;
        border-radius: 12px;
        color: white;
        background: #0a84ff;
        font: inherit;
        font-weight: 600;
        cursor: pointer;
      }
      button:hover { filter: brightness(1.06); }
      button:active { transform: scale(0.98); }
      button:focus-visible { outline: 3px solid Highlight; outline-offset: 3px; }
    \`;
    head.append(style);

    const body = document.createElement("body");
    const main = document.createElement("main");
    const mark = document.createElement("img");
    mark.className = "mark";
    mark.src = logoDataUrl;
    mark.alt = "";
    mark.setAttribute("aria-hidden", "true");
    const heading = document.createElement("h1");
    heading.textContent = "Finite is temporarily unavailable";
    const explanation = document.createElement("p");
    explanation.textContent =
      "The service did not finish loading. Your local chat data is safe.";
    const retry = document.createElement("button");
    retry.type = "button";
    retry.textContent = "Try again";
    retry.addEventListener("click", () => location.reload());
    main.append(mark, heading, explanation, retry);
    body.append(main);
    document.title = "Finite is temporarily unavailable";
    document.documentElement.replaceChildren(head, body);
  })()`;
}

module.exports = {
  dashboardLoadErrorScript,
  shouldReplaceFailedDashboardDocument,
};
