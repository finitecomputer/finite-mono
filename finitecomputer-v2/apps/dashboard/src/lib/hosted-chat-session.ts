import { safeWorkosReturnPathname } from "@/lib/workos-auth";

const SIGN_IN_REDIRECT_STORAGE_KEY = "fc-hosted-chat-sign-in-redirected-at";
const SIGN_IN_REDIRECT_BOUNCE_WINDOW_MS = 10_000;

/**
 * A chat request that came back 401 means the WorkOS session itself is gone
 * ("Session ended due to inactivity" and friends). Retrying the request can
 * never succeed, so the hosted chat client treats it as a session-level
 * terminal state rather than a transport error: no dead "Retry load" button,
 * just a full-page trip through sign-in.
 *
 * A hosted-device core envelope carries its own retry decision; a 401 the
 * core explicitly marked retryable is a service-auth problem, not the
 * viewer's session, and keeps the ordinary transport handling.
 */
export function isHostedChatSessionAuthFailure(error: unknown): boolean {
  if (typeof error !== "object" || error === null) return false;
  const candidate = error as { status?: unknown; retryable?: unknown };
  return candidate.status === 401 && candidate.retryable !== true;
}

/**
 * The full-page sign-in destination for an expired chat session: the same
 * `/login?returnTo=…` surface every other dashboard flow uses. The /login
 * route re-sanitizes `returnTo` server-side before redirecting to WorkOS.
 */
export function hostedChatSignInUrl(returnPath: string | null | undefined): string {
  return `/login?returnTo=${encodeURIComponent(safeWorkosReturnPathname(returnPath))}`;
}

/** The chat page the person was on, so sign-in can bring them straight back. */
export function currentHostedChatReturnPath(): string | null {
  if (typeof window === "undefined") return null;
  return `${window.location.pathname}${window.location.search}`;
}

let autoRedirectedThisPageLoad = false;

/**
 * Send the person to sign-in with a full-page navigation — never client-side
 * routing. A dead session can only be repaired by leaving the page, and the
 * hard navigation also replaces any stale deployment bundle a zombie tab is
 * still running.
 *
 * Loop guard: at most one automatic redirect per page load (a fresh page load
 * resets the flag naturally), and none at all when the previous redirect left
 * within the bounce window — a return trip that 401s again that fast cannot
 * be fixed by bouncing, so it lands on the "Sign in again" banner instead.
 * `force` bypasses the guard for the banner's explicit button.
 */
export function redirectToHostedChatSignIn(
  returnPath: string | null | undefined,
  { force = false }: { force?: boolean } = {}
): boolean {
  if (!force && (autoRedirectedThisPageLoad || signInRedirectWithinBounceWindow())) {
    return false;
  }
  autoRedirectedThisPageLoad = true;
  markSignInRedirect();
  if (typeof window !== "undefined") {
    window.location.assign(hostedChatSignInUrl(returnPath));
  }
  return true;
}

/** Clear the once-per-page-load and bounce-window redirect guard state. */
export function resetHostedChatSignInRedirect(): void {
  autoRedirectedThisPageLoad = false;
  try {
    sessionStorage.removeItem(SIGN_IN_REDIRECT_STORAGE_KEY);
  } catch {
    // Storage can be unavailable (privacy mode); the module flag still guards.
  }
}

function markSignInRedirect(): void {
  try {
    sessionStorage.setItem(
      SIGN_IN_REDIRECT_STORAGE_KEY,
      String(Date.now())
    );
  } catch {
    // Storage can be unavailable (privacy mode); the per-page-load flag
    // still bounds automatic redirects for this document.
  }
}

function signInRedirectWithinBounceWindow(): boolean {
  let markedAt: number | null = null;
  try {
    const raw = sessionStorage.getItem(SIGN_IN_REDIRECT_STORAGE_KEY);
    const parsed = raw === null ? Number.NaN : Number(raw);
    markedAt = Number.isFinite(parsed) ? parsed : null;
  } catch {
    return false;
  }
  return markedAt !== null && Date.now() - markedAt < SIGN_IN_REDIRECT_BOUNCE_WINDOW_MS;
}
