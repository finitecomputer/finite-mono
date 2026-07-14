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
  runInitialHostedChatRetries,
  shouldRetryHostedChatRequest,
  type HostedChatRetryAttempt,
} from "@/lib/hosted-web-chat-retry";

const STREAM_RECONNECT_DELAY_MS = 1_000;

type MutationSnapshotRequest = {
  allowEqualRevision: boolean;
  generation: number;
  highestRev: number;
  sequence: number;
};

type HostedChatContextValue = {
  apiBase: string;
  state: HostedChatState | null;
  transportError: string | null;
  claimError: string | null;
  streamConnected: boolean;
  ownerClaimed: boolean;
  bindingRecoveryRequired: boolean;
  load: (showError?: boolean) => Promise<HostedChatRetryAttempt>;
  claimOwner: () => Promise<HostedChatRetryAttempt>;
  recoverBinding: () => Promise<HostedChatRetryAttempt>;
  dispatch: (action: HostedChatAction) => Promise<HostedChatState>;
  dispatchQuiet: (action: HostedChatAction) => Promise<HostedChatState | null>;
  uploadAttachments: (formData: FormData) => Promise<HostedChatState>;
};

const HostedChatContext = createContext<HostedChatContextValue | null>(null);

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
  const [streamConnected, setStreamConnected] = useState(false);
  const [ownerClaimed, setOwnerClaimed] = useState(false);
  const [bindingRecoveryRequired, setBindingRecoveryRequired] = useState(false);
  const snapshotSourceRef = useRef(initialHostedChatSnapshotSource());
  const stateLoadRef = useRef<Promise<HostedChatRetryAttempt> | null>(null);
  const lastLoadErrorRef = useRef<string | null>(null);
  const ownerClaimRef = useRef<Promise<HostedChatRetryAttempt> | null>(null);
  const navigationMutationTailRef = useRef<Promise<void>>(Promise.resolve());
  const nextMutationSequenceRef = useRef(0);
  const latestAppliedMutationSequenceRef = useRef(0);
  const snapshotSequenceRef = useRef(0);
  const hasState = state !== null;

  const setMergedState = useCallback((next: HostedChatState) => {
    setState((current) => ({
      ...next,
      hosted_agent_binding: next.hosted_agent_binding === undefined
        ? current?.hosted_agent_binding ?? null
        : next.hosted_agent_binding,
    }));
  }, []);

  const applyHttpSnapshot = useCallback((next: HostedChatState, requestGeneration: number) => {
    const source = snapshotSourceRef.current;
    if (!shouldApplyHttpHostedChatSnapshot(source, requestGeneration, next.rev)) {
      return false;
    }
    snapshotSourceRef.current = recordHostedChatSnapshot(source, next.rev, false);
    snapshotSequenceRef.current += 1;
    setMergedState(next);
    return true;
  }, [setMergedState]);

  // A mutation response is authoritative even when the daemon did not advance
  // its revision for a selection-only action. Requests run concurrently so
  // navigation cannot wait behind typing/read receipts or uploads; the client
  // sequence prevents an older equal-revision response from rolling back a
  // newer mutation response.
  const applyMutationSnapshot = useCallback((
    next: HostedChatState,
    request: MutationSnapshotRequest
  ) => {
    const source = snapshotSourceRef.current;
    if (!shouldApplyMutationHostedChatSnapshot(
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
  }, [setMergedState]);

  const load = useCallback((showError = true) => {
    if (stateLoadRef.current) return stateLoadRef.current;
    const requestGeneration = snapshotSourceRef.current.generation;
    const pending = (async (): Promise<HostedChatRetryAttempt> => {
      try {
        const next = await hostedChatRequest<HostedChatState>(`${apiBase}/state`);
        applyHttpSnapshot(next, requestGeneration);
        setTransportError(null);
        setBindingRecoveryRequired(false);
        return "succeeded";
      } catch (caught) {
        const message = hostedChatErrorMessage(caught);
        setBindingRecoveryRequired(
          caught instanceof HostedChatHttpError &&
          caught.code === "binding_authorization_required"
        );
        lastLoadErrorRef.current = message;
        if (showError) setTransportError(message);
        const status = caught instanceof HostedChatHttpError ? caught.status : null;
        return shouldRetryHostedChatRequest(status) ? "retry" : "stop";
      }
    })();
    stateLoadRef.current = pending;
    void pending.finally(() => {
      if (stateLoadRef.current === pending) stateLoadRef.current = null;
    });
    return pending;
  }, [apiBase, applyHttpSnapshot]);

  const claimOwner = useCallback(() => {
    if (ownerClaimRef.current) return ownerClaimRef.current;
    const pending = (async (): Promise<HostedChatRetryAttempt> => {
      try {
        await hostedChatRequest<{ claimed: true }>(`${apiBase}/claim`, { method: "POST" });
        setOwnerClaimed(true);
        setClaimError(null);
        return "succeeded";
      } catch (caught) {
        setClaimError(hostedChatErrorMessage(caught));
        const status = caught instanceof HostedChatHttpError ? caught.status : null;
        return shouldRetryHostedChatRequest(status) ? "retry" : "stop";
      }
    })();
    ownerClaimRef.current = pending;
    void pending.finally(() => {
      if (ownerClaimRef.current === pending) ownerClaimRef.current = null;
    });
    return pending;
  }, [apiBase]);

  const requestMutationSnapshot = useCallback(async (
    path: string,
    init: RequestInit,
    allowEqualRevision = true,
    reconcileRejectedSnapshot = false
  ) => {
    const captureRequest = (): MutationSnapshotRequest => {
      const source = snapshotSourceRef.current;
      return {
        generation: source.generation,
        highestRev: source.highestRev,
        sequence: ++nextMutationSequenceRef.current,
        allowEqualRevision,
      };
    };
    const request = captureRequest();
    const next = await hostedChatRequest<HostedChatState>(`${apiBase}${path}`, init);
    const applied = applyMutationSnapshot(next, request);
    if (applied || !reconcileRejectedSnapshot) return next;

    // A selection-only action can return an older revision after a concurrent
    // stream event advanced the client. Refetch after the server applied the
    // selection so an equal-revision full snapshot can reconcile it.
    const reconciliationRequest = captureRequest();
    const reconciled = await hostedChatRequest<HostedChatState>(`${apiBase}/state`);
    applyMutationSnapshot(reconciled, reconciliationRequest);
    return reconciled;
  }, [apiBase, applyMutationSnapshot]);

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
      setTransportError(hostedChatErrorMessage(caught));
      return "stop";
    }
  }, [requestMutationSnapshot]);

  const requestActionSnapshot = useCallback((
    action: HostedChatAction,
    allowEqualRevision = true
  ) => {
    const navigationAction = isHostedChatNavigationAction(action);
    const request = () => requestMutationSnapshot("/actions", {
      method: "POST",
      body: JSON.stringify(action),
    }, allowEqualRevision, navigationAction);

    if (!navigationAction) return request();

    // Send selection-changing actions in click order so delayed network
    // arrival cannot make the server persist an older intent as the final
    // selection. This is intentionally not a global mutation queue: messages,
    // typing, reads, and uploads still run independently of navigation.
    const pending = navigationMutationTailRef.current.then(request, request);
    navigationMutationTailRef.current = pending.then(
      () => undefined,
      () => undefined
    );
    return pending;
  }, [requestMutationSnapshot]);

  const dispatch = useCallback((action: HostedChatAction) =>
    requestActionSnapshot(action), [requestActionSnapshot]);

  const dispatchQuiet = useCallback(async (action: HostedChatAction) => {
    try {
      return await requestActionSnapshot(action, false);
    } catch {
      return null;
    }
  }, [requestActionSnapshot]);

  const uploadAttachments = useCallback((formData: FormData) =>
    requestMutationSnapshot("/attachments", {
      method: "POST",
      body: formData,
    }), [requestMutationSnapshot]);

  useEffect(() => {
    if (hasState) return;
    const controller = new AbortController();
    void runInitialHostedChatRetries(() => load(false), controller.signal).then((result) => {
      if (result === "stop" && !controller.signal.aborted) {
        setTransportError(lastLoadErrorRef.current ?? CHAT_UNAVAILABLE_MESSAGE);
      }
    });
    return () => controller.abort();
  }, [hasState, load]);

  useEffect(() => {
    if (!hasState || ownerClaimed) return;
    const controller = new AbortController();
    void runInitialHostedChatRetries(claimOwner, controller.signal);
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
      const nextEvents = new EventSource(`${apiBase}/updates`);
      events = nextEvents;
      const onState = (event: MessageEvent<string>) => {
        try {
          const next = JSON.parse(event.data) as HostedChatState;
          if (events !== nextEvents) return;
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
        setTransportError((current) => current ?? "Reconnecting…");
        reconnectTimer = setTimeout(connect, STREAM_RECONNECT_DELAY_MS);
      });
    };

    connect();
    return () => {
      disposed = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      events?.close();
    };
  }, [apiBase, hasState, setMergedState]);

  return (
    <HostedChatContext.Provider value={{
      apiBase,
      state,
      transportError,
      claimError,
      streamConnected,
      ownerClaimed,
      bindingRecoveryRequired,
      load,
      claimOwner,
      recoverBinding,
      dispatch,
      dispatchQuiet,
      uploadAttachments,
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

async function hostedChatRequest<T>(url: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  if (typeof init.body === "string") headers.set("content-type", "application/json");
  const response = await fetch(url, { ...init, cache: "no-store", headers });
  if (!response.ok) {
    const text = await response.text();
    try {
      const parsed = JSON.parse(text) as { error?: string; code?: string };
      throw new HostedChatHttpError(
        parsed.error || text || `Chat returned ${response.status}`,
        response.status,
        parsed.code
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
    readonly code?: string
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
    || "StartTopicChatIntent" in action;
}
