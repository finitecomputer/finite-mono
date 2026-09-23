"use client";

import type { ReactNode } from "react";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";

import {
  CHAT_INVALID_UPDATE_MESSAGE,
  CHAT_NAVIGATION_TIMEOUT_MESSAGE,
  CHAT_UNAVAILABLE_MESSAGE,
} from "@/lib/chat-product-copy";
import type { HostedChatAction, HostedChatState } from "@/lib/hosted-web-device";
import {
  beginHostedChatStreamConnection,
  hostedChatStreamSnapshotProvesRestart,
  initialHostedChatSnapshotSource,
  nextHostedChatSnapshotGeneration,
  recordHostedChatSnapshot,
  shouldApplyHttpHostedChatSnapshot,
  shouldApplyMutationHostedChatSnapshot,
  shouldApplyStreamHostedChatSnapshot,
} from "@/lib/hosted-web-chat-snapshots";
import {
  attemptHostedChatSignIn,
  currentHostedChatReturnPath,
  isHostedChatSessionAuthFailure,
  redirectToHostedChatSignIn,
  shouldAutoRedirectForSessionAuthFailure,
} from "@/lib/hosted-chat-session";
import {
  runInitialHostedChatRetries,
  shouldRetryHostedChatRequest,
  type HostedChatRetryAttempt,
} from "@/lib/hosted-web-chat-retry";
import {
  HOSTED_CHAT_NAVIGATION_TIMEOUT_MS,
  hostedChatSelectionFromState,
  hostedChatSelectionIntentTarget,
  hostedChatSelectionIntentSatisfied,
  hostedChatSnapshotMatchesSelection,
  type HostedChatSelection,
  type HostedChatSelectionIntent,
} from "@/lib/hosted-web-chat-selection";
import {
  pendingChatRefreshAdvancesTranscript,
  type PendingChatRefreshTarget,
} from "@/lib/hosted-web-chat-refresh";

import { hostedChatViewQuery } from "@/lib/hosted-chat-view";

const STREAM_RECONNECT_DELAY_MS = 1_000;

type MutationSnapshotRequest = {
  allowEqualRevision: boolean;
  generation: number;
  highestRev: number;
  sequence: number;
};

export type HostedChatContextValue = {
  apiBase: string;
  state: HostedChatState | null;
  transportError: string | null;
  claimError: string | null;
  sessionError: string | null;
  streamConnected: boolean;
  ownerClaimed: boolean;
  bindingRecoveryRequired: boolean;
  selectionPending: boolean;
  load: (showError?: boolean) => Promise<HostedChatRetryAttempt>;
  claimOwner: (showError?: boolean) => Promise<HostedChatRetryAttempt>;
  recoverBinding: () => Promise<HostedChatRetryAttempt>;
  /**
   * Terminal session handling for a caught chat error: true when the error
   * was a 401 session failure (the caller must not also present it as a
   * transport/claim/action error). With unsent composer input the composer
   * stays mounted and only the banner's "Sign in again" navigates.
   */
  reportSessionAuthFailure: (error: unknown) => boolean;
  /** Full-page sign-in trip; parks the current draft text first. */
  signInAgain: () => void;
  /** Keeps the provider current on unsent composer input (draft, attachments). */
  noteComposerInput: (input: { draftText: string; attachmentCount: number }) => void;
  dispatch: (action: HostedChatAction) => Promise<HostedChatState>;
  dispatchQuiet: (action: HostedChatAction) => Promise<HostedChatState | null>;
  refreshPendingChat: (target: PendingChatRefreshTarget) => Promise<boolean>;
  uploadAttachments: (formData: FormData) => Promise<HostedChatState>;
  interruptTurn?: () => Promise<void>;
  pendingAgentAction?: ReactNode;
  attachmentUrl: (address: {
    room_id: string;
    message_id: string;
    attachment_id: string;
  }) => string;
};

export const HostedChatContext = createContext<HostedChatContextValue | null>(null);

export function HostedChatProvider({
  children,
  machineId,
}: {
  children: ReactNode;
  machineId: string;
}) {
  const apiBase = `/api/chat/machines/${encodeURIComponent(machineId)}/hosted-device`;
  const [state, setState] = useState<HostedChatState | null>(null);
  const [transportError, setTransportError] = useState<string | null>(null);
  const [claimError, setClaimError] = useState<string | null>(null);
  const [sessionError, setSessionError] = useState<string | null>(null);
  const [streamConnected, setStreamConnected] = useState(false);
  const [ownerClaimed, setOwnerClaimed] = useState(false);
  const [bindingRecoveryRequired, setBindingRecoveryRequired] = useState(false);
  const [selectionPending, setSelectionPending] = useState(false);
  const stateRef = useRef<HostedChatState | null>(null);
  const snapshotSourceRef = useRef(initialHostedChatSnapshotSource());
  const stateLoadRef = useRef<Promise<HostedChatRetryAttempt> | null>(null);
  const lastLoadErrorRef = useRef<string | null>(null);
  const ownerClaimRef = useRef<Promise<HostedChatRetryAttempt> | null>(null);
  const lastClaimErrorRef = useRef<string | null>(null);
  const sessionAuthFailureRef = useRef(false);
  const composerInputRef = useRef({ draftText: "", attachmentCount: 0 });
  const navigationMutationTailRef = useRef<Promise<void>>(Promise.resolve());
  const nextMutationSequenceRef = useRef(0);
  const latestAppliedMutationSequenceRef = useRef(0);
  const snapshotSequenceRef = useRef(0);
  const selectionIntentRef = useRef<HostedChatSelectionIntent | null>(null);
  const selectionIntentTokenRef = useRef(0);
  const coherentStateRef = useRef<HostedChatState | null>(null);
  const messageLimitRef = useRef(50);
  const [messageLimit, setMessageLimit] = useState(50);
  const oldestMessageIdRef = useRef<string | null>(null);
  const [oldestMessageId, setOldestMessageId] = useState<string | null>(null);
  const localSelectionRef = useRef<HostedChatSelection | null>(null);
  const hasState = state !== null;

  const viewQuery = hostedChatViewQuery(
    state ? hostedChatSelectionFromState(state) : null,
    messageLimit,
    oldestMessageId
  );
  const currentViewQuery = useCallback(
    () => hostedChatViewQuery(localSelectionRef.current, messageLimitRef.current, oldestMessageIdRef.current),
    []
  );

  // A snapshot is one coherent selection and transcript. A late response or
  // an older service ignoring the view query cannot replace another Chat's
  // messages underneath our current heading.
  const matchesView = useCallback((next: HostedChatState) => {
    const intent = selectionIntentRef.current;
    const selectionMatches = intent
      ? hostedChatSelectionIntentSatisfied(intent, next)
      : hostedChatSnapshotMatchesSelection(localSelectionRef.current, next);
    const anchor = oldestMessageIdRef.current;
    // Delayed reads/stream events with a narrower window must not discard
    // history already loaded in this Chat. A missing Chat may still fall back.
    const sameChat = localSelectionRef.current?.selected_chat_id === next.selected_chat_id;
    return selectionMatches && (!sameChat || !anchor || next.messages.some(message => message.message_id === anchor));
  }, []);

  const setMergedState = useCallback((next: HostedChatState) => {
    localSelectionRef.current = hostedChatSelectionFromState(next);
    const anchor = next.messages.length > 50 ? next.messages[0].message_id : null;
    oldestMessageIdRef.current = anchor;
    setOldestMessageId(anchor);
    selectionIntentRef.current = null;
    setSelectionPending(false);
    const merged = {
      ...next,
      hosted_agent_binding: next.hosted_agent_binding === undefined
        ? stateRef.current?.hosted_agent_binding ?? null
        : next.hosted_agent_binding,
    };
    coherentStateRef.current = merged;
    stateRef.current = merged;
    setState(merged);
  }, []);

  const applyHttpSnapshot = useCallback((next: HostedChatState, requestGeneration: number) => {
    const source = snapshotSourceRef.current;
    if (!matchesView(next) || !shouldApplyHttpHostedChatSnapshot(source, requestGeneration, next.rev)) {
      return false;
    }
    snapshotSourceRef.current = recordHostedChatSnapshot(source, next.rev, false);
    snapshotSequenceRef.current += 1;
    setMergedState(next);
    return true;
  }, [matchesView, setMergedState]);

  // A fresh view can change selection or its history window without advancing
  // the Device revision. Requests run concurrently so
  // navigation cannot wait behind typing/read receipts or uploads; the client
  // sequence prevents an older equal-revision response from rolling back a
  // newer mutation response.
  const applyMutationSnapshot = useCallback((
    next: HostedChatState,
    request: MutationSnapshotRequest
  ) => {
    const source = snapshotSourceRef.current;
    if (!matchesView(next) || !shouldApplyMutationHostedChatSnapshot(
      source,
      request.generation,
      request.highestRev,
      request.sequence,
      latestAppliedMutationSequenceRef.current,
      request.allowEqualRevision,
      next.rev
    )) {
      return false;
    }
    latestAppliedMutationSequenceRef.current = Math.max(
      latestAppliedMutationSequenceRef.current,
      request.sequence
    );
    snapshotSourceRef.current = recordHostedChatSnapshot(source, next.rev, false);
    snapshotSequenceRef.current += 1;
    setMergedState(next);
    return true;
  }, [matchesView, setMergedState]);

  // A 401 from a hosted-device chat route means the WorkOS session itself is
  // dead. That is not a transport error: retrying can never succeed, so the
  // person is routed to sign-in by full-page navigation (which also replaces
  // any stale deployment bundle a zombie tab is still running) and the chat
  // error surfaces keep clear of the dead "Retry load" path. Unsent composer
  // input must not be navigated away from automatically — it only exists in
  // React state — so with a draft or staged attachments the composer stays
  // put and the banner's explicit "Sign in again" carries the trip.
  const reportSessionAuthFailure = useCallback((error: unknown) => {
    if (!isHostedChatSessionAuthFailure(error)) return false;
    sessionAuthFailureRef.current = true;
    setSessionError(hostedChatErrorMessage(error));
    setTransportError(null);
    setClaimError(null);
    if (shouldAutoRedirectForSessionAuthFailure(composerInputRef.current)) {
      redirectToHostedChatSignIn(currentHostedChatReturnPath());
    }
    return true;
  }, []);

  const signInAgain = useCallback(() => {
    // Navigate only when there is no unsent text to lose or it is safely
    // parked; otherwise the composer stays mounted and the banner says why.
    const unsaved = attemptHostedChatSignIn(machineId, composerInputRef.current.draftText);
    if (unsaved) setSessionError(unsaved);
  }, [machineId]);

  // The chat surface keeps this current so a session failure knows whether
  // the composer holds unsent input. A ref, not state: keystrokes must not
  // re-render the provider.
  const noteComposerInput = useCallback((input: {
    draftText: string;
    attachmentCount: number;
  }) => {
    composerInputRef.current = input;
  }, []);

  const load = useCallback((showError = true) => {
    if (stateLoadRef.current) return stateLoadRef.current;
    const requestGeneration = snapshotSourceRef.current.generation;
    const pending = (async (): Promise<HostedChatRetryAttempt> => {
      try {
        // In development, React may tear down and immediately remount the
        // mount effect while the shared request is still in flight; aborting
        // that request leaves the remount holding the same cancelled promise,
        // so the hosted browser load does not abort its request.
        const next = await hostedChatRequest<HostedChatState>(`${apiBase}/state${currentViewQuery()}`);
        applyHttpSnapshot(next, requestGeneration);
        setTransportError(null);
        setBindingRecoveryRequired(false);
        return "succeeded";
      } catch (caught) {
        if (reportSessionAuthFailure(caught)) return "stop";
        const message = hostedChatErrorMessage(caught);
        setBindingRecoveryRequired(
          caught instanceof HostedChatHttpError &&
          caught.code === "binding_authorization_required"
        );
        lastLoadErrorRef.current = message;
        if (showError) setTransportError(message);
        const http = caught instanceof HostedChatHttpError ? caught : null;
        return shouldRetryHostedChatRequest(http?.status ?? null, http?.retryable)
          ? "retry"
          : "stop";
      }
    })();
    stateLoadRef.current = pending;
    void pending.finally(() => {
      if (stateLoadRef.current === pending) stateLoadRef.current = null;
    });
    return pending;
  }, [apiBase, applyHttpSnapshot, currentViewQuery, reportSessionAuthFailure]);

  const claimOwner = useCallback((showError = true) => {
    if (ownerClaimRef.current) return ownerClaimRef.current;
    const pending = (async (): Promise<HostedChatRetryAttempt> => {
      try {
        await hostedChatRequest<{ claimed: true }>(`${apiBase}/claim`, { method: "POST" });
        setOwnerClaimed(true);
        setClaimError(null);
        return "succeeded";
      } catch (caught) {
        if (reportSessionAuthFailure(caught)) return "stop";
        const message = hostedChatErrorMessage(caught);
        lastClaimErrorRef.current = message;
        if (showError) setClaimError(message);
        const http = caught instanceof HostedChatHttpError ? caught : null;
        return shouldRetryHostedChatRequest(http?.status ?? null, http?.retryable)
          ? "retry"
          : "stop";
      }
    })();
    ownerClaimRef.current = pending;
    void pending.finally(() => {
      if (ownerClaimRef.current === pending) ownerClaimRef.current = null;
    });
    return pending;
  }, [apiBase, reportSessionAuthFailure]);

  const requestViewSnapshot = useCallback(async (
    signal?: AbortSignal | null,
    actionSnapshot?: HostedChatState
  ) => {
    const query = currentViewQuery();
    const source = snapshotSourceRef.current;
    const request: MutationSnapshotRequest = {
      generation: source.generation,
      highestRev: source.highestRev,
      sequence: ++nextMutationSequenceRef.current,
      allowEqualRevision: true,
    };
    let projected = !query && actionSnapshot
      ? actionSnapshot
      : await hostedChatRequest<HostedChatState>(`${apiBase}/state${query}`, { signal });
    if (actionSnapshot?.hosted_agent_binding !== undefined) {
      projected = { ...projected, hosted_agent_binding: actionSnapshot.hosted_agent_binding };
    }
    applyMutationSnapshot(projected, request);
    return projected;
  }, [apiBase, applyMutationSnapshot, currentViewQuery]);

  const requestMutationSnapshot = useCallback(async (
    path: string,
    init: RequestInit,
    allowEqualRevision = true,
    adoptSelection?: (next: HostedChatState) => boolean
  ) => {
    const next = await hostedChatRequest<HostedChatState>(`${apiBase}${path}`, init);
    // Typing/read receipts do not replace the transcript. For a user mutation,
    // read the current view: the mutation response belongs to the shared Device
    // cursor and may have a different Chat or history window.
    if (!allowEqualRevision || (adoptSelection && !adoptSelection(next))) return next;
    try {
      await requestViewSnapshot(init.signal, next);
    } catch (caught) {
      // The action already succeeded. A refresh outage must not report a
      // failed send and invite a duplicate; retain the coherent transcript
      // while the stream reconnects.
      if (!reportSessionAuthFailure(caught)) setTransportError(hostedChatErrorMessage(caught));
    }
    // The view's status/toast may belong to another tab's later action.
    // Callers decide whether to clear a draft from this action's own outcome.
    return next;
  }, [apiBase, requestViewSnapshot, reportSessionAuthFailure]);

  const recoverBinding = useCallback(async (): Promise<HostedChatRetryAttempt> => {
    try {
      const next = await requestMutationSnapshot("/recover-binding", { method: "POST" });
      setTransportError(null);
      setBindingRecoveryRequired(false);
      setOwnerClaimed(false);
      if (!next.hosted_agent_binding) {
        setTransportError(CHAT_UNAVAILABLE_MESSAGE);
        return "stop";
      }
      return "succeeded";
    } catch (caught) {
      if (reportSessionAuthFailure(caught)) return "stop";
      setTransportError(hostedChatErrorMessage(caught));
      return "stop";
    }
  }, [requestMutationSnapshot, reportSessionAuthFailure]);

  const requestActionSnapshot = useCallback((
    action: HostedChatAction,
    allowEqualRevision = true
  ) => {
    const navigationAction = isHostedChatNavigationAction(action);
    const target = hostedChatSelectionIntentTarget(action);
    const token = navigationAction ? ++selectionIntentTokenRef.current : null;
    if (navigationAction) {
      messageLimitRef.current = 50;
      setMessageLimit(50);
      oldestMessageIdRef.current = null;
      setOldestMessageId(null);
    }
    if ("LoadOlderMessages" in action) {
      messageLimitRef.current = Math.max(messageLimitRef.current, coherentStateRef.current?.messages.length ?? 0)
        + action.LoadOlderMessages.limit;
      setMessageLimit(messageLimitRef.current);
      // Paging is a tab-local read. The legacy action changes the shared
      // Device cursor/window and publishes a revision to every caller.
      return requestViewSnapshot();
    }
    const request = () => requestMutationSnapshot("/actions", {
      method: "POST",
      body: JSON.stringify(action),
      signal: navigationAction
        ? AbortSignal.timeout(HOSTED_CHAT_NAVIGATION_TIMEOUT_MS)
        : undefined,
    }, allowEqualRevision, navigationAction && !target ? (next) => {
      if (selectionIntentTokenRef.current !== token) return false;
      const selection = hostedChatSelectionFromState(next);
      localSelectionRef.current = selection;
      selectionIntentRef.current = { ...selection, token: token! };
      return true;
    } : undefined);

    if (!navigationAction) return request();

    // Pin the clicked selection immediately. The pin clears when a snapshot
    // confirms it (setMergedState); if the request fails, times out, or the
    // read fails, restore the last coherent view rather than combining a
    // different server selection with the current messages.
    if (target && token !== null) {
      selectionIntentRef.current = { ...target, token };
      localSelectionRef.current = target;
      setSelectionPending(true);
      setState((current) => {
        const selected = current ? { ...current, ...target } : current;
        stateRef.current = selected;
        return selected;
      });
    }

    const releaseIntent = (caught?: unknown) => {
      if (selectionIntentRef.current?.token !== token) return;
      selectionIntentRef.current = null;
      setSelectionPending(false);
      if (isHostedChatNavigationTimeout(caught)) {
        setTransportError(CHAT_NAVIGATION_TIMEOUT_MESSAGE);
      }
      const coherent = coherentStateRef.current;
      if (coherent) {
        messageLimitRef.current = Math.max(50, coherent.messages.length);
        setMessageLimit(messageLimitRef.current);
        setMergedState(coherent);
      }
    };

    // Send selection-changing actions in click order so delayed network
    // arrival cannot make the server persist an older intent as the final
    // selection. This is intentionally not a global mutation queue: messages,
    // typing, reads, and uploads still run independently of navigation.
    const pending = navigationMutationTailRef.current.then(request, request);
    navigationMutationTailRef.current = pending.then(
      () => releaseIntent(),
      releaseIntent
    );
    return pending;
  }, [requestMutationSnapshot, requestViewSnapshot, setMergedState]);

  const dispatch = useCallback((action: HostedChatAction) =>
    requestActionSnapshot(action), [requestActionSnapshot]);

  const dispatchQuiet = useCallback(async (action: HostedChatAction) => {
    try {
      return await requestActionSnapshot(action, false);
    } catch {
      return null;
    }
  }, [requestActionSnapshot]);

  const refreshPendingChat = useCallback(async (target: PendingChatRefreshTarget) => {
    const requestGeneration = snapshotSourceRef.current.generation;
    const selectionToken = selectionIntentTokenRef.current;
    try {
      const next = await hostedChatRequest<HostedChatState>(`${apiBase}/state${currentViewQuery()}`);
      const source = snapshotSourceRef.current;
      const current = stateRef.current;
      if (
        !current
        || !matchesView(next)
        || selectionIntentTokenRef.current !== selectionToken
        || source.generation !== requestGeneration
        || next.rev < source.highestRev
        || !pendingChatRefreshAdvancesTranscript(current, next, target)
      ) {
        return false;
      }
      snapshotSourceRef.current = recordHostedChatSnapshot(source, next.rev, false);
      snapshotSequenceRef.current += 1;
      setMergedState(next);
      setTransportError(null);
      return true;
    } catch {
      return false;
    }
  }, [apiBase, currentViewQuery, matchesView, setMergedState]);

  const uploadAttachments = useCallback((formData: FormData) =>
    requestMutationSnapshot("/attachments", {
      method: "POST",
      body: formData,
    }), [requestMutationSnapshot]);

  const attachmentUrl = useCallback((address: {
    room_id: string;
    message_id: string;
    attachment_id: string;
  }) =>
    `${apiBase}/attachments/${encodeURIComponent(address.room_id)}/${encodeURIComponent(address.message_id)}/${encodeURIComponent(address.attachment_id)}`,
  [apiBase]);

  useEffect(() => {
    if (hasState) return;
    const controller = new AbortController();
    void runInitialHostedChatRetries(
      () => load(false),
      controller.signal
    ).then((result) => {
      if (
        result === "stop" &&
        !controller.signal.aborted &&
        !sessionAuthFailureRef.current
      ) {
        setTransportError(lastLoadErrorRef.current ?? CHAT_UNAVAILABLE_MESSAGE);
      }
    });
    return () => controller.abort();
  }, [hasState, load]);

  useEffect(() => {
    if (!hasState || ownerClaimed) return;
    const controller = new AbortController();
    void runInitialHostedChatRetries(
      () => claimOwner(false),
      controller.signal
    ).then((result) => {
      if (
        result === "stop" &&
        !controller.signal.aborted &&
        !sessionAuthFailureRef.current
      ) {
        setClaimError(lastClaimErrorRef.current ?? CHAT_UNAVAILABLE_MESSAGE);
      }
    });
    return () => controller.abort();
  }, [claimOwner, hasState, ownerClaimed]);

  useEffect(() => {
    if (!hasState) return;

    let disposed = false;
    let events: EventSource | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

    const connect = () => {
      if (disposed) return;
      const stream = beginHostedChatStreamConnection(
        snapshotSourceRef.current,
        snapshotSequenceRef.current
      );
      snapshotSourceRef.current = stream.source;
      const nextEvents = new EventSource(`${apiBase}/updates${viewQuery}`);
      events = nextEvents;
      const onState = (event: MessageEvent<string>) => {
        try {
          const next = JSON.parse(event.data) as HostedChatState;
          if (disposed || events !== nextEvents || !matchesView(next)) return;
          let source = snapshotSourceRef.current;
          if (hostedChatStreamSnapshotProvesRestart(
            source,
            stream.connection,
            snapshotSequenceRef.current,
            next.rev
          )) {
            source = nextHostedChatSnapshotGeneration(source);
            snapshotSourceRef.current = source;
          }
          const snapshotAdvancedWhileBaselinePending = !source.hasStreamBaseline
            && snapshotSequenceRef.current > stream.connection.snapshotSequenceAtConnect;
          if (!shouldApplyStreamHostedChatSnapshot(
            source,
            next.rev,
            snapshotAdvancedWhileBaselinePending
          )) return;
          snapshotSourceRef.current = recordHostedChatSnapshot(source, next.rev, true);
          snapshotSequenceRef.current += 1;
          setMergedState(next);
          setTransportError(null);
          setStreamConnected(true);
        } catch {
          setTransportError(CHAT_INVALID_UPDATE_MESSAGE);
        }
      };
      nextEvents.addEventListener("open", () => setStreamConnected(true));
      nextEvents.addEventListener("state", onState as EventListener);
      nextEvents.addEventListener("error", () => {
        if (disposed || events !== nextEvents) return;
        nextEvents.close();
        events = null;
        setStreamConnected(false);
        // A 401 already observed on a chat request means the session is dead
        // and the page is heading to sign-in; reconnecting every second
        // against a dead session is pure noise.
        if (sessionAuthFailureRef.current) return;
        reconnectTimer = setTimeout(connect, STREAM_RECONNECT_DELAY_MS);
      });
    };

    connect();
    return () => {
      disposed = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      events?.close();
    };
  }, [apiBase, hasState, matchesView, setMergedState, viewQuery]);

  return (
    <HostedChatContext.Provider value={{
      apiBase,
      state,
      transportError,
      claimError,
      sessionError,
      streamConnected,
      ownerClaimed,
      bindingRecoveryRequired,
      selectionPending,
      load,
      claimOwner,
      recoverBinding,
      reportSessionAuthFailure,
      signInAgain,
      noteComposerInput,
      dispatch,
      dispatchQuiet,
      refreshPendingChat,
      uploadAttachments,
      attachmentUrl,
    }}>
      {children}
    </HostedChatContext.Provider>
  );
}

export function useHostedChat() {
  const context = useContext(HostedChatContext);
  if (!context) {
    throw new Error("useHostedChat must be used inside HostedChatProvider");
  }
  return context;
}

export function useOptionalHostedChat() {
  return useContext(HostedChatContext);
}

async function hostedChatRequest<T>(url: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  if (typeof init.body === "string") headers.set("content-type", "application/json");
  const response = await fetch(url, { ...init, cache: "no-store", headers });
  if (!response.ok) {
    const text = await response.text();
    try {
      const parsed = JSON.parse(text) as {
        error?: string;
        code?: string;
        retryable?: unknown;
      };
      throw new HostedChatHttpError(
        parsed.error || text || `Chat returned ${response.status}`,
        response.status,
        parsed.code,
        typeof parsed.retryable === "boolean" ? parsed.retryable : undefined
      );
    } catch (error) {
      if (error instanceof SyntaxError) {
        throw new HostedChatHttpError(text || `Chat returned ${response.status}`, response.status);
      }
      throw error;
    }
  }
  return response.json() as Promise<T>;
}

class HostedChatHttpError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly code?: string,
    readonly retryable?: boolean
  ) {
    super(message);
  }
}

export function hostedChatErrorMessage(error: unknown) {
  if (typeof error === "string") return error;
  return error instanceof Error ? error.message : CHAT_UNAVAILABLE_MESSAGE;
}

function isHostedChatNavigationAction(action: HostedChatAction) {
  return "OpenRoom" in action
    || "OpenTopic" in action
    || "OpenChat" in action
    || "CreateTopic" in action
    || "StartTopicChatIntent" in action;
}

/** A navigation request that outlives its AbortSignal.timeout rejects with a
 * DOMException named "TimeoutError". */
function isHostedChatNavigationTimeout(error: unknown) {
  return error instanceof Error && error.name === "TimeoutError";
}
