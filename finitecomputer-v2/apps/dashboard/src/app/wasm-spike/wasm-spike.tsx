"use client";

import { useCallback, useEffect, useRef, useState } from "react";

type Snapshot = {
  room: string;
  device: { account_id: string; device_id: string };
  afterSeq: number;
  epoch: number;
  messages: { id: string; sender: string; text: string }[];
};
type Chat = {
  connect: (agent: string, room: string) => Promise<void>;
  send: (text: string) => Promise<string>;
  sync: () => Promise<string>;
  snapshot: () => string;
  free: () => void;
};
type WasmModule = {
  default: (options: { module_or_path: string }) => Promise<void>;
  BrowserChat: new (nsec: string, server: string, device: string) => Chat;
};

// Native ESM import keeps wasm-bindgen's browser loader out of Next's server bundle.
// This fixed local URL contains generated build output, never user-provided code.
async function loadWasm(): Promise<WasmModule> {
  const moduleUrl = "/wasm-spike/finitechat_wasm.js";
  const bindings = await import(/* webpackIgnore: true */ /* turbopackIgnore: true */ moduleUrl) as WasmModule;
  await bindings.default({ module_or_path: "/wasm-spike/finitechat_wasm_bg.wasm" });
  return bindings;
}

export default function WasmSpike() {
  const client = useRef<Chat | null>(null);
  const pending = useRef(0);
  const queue = useRef(Promise.resolve());
  const serialize = useCallback(<T,>(action: () => Promise<T>): Promise<T> => {
    pending.current += 1;
    const result = queue.current.then(action);
    queue.current = result.then(() => { pending.current -= 1; }, () => { pending.current -= 1; });
    return result;
  }, []);
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [text, setText] = useState("");
  const [status, setStatus] = useState("Ready for a fresh local session");
  const [error, setError] = useState("");
  const [sending, setSending] = useState(false);
  const [connecting, setConnecting] = useState(false);

  useEffect(() => {
    let active = true;
    const timer = setInterval(async () => {
      if (!client.current || pending.current > 0) return;
      try {
        const next = JSON.parse(await serialize(() => client.current!.sync())) as Snapshot;
        if (active) setSnapshot(next);
      } catch (error) {
        if (active) setError(String(error));
      }
    }, 500);
    return () => { active = false; clearInterval(timer); };
  }, [serialize]);

  async function connect() {
    if (pending.current > 0 || client.current) return;
    setConnecting(true);
    setError("");
    try {
      setStatus("Loading Rust WebAssembly…");
      const wasm = await loadWasm();
      const response = await fetch("/api/wasm-spike/bootstrap", { method: "POST", cache: "no-store" });
      if (!response.ok) throw new Error(`Local login fixture returned ${response.status}`);
      const bootstrap = await response.json();
      const id = crypto.randomUUID();
      const chat = new wasm.BrowserChat(bootstrap.nsec, bootstrap.serverUrl, `browser_${id}`);
      bootstrap.nsec = "";
      setStatus("Creating an MLS Room and admitting the native agent…");
      await chat.connect(bootstrap.agentAccountId, `wasm_${id}`);
      client.current = chat;
      setSnapshot(JSON.parse(chat.snapshot()));
      setStatus("Connected · Rust/WASM → encrypted FiniteChat → native agent");
    } catch (error) { setError(String(error)); setStatus("Connection failed"); }
    finally { setConnecting(false); }
  }

  async function send(event: React.FormEvent) {
    event.preventDefault();
    if (!client.current || sending || !text.trim()) return;
    setSending(true);
    setError("");
    try {
      setSnapshot(JSON.parse(await serialize(() => client.current!.send(text))));
      setText("");
    } catch (error) { setError(String(error)); }
    finally { setSending(false); }
  }

  return <main className="mx-auto flex min-h-screen max-w-3xl flex-col gap-6 p-8">
    <header className="space-y-2">
      <p className="text-sm text-muted-foreground">LOCAL FEASIBILITY SPIKE</p>
      <h1 className="text-3xl font-semibold">FiniteChat in your browser</h1>
      <p className="text-muted-foreground">The browser encrypts and decrypts with the same Rust MLS client as the agent. No Hosted Web Device is running.</p>
      <p className="text-sm text-muted-foreground">Disposable login simulates Core returning an nsec. This peer replies deterministically; it is not an LLM. Reload starts a new Device and Room.</p>
    </header>
    <p role="status" className="rounded-lg border p-3">{status}</p>
    {!snapshot && <button className="rounded-lg bg-foreground px-4 py-3 text-background disabled:opacity-50" disabled={connecting} onClick={connect}>Sign in as local spike user</button>}
    {snapshot && <>
      <details className="rounded-lg border p-3 text-sm">
        <summary>Transport details · MLS epoch {snapshot.epoch} · log position {snapshot.afterSeq}</summary>
        <dl className="mt-3 break-all"><dt>Room</dt><dd>{snapshot.room}</dd><dt>Browser Device</dt><dd>{snapshot.device.device_id}</dd></dl>
      </details>
      <div aria-label="Messages" className="flex min-h-60 flex-1 flex-col gap-3">
        {snapshot.messages.length === 0 && <p className="text-muted-foreground">Send a message to test an encrypted round trip.</p>}
        {snapshot.messages.map(message => <article key={message.id} className={`max-w-[90%] rounded-xl border p-4 ${message.sender === "You" ? "self-end bg-muted" : "self-start"}`}>
          <p className="mb-1 text-xs text-muted-foreground">{message.sender === "You" ? "You" : "Native agent"}</p>
          <p className="whitespace-pre-wrap">{message.text}</p>
        </article>)}
      </div>
      <form onSubmit={send} className="flex gap-2">
        <input aria-label="Message" className="min-w-0 flex-1 rounded-lg border p-3" value={text} onChange={event => setText(event.target.value)} placeholder="Say something over MLS…" />
        <button className="rounded-lg bg-foreground px-5 text-background disabled:opacity-50" disabled={sending || !text.trim()}>{sending ? "Sending…" : "Send"}</button>
      </form>
    </>}
    {error && <p role="alert" className="rounded-lg border border-red-500 p-3 text-red-500">{error}</p>}
  </main>;
}
