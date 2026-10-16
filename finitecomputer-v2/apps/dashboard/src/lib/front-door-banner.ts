// Per-session dismissal for the "Try the new Finite" banner. The key uses the
// `finite.` prefix so sign-out clears it with the rest of Finite browser state
// (see browser-session.ts); a new sign-in shows the banner again.

export const FRONT_DOOR_BANNER_DISMISSED_KEY = "finite.frontDoorBanner.dismissed";

type BannerStorage = Pick<Storage, "getItem" | "setItem">;

export function createFrontDoorBannerStore(storage: () => BannerStorage | null) {
  // Browsers that refuse sessionStorage still hide the banner for this page.
  let dismissedThisPage = false;
  const listeners = new Set<() => void>();
  return {
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    dismissed() {
      if (dismissedThisPage) return true;
      try {
        return storage()?.getItem(FRONT_DOOR_BANNER_DISMISSED_KEY) === "1";
      } catch {
        return false;
      }
    },
    dismiss() {
      dismissedThisPage = true;
      try {
        storage()?.setItem(FRONT_DOOR_BANNER_DISMISSED_KEY, "1");
      } catch {
        // Privacy modes can refuse storage; the page-level flag still applies.
      }
      for (const listener of listeners) listener();
    },
  };
}

export function browserSessionStorage(): BannerStorage | null {
  try {
    return typeof window === "undefined" ? null : window.sessionStorage;
  } catch {
    return null;
  }
}
