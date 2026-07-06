type BrowserStorage = Pick<Storage, "key" | "length" | "removeItem">;
type BrowserWindowStorage = {
  localStorage?: BrowserStorage;
  sessionStorage?: BrowserStorage;
};

const FINITE_BROWSER_STATE_PREFIX = "finite.";

export function finiteBrowserStateKeys(storage: Pick<BrowserStorage, "key" | "length">) {
  const keys: string[] = [];
  for (let index = 0; index < storage.length; index += 1) {
    const key = storage.key(index);
    if (key?.startsWith(FINITE_BROWSER_STATE_PREFIX)) {
      keys.push(key);
    }
  }
  return keys;
}

export function clearFiniteBrowserStorage(storage: BrowserStorage | undefined) {
  if (!storage) {
    return [];
  }

  const keys = finiteBrowserStateKeys(storage);
  for (const key of keys) {
    storage.removeItem(key);
  }
  return keys;
}

export function clearFiniteBrowserSessionState(
  win: BrowserWindowStorage | undefined = typeof window === "undefined" ? undefined : window
) {
  if (!win) {
    return { localStorage: [], sessionStorage: [] };
  }

  let sessionStorage: string[] = [];
  let localStorage: string[] = [];

  try {
    sessionStorage = clearFiniteBrowserStorage(win.sessionStorage);
  } catch {
    sessionStorage = [];
  }

  try {
    localStorage = clearFiniteBrowserStorage(win.localStorage);
  } catch {
    localStorage = [];
  }

  return { localStorage, sessionStorage };
}
