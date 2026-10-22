// Next 16.2's Webpack dev server awaits this optional version lookup before
// serving requests (hot-reloader-shared-utils.getVersionInfo). Keep the browser
// fixtures bounded when npm is unavailable. Only test:browser preloads this;
// application, Core, Chat, Sites and authentication requests pass through intact.
export function boundNextVersionCheck(fetch) {
  return function (input, init) {
    const url = typeof input === "string" || input instanceof URL ? String(input) : input.url;
    if (url !== "https://registry.npmjs.org/-/package/next/dist-tags") {
      return fetch(input, init);
    }
    const prior = init?.signal ?? (input instanceof Request ? input.signal : undefined);
    const deadline = AbortSignal.timeout(1_000);
    const signal = prior ? AbortSignal.any([prior, deadline]) : deadline;
    return fetch(input, { ...init, signal });
  };
}

globalThis.fetch = boundNextVersionCheck(globalThis.fetch);
