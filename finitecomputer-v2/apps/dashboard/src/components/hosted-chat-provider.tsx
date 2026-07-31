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
import {
  ElectronChatStateError,
  electronAttachmentUpload,
  electronChatRuntime,
  isElectronLocalDeviceRecoveryRequired,
  mergeElectronChatState,
  reconcileElectronChatState,
  type ElectronAttachmentAddress,
  type ElectronChatRuntime,
  type ElectronDeviceLinkStatus,
  type ElectronLocalDevice,
} from "@/lib/electron-chat-runtime";
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
import {
  hostedChatSelectionFromState,
  hostedChatSelectionIntentTarget,
  projectHostedChatVisibleSelection,
  type HostedChatSelection,
} from "@/lib/hosted-web-chat-selection";
import {
  pendingChatRefreshAdvancesTranscript,
  type PendingChatRefreshTarget,
} from "@/lib/hosted-web-chat-refresh";
import {
  recordHostedChatNavigation,
  type HostedChatSnapshotSource,
} from "@/lib/hosted-web-chat-journal";

const STREAM_RECONNECT_DELAY_MS = 1_000;
const REVOKED_DESKTOP_MESSAGE =
  "This desktop Device was revoked. Relink this Mac to create a fresh Device. Your existing encrypted local store will be kept as a backup.";

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
  localDeviceRecoveryRequired: boolean;
  deviceLinkStatus: ElectronDeviceLinkStatus | null;
  selectionPending: boolean;
  load: (showError?: boolean) => Promise<HostedChatRetryAttempt>;
  claimOwner: (showError?: boolean) => Promise<HostedChatRetryAttempt>;
  recoverBinding: () => Promise<HostedChatRetryAttempt>;
  recoverLocalDevice: () => Promise<HostedChatRetryAttempt>;
  dispatch: (action: HostedChatAction) => Promise<HostedChatState>;
  dispatchQuiet: (action: HostedChatAction) => Promise<HostedChatState | null>;
  refreshPendingChat: (target: PendingChatRefreshTarget) => Promise<boolean>;
  uploadAttachments: (formData: FormData) => Promise<HostedChatState>;
  attachmentUrl: (address: ElectronAttachmentAddress) => string;
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
  const runtime = electronChatRuntime();
  const [state, setState] = useState<HostedChatState | null>(null);
  const [transportError, setTransportError] = useState<string | null>(null);
  const [claimError, setClaimError] = useState<string | null>(null);
  const [streamConnected, setStreamConnected] = useState(false);
  const [ownerClaimed, setOwnerClaimed] = useState(false);
  const [bindingRecoveryRequired, setBindingRecoveryRequired] = useState(false);
  const [localDeviceRecoveryRequired, setLocalDeviceRecoveryRequired] = useState(false);
  const [deviceLinkStatus, setDeviceLinkStatus] =
    useState<ElectronDeviceLinkStatus | null>(runtime ? { status: "preparing" } : null);
  const [selectionPending, setSelectionPending] = useState(false);
  const stateRef = useRef<HostedChatState | null>(null);
  const snapshotSourceRef = useRef(initialHostedChatSnapshotSource());
  const stateLoadRef = useRef<Promise<HostedChatRetryAttempt> | null>(null);
  const lastLoadErrorRef = useRef<string | null>(null);
  const ownerClaimRef = useRef<Promise<HostedChatRetryAttempt> | null>(null);
  const lastClaimErrorRef = useRef<string | null>(null);
  const navigationMutationTailRef = useRef<Promise<void>>(Promise.resolve());
  const nextMutationSequenceRef = useRef(0);
  const latestAppliedMutationSequenceRef = useRef(0);
  const snapshotSequenceRef = useRef(0);
  const selectionIntentTokenRef = useRef(0);
  const visibleSelectionRef = useRef<HostedChatSelection | null>(null);
  const serverSelectionRef = useRef<HostedChatSelection | null>(null);
  const hostedAuthorityRef = useRef<HostedChatState | null>(null);
  const localDeviceRef = useRef<ElectronLocalDevice | null>(null);
  const hasState = state !== null;

  // Every applied snapshot funnels through here. The browser-visible route is
  // durable client navigation state: daemon snapshots update transcripts and
  // activity but cannot move it. Daemon selection initializes a new browser
  // and becomes a fallback only if the visible route disappears.
  const setMergedState = useCallback((
    next: HostedChatState,
    source: HostedChatSnapshotSource
  ) => {
    const snapshotSelection = hostedChatSelectionFromState(next);
    serverSelectionRef.current = snapshotSelection;
    const projected = projectHostedChatVisibleSelection(
      visibleSelectionRef.current,
      next
    );
    visibleSelectionRef.current = projected.selection;
    recordHostedChatNavigation({
      source,
      snapshot_sequence: snapshotSequenceRef.current,
      snapshot_rev: next.rev,
      snapshot_selection: snapshotSelection,
      visible_selection: projected.selection,
      navigation_intent_generation: selectionIntentTokenRef.current,
      decision: projected.decision,
    });
    setState((current) => {
      const merged = {
        ...projected.state,
        hosted_agent_binding: projected.state.hosted_agent_binding === undefined
          ? current?.hosted_agent_binding ?? null
          : projected.state.hosted_agent_binding,
      };
      stateRef.current = merged;
      return merged;
    });
  }, []);

  const mergeLocalState = useCallback((next: HostedChatState) => {
    const hosted = hostedAuthorityRef.current;
    const device = localDeviceRef.current;
    if (!hosted || !device) {
      throw new Error("This Device's chat account has not been verified.");
    }
    return mergeElectronChatState(next, hosted, device);
  }, []);

  const applyHttpSnapshot = useCallback((next: HostedChatState, requestGeneration: number) => {
    const source = snapshotSourceRef.current;
    if (!shouldApplyHttpHostedChatSnapshot(source, requestGeneration, next.rev)) {
      return false;
    }
    snapshotSourceRef.current = recordHostedChatSnapshot(source, next.rev, false);
    snapshotSequenceRef.current += 1;
    setMergedState(next, "http");
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
    setMergedState(next, "mutation");
    return true;
  }, [setMergedState]);

  const load = useCallback((showError = true, signal?: AbortSignal) => {
    if (stateLoadRef.current) return stateLoadRef.current;
    const requestGeneration = snapshotSourceRef.current.generation;
    const pending = (async (): Promise<HostedChatRetryAttempt> => {
      try {
        let next: HostedChatState;
        if (runtime) {
          setDeviceLinkStatus({ status: "preparing" });
          const deviceResult = await runtime.ensureLocalDevice();
          if (isElectronLocalDeviceRecoveryRequired(deviceResult)) {
            setLocalDeviceRecoveryRequired(true);
            lastLoadErrorRef.current = REVOKED_DESKTOP_MESSAGE;
            if (showError) setTransportError(REVOKED_DESKTOP_MESSAGE);
            return "stop";
          }
          const device = deviceResult;
          const hosted = await hostedChatRequest<HostedChatState>(
            `${apiBase}/state`,
            signal ? { signal } : undefined
          );
          localDeviceRef.current = device;
          hostedAuthorityRef.current = hosted;
          next = await reconcileElectronChatState(
            runtime,
            hosted,
            device,
            (targetDeviceId) => hostedChatRequest(`${apiBase}/reconcile-device`, {
              method: "POST",
              body: JSON.stringify({ target_device_id: targetDeviceId }),
              signal,
            }),
            { signal }
          );
        } else {
          // Preserve the existing browser load semantics. In development,
          // React may tear down and immediately remount this effect while the
          // shared request is still in flight; aborting that request leaves
          // the remount holding the same cancelled promise. Electron needs
          // cancellation for its bounded native reconciliation, but the
          // hosted browser path does not.
          next = await hostedChatRequest<HostedChatState>(`${apiBase}/state`);
        }
        if (runtime && signal?.aborted) return "stop";
        applyHttpSnapshot(next, requestGeneration);
        setTransportError(null);
        setBindingRecoveryRequired(false);
        setLocalDeviceRecoveryRequired(false);
        if (runtime) setDeviceLinkStatus({ status: "ready" });
        return "succeeded";
      } catch (caught) {
        if (runtime && signal?.aborted) return "stop";
        const message = hostedChatErrorMessage(caught);
        setBindingRecoveryRequired(
          caught instanceof HostedChatHttpError &&
          caught.code === "binding_authorization_required"
        );
        lastLoadErrorRef.current = message;
        if (showError) setTransportError(message);
        if (caught instanceof ElectronChatStateError) return "stop";
        const status = caught instanceof HostedChatHttpError ? caught.status : null;
        return shouldRetryHostedChatRequest(status) ? "retry" : "stop";
      }
    })();
    stateLoadRef.current = pending;
    void pending.finally(() => {
      if (stateLoadRef.current === pending) stateLoadRef.current = null;
    });
    return pending;
  }, [apiBase, applyHttpSnapshot, runtime]);

  const claimOwner = useCallback((showError = true) => {
    if (ownerClaimRef.current) return ownerClaimRef.current;
    const pending = (async (): Promise<HostedChatRetryAttempt> => {
      try {
        await hostedChatRequest<{ claimed: true }>(`${apiBase}/claim`, { method: "POST" });
        setOwnerClaimed(true);
        setClaimError(null);
        return "succeeded";
      } catch (caught) {
        const message = hostedChatErrorMessage(caught);
        lastClaimErrorRef.current = message;
        if (showError) setClaimError(message);
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

  const requestElectronMutationSnapshot = useCallback(async (
    operation: (runtime: ElectronChatRuntime) => Promise<HostedChatState>,
    allowEqualRevision = true,
    reconcileRejectedSnapshot = false
  ) => {
    if (!runtime) throw new Error("The local chat runtime is unavailable.");
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
    const next = mergeLocalState(await operation(runtime));
    const applied = applyMutationSnapshot(next, request);
    if (applied || !reconcileRejectedSnapshot) return next;

    const reconciliationRequest = captureRequest();
    const reconciled = mergeLocalState(await runtime.daemonState());
    applyMutationSnapshot(reconciled, reconciliationRequest);
    return reconciled;
  }, [applyMutationSnapshot, mergeLocalState, runtime]);

  const recoverBinding = useCallback(async (): Promise<HostedChatRetryAttempt> => {
    try {
      let next: HostedChatState;
      if (runtime) {
        const device = await runtime.ensureLocalDevice();
        if (isElectronLocalDeviceRecoveryRequired(device)) {
          setLocalDeviceRecoveryRequired(true);
          setTransportError(REVOKED_DESKTOP_MESSAGE);
          return "stop";
        }
        const hosted = await hostedChatRequest<HostedChatState>(`${apiBase}/recover-binding`, {
          method: "POST",
        });
        hostedAuthorityRef.current = hosted;
        localDeviceRef.current = device;
        next = await requestElectronMutationSnapshot(
          () => reconcileElectronChatState(
            runtime,
            hosted,
            device,
            (targetDeviceId) => hostedChatRequest(`${apiBase}/reconcile-device`, {
              method: "POST",
              body: JSON.stringify({ target_device_id: targetDeviceId }),
            })
          )
        );
      } else {
        next = await requestMutationSnapshot("/recover-binding", { method: "POST" });
      }
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
  }, [apiBase, requestElectronMutationSnapshot, requestMutationSnapshot, runtime]);

  const recoverLocalDevice = useCallback(async (): Promise<HostedChatRetryAttempt> => {
    if (!runtime || !("recoverLocalDevice" in runtime)) {
      setTransportError(CHAT_UNAVAILABLE_MESSAGE);
      return "stop";
    }
    setLocalDeviceRecoveryRequired(false);
    setTransportError(null);
    try {
      const device = await runtime.recoverLocalDevice();
      if (isElectronLocalDeviceRecoveryRequired(device)) {
        setLocalDeviceRecoveryRequired(true);
        setTransportError(REVOKED_DESKTOP_MESSAGE);
        return "stop";
      }
      localDeviceRef.current = device;
      return load(true);
    } catch (caught) {
      setTransportError(hostedChatErrorMessage(caught));
      return "stop";
    }
  }, [load, runtime]);

  const requestActionSnapshot = useCallback((
    action: HostedChatAction,
    allowEqualRevision = true
  ) => {
    const navigationAction = isHostedChatNavigationAction(action);
    const request = () => runtime
      ? requestElectronMutationSnapshot(
        (bridge) => bridge.dispatchDaemonAction(action),
        allowEqualRevision,
        navigationAction
      )
      : requestMutationSnapshot("/actions", {
        method: "POST",
        body: JSON.stringify(action),
      }, allowEqualRevision, navigationAction);

    if (!navigationAction) return request();

    // Explicit navigation immediately owns the visible route. The route stays
    // browser-owned after confirmation; later send/SSE/refresh snapshots
    // cannot reinterpret daemon persistence as a new navigation command.
    const target = hostedChatSelectionIntentTarget(action);
    const token = ++selectionIntentTokenRef.current;
    setSelectionPending(true);
    if (target) {
      visibleSelectionRef.current = target;
      if (stateRef.current) {
        recordHostedChatNavigation({
          source: "navigation",
          snapshot_sequence: snapshotSequenceRef.current,
          snapshot_rev: stateRef.current.rev,
          snapshot_selection:
            serverSelectionRef.current ?? hostedChatSelectionFromState(stateRef.current),
          visible_selection: target,
          navigation_intent_generation: token,
          decision: "navigation",
        });
      }
      setState((current) => {
        const selected = current ? { ...current, ...target } : current;
        stateRef.current = selected;
        return selected;
      });
    }

    const finishNavigation = (next: HostedChatState) => {
      if (selectionIntentTokenRef.current !== token) return;
      setSelectionPending(false);
      const explicitSelection = hostedChatSelectionFromState(next);
      visibleSelectionRef.current = explicitSelection;
      setState((current) => {
        if (!current) return current;
        const projected = projectHostedChatVisibleSelection(explicitSelection, current);
        visibleSelectionRef.current = projected.selection;
        stateRef.current = projected.state;
        return projected.state;
      });
    };
    const failNavigation = () => {
      if (selectionIntentTokenRef.current !== token) return;
      setSelectionPending(false);
      const fallback = serverSelectionRef.current;
      if (!fallback) return;
      visibleSelectionRef.current = fallback;
      setState((current) => {
        if (!current) return current;
        const projected = projectHostedChatVisibleSelection(fallback, current);
        visibleSelectionRef.current = projected.selection;
        stateRef.current = projected.state;
        return projected.state;
      });
    };

    // Send selection-changing actions in click order so delayed network
    // arrival cannot make the server persist an older intent as the final
    // selection. This is intentionally not a global mutation queue: messages,
    // typing, reads, and uploads still run independently of navigation.
    const pending = navigationMutationTailRef.current.then(request, request);
    void pending.then(finishNavigation, failNavigation);
    navigationMutationTailRef.current = pending.then(
      () => undefined,
      () => undefined
    );
    return pending;
  }, [requestElectronMutationSnapshot, requestMutationSnapshot, runtime]);

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
    if (runtime) return false;
    const requestGeneration = snapshotSourceRef.current.generation;
    const selectionToken = selectionIntentTokenRef.current;
    try {
      const next = await hostedChatRequest<HostedChatState>(`${apiBase}/state`);
      const source = snapshotSourceRef.current;
      const current = stateRef.current;
      if (
        !current
        || selectionIntentTokenRef.current !== selectionToken
        || source.generation !== requestGeneration
        || next.rev < source.highestRev
        || !pendingChatRefreshAdvancesTranscript(current, next, target)
      ) {
        return false;
      }
      snapshotSourceRef.current = recordHostedChatSnapshot(source, next.rev, false);
      snapshotSequenceRef.current += 1;
      setMergedState(next, "pending_refresh");
      setTransportError(null);
      return true;
    } catch {
      return false;
    }
  }, [apiBase, runtime, setMergedState]);

  const uploadAttachments = useCallback((formData: FormData) => runtime
    ? requestElectronMutationSnapshot(async (bridge) =>
      bridge.uploadDaemonAttachments(await electronAttachmentUpload(formData)))
    : requestMutationSnapshot("/attachments", {
      method: "POST",
      body: formData,
    }), [requestElectronMutationSnapshot, requestMutationSnapshot, runtime]);

  const attachmentUrl = useCallback((address: ElectronAttachmentAddress) => runtime
    ? runtime.attachmentUrl(address)
    : `${apiBase}/attachments/${encodeURIComponent(address.room_id)}/${encodeURIComponent(address.message_id)}/${encodeURIComponent(address.attachment_id)}`,
  [apiBase, runtime]);

  useEffect(() => {
    if (hasState) return;
    const controller = new AbortController();
    void runInitialHostedChatRetries(
      () => load(false, controller.signal),
      controller.signal
    ).then((result) => {
      if (result === "stop" && !controller.signal.aborted) {
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
      if (result === "stop" && !controller.signal.aborted) {
        setClaimError(lastClaimErrorRef.current ?? CHAT_UNAVAILABLE_MESSAGE);
      }
    });
    return () => controller.abort();
  }, [claimOwner, hasState, ownerClaimed]);

  useEffect(() => {
    if (!hasState) return;

    if (runtime) {
      let disposed = false;
      let lastGeneration: number | null = null;
      const unsubscribeState = runtime.onDaemonUpdate((raw) => {
        if (disposed) return;
        try {
          const next = mergeLocalState(raw);
          const source = snapshotSourceRef.current;
          if (!shouldApplyStreamHostedChatSnapshot(source, next.rev)) return;
          snapshotSourceRef.current = recordHostedChatSnapshot(source, next.rev, true);
          snapshotSequenceRef.current += 1;
          setMergedState(next, "electron_stream");
          setTransportError(null);
          setStreamConnected(true);
        } catch (caught) {
          setStreamConnected(false);
          setTransportError(hostedChatErrorMessage(caught));
        }
      });
      const unsubscribeError = runtime.onDaemonError((message) => {
        if (disposed) return;
        setStreamConnected(false);
        setTransportError(message || CHAT_UNAVAILABLE_MESSAGE);
      });
      const unsubscribeLinkStatus = runtime.onDeviceLinkStatus((status) => {
        if (disposed) return;
        setDeviceLinkStatus(status);
        if (status.status === "failed") {
          setStreamConnected(false);
          setTransportError(status.message || CHAT_UNAVAILABLE_MESSAGE);
        }
      });
      // Register generation last. Main replays generation before the first
      // state, allowing a restarted daemon to reset revision ordering.
      const unsubscribeGeneration = runtime.onDaemonGeneration(({ generation }) => {
        if (disposed || generation === lastGeneration) return;
        lastGeneration = generation;
        snapshotSourceRef.current = nextHostedChatSnapshotGeneration(
          snapshotSourceRef.current
        );
        setStreamConnected(false);
      });

      return () => {
        disposed = true;
        unsubscribeState();
        unsubscribeError();
        unsubscribeLinkStatus();
        unsubscribeGeneration();
      };
    }

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
          setMergedState(next, "sse");
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
        reconnectTimer = setTimeout(connect, STREAM_RECONNECT_DELAY_MS);
      });
    };

    connect();
    return () => {
      disposed = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      events?.close();
    };
  }, [apiBase, hasState, mergeLocalState, runtime, setMergedState]);

  return (
    <HostedChatContext.Provider value={{
      apiBase,
      state,
      transportError,
      claimError,
      streamConnected,
      ownerClaimed,
      bindingRecoveryRequired,
      localDeviceRecoveryRequired,
      deviceLinkStatus,
      selectionPending,
      load,
      claimOwner,
      recoverBinding,
      recoverLocalDevice,
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
    || "CreateTopic" in action
    || "StartTopicChatIntent" in action;
}
