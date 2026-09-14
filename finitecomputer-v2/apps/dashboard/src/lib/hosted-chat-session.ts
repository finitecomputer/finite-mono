import { CHAT_SIGN_IN_DRAFT_UNSAVED_MESSAGE } from "@/lib/chat-product-copy";
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

/**
 * Decide whether an observed session failure may navigate away on its own.
 * Unsent composer input (draft text or staged attachments) lives only in
 * React state, so an automatic full-page navigation would destroy it without
 * the person asking: with input present the composer stays mounted and the
 * banner's explicit "Sign in again" carries the trip (persisting the draft
 * first). An empty composer keeps the automatic redirect.
 */
export function shouldAutoRedirectForSessionAuthFailure(composer: {
  draftText: string;
  attachmentCount: number;
}): boolean {
  return composer.draftText.trim().length === 0 && composer.attachmentCount === 0;
}

const HOSTED_CHAT_DRAFT_PREFIX = "finite.chat-draft.";
const HOSTED_CHAT_DRAFT_TTL_MS = 15 * 60_000;
const HOSTED_CHAT_DRAFT_MAX_CHARS = 64 * 1024;

type HostedChatDraftStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;

type StoredHostedChatDraft = {
  text?: unknown;
  savedAtMs?: unknown;
};

/** The per-machine localStorage key for a draft parked across a sign-in trip. */
export function hostedChatDraftStorageKey(machineId: string): string {
  return `${HOSTED_CHAT_DRAFT_PREFIX}${encodeURIComponent(machineId)}`;
}

/**
 * The one shape rule both parking and restoring apply, so anything save
 * accepts, restore accepts — a successful save can never strand a draft the
 * restore side would reject and delete.
 */
function isRestorableHostedChatDraft(text: string): boolean {
  return text.trim().length > 0 && text.length <= HOSTED_CHAT_DRAFT_MAX_CHARS;
}

/**
 * Park the composer's draft text so the sign-in round trip can bring it back.
 * The `finite.` prefix means signing out clears it with the rest of the
 * dashboard's browser state. True means the trip may proceed: the text was
 * blank (nothing to lose, any stale key removed) or is parked and restorable.
 * False means there is unsent text that could not be parked — do not
 * navigate away from it.
 */
export function saveHostedChatDraft(
  machineId: string,
  text: string,
  options: { nowMs?: number; storage?: HostedChatDraftStorage | null } = {}
): boolean {
  const storage = options.storage !== undefined ? options.storage : defaultDraftStorage();
  const key = hostedChatDraftStorageKey(machineId);
  if (text.trim().length === 0) {
    if (storage) tryRemoveDraft(storage, key);
    return true;
  }
  if (!storage || !isRestorableHostedChatDraft(text)) return false;
  try {
    storage.setItem(key, JSON.stringify({
      text,
      savedAtMs: options.nowMs ?? Date.now(),
    } satisfies StoredHostedChatDraft));
    return true;
  } catch {
    // Quota or privacy mode: the in-page composer still holds the text.
    return false;
  }
}

/**
 * Restore a parked draft for this machine within its TTL; consuming or
 * expiring it clears the key so a draft is a one-shot handoff, not a
 * persistent archive.
 */
export function loadHostedChatDraft(
  machineId: string,
  options: { nowMs?: number; storage?: HostedChatDraftStorage | null } = {}
): string | null {
  const storage = options.storage !== undefined ? options.storage : defaultDraftStorage();
  if (!storage) return null;
  const key = hostedChatDraftStorageKey(machineId);
  let raw: string | null;
  try {
    raw = storage.getItem(key);
  } catch {
    return null;
  }
  if (raw === null) return null;
  let text: string | null = null;
  try {
    const parsed = JSON.parse(raw) as StoredHostedChatDraft;
    if (typeof parsed.text === "string" && isRestorableHostedChatDraft(parsed.text)) {
      const savedAtMs = typeof parsed.savedAtMs === "number" ? parsed.savedAtMs : Number.NaN;
      const nowMs = options.nowMs ?? Date.now();
      text = Number.isFinite(savedAtMs) && nowMs - savedAtMs < HOSTED_CHAT_DRAFT_TTL_MS
        ? parsed.text
        : null;
    }
  } catch {
    text = null;
  }
  if (text === null) {
    tryRemoveDraft(storage, key);
    return null;
  }
  return text;
}

/** Drop a parked draft (consumed, superseded by an explicit `?prompt=`, …). */
export function clearHostedChatDraft(
  machineId: string,
  options: { storage?: HostedChatDraftStorage | null } = {}
): void {
  const storage = options.storage !== undefined ? options.storage : defaultDraftStorage();
  if (!storage) return;
  tryRemoveDraft(storage, hostedChatDraftStorageKey(machineId));
}

/**
 * The composer draft for a fresh page load. A draft parked by the sign-in
 * round trip resumes the person's newer edits, so it outranks any `?prompt=`
 * the return URL still carries; consuming it clears the key. With nothing
 * parked, the `?prompt=` prefills as usual.
 */
export function restoreHostedChatComposerDraft(
  machineId: string,
  initialDraft: string,
  options: { nowMs?: number; storage?: HostedChatDraftStorage | null } = {}
): string {
  const parked = loadHostedChatDraft(machineId, options);
  if (parked === null) return initialDraft;
  clearHostedChatDraft(machineId, options);
  return parked;
}

/**
 * The explicit "Sign in again" click: park the composer's draft, then start
 * the full-page sign-in trip. Returns a banner message when the trip must
 * not proceed because unsent text could not be parked (the composer stays
 * mounted with it); null once navigation has started.
 */
export function attemptHostedChatSignIn(
  machineId: string,
  draftText: string,
  options: { storage?: HostedChatDraftStorage | null } = {}
): string | null {
  if (!saveHostedChatDraft(machineId, draftText, options)) {
    return CHAT_SIGN_IN_DRAFT_UNSAVED_MESSAGE;
  }
  redirectToHostedChatSignIn(currentHostedChatReturnPath(), { force: true });
  return null;
}

function tryRemoveDraft(storage: HostedChatDraftStorage, key: string): void {
  try {
    storage.removeItem(key);
  } catch {
    // Best effort; the TTL bounds any key we fail to remove.
  }
}

function defaultDraftStorage(): HostedChatDraftStorage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}
