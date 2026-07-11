import type { AppAction, AppState } from "../model";
import type { ChatProductUpload, ChatTransport } from "./transport";

export type ChatProductControllerState = {
  state: AppState | null;
  error: string | null;
  loading: boolean;
  streamConnected: boolean;
};

type Listener = (view: ChatProductControllerState) => void;

export class ChatProductController {
  private view: ChatProductControllerState = {
    state: null,
    error: null,
    loading: true,
    streamConnected: false,
  };
  private generation = 0;
  private acceptsResetBaseline = false;
  private unsubscribeTransport: (() => void) | null = null;
  private readonly listeners = new Set<Listener>();

  constructor(readonly transport: ChatTransport) {}

  snapshot() {
    return this.view;
  }

  listen(listener: Listener) {
    this.listeners.add(listener);
    listener(this.view);
    return () => {
      this.listeners.delete(listener);
    };
  }

  start() {
    this.unsubscribeTransport?.();
    this.unsubscribeTransport = this.transport.subscribe?.(
      (state) => this.applyState(state),
      (error) => this.setView({ error: errorMessage(error), streamConnected: false }),
      () => {
        this.generation += 1;
        this.acceptsResetBaseline = true;
        this.setView({ streamConnected: false });
      }
    ) ?? null;
    void this.refresh().catch(() => undefined);
  }

  stop() {
    this.unsubscribeTransport?.();
    this.unsubscribeTransport = null;
  }

  async refresh() {
    const requestGeneration = this.generation;
    this.setView({ loading: this.view.state === null });
    try {
      const state = await this.transport.load();
      if (requestGeneration === this.generation) this.applyState(state);
      return state;
    } catch (error) {
      this.setView({ error: errorMessage(error), loading: false });
      throw error;
    }
  }

  async dispatch(action: AppAction) {
    const requestGeneration = this.generation;
    const state = await this.transport.dispatch(action);
    if (requestGeneration === this.generation) this.applyState(state);
    return state;
  }

  async upload(upload: ChatProductUpload) {
    if (!this.transport.upload) {
      throw new Error("File attachments are not available on this Device.");
    }
    const requestGeneration = this.generation;
    const state = await this.transport.upload(upload);
    if (requestGeneration === this.generation) this.applyState(state);
    return state;
  }

  private applyState(state: AppState) {
    const current = this.view.state;
    if (!this.acceptsResetBaseline && current && state.rev < current.rev) return;
    this.acceptsResetBaseline = false;
    this.setView({
      state,
      error: null,
      loading: false,
      streamConnected: this.transport.subscribe ? true : this.view.streamConnected,
    });
  }

  private setView(patch: Partial<ChatProductControllerState>) {
    this.view = { ...this.view, ...patch };
    for (const listener of this.listeners) listener(this.view);
  }
}

export function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : "Chat is unavailable right now.";
}
