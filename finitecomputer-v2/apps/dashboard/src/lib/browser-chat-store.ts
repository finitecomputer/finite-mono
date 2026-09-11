// The sole browser checkpoint store. The nsec and derived key stay in memory.
// A Web Lock spans reload of the latest state, all MLS operations, and durable writes.
export type BrowserCheckpoint = { deviceId: string; wasm: string; topic: string; chat: string };
type SealedCheckpoint = { id: string; revision: number; iv: Uint8Array<ArrayBuffer>; ciphertext: ArrayBuffer };
const bytes = (text: string) => new TextEncoder().encode(text);

export class BrowserChatStore {
  private constructor(readonly scope: string, private db: IDBDatabase, private key: CryptoKey) {}

  static async open(nsec: string, server: string, agent: string) {
    if (!navigator.locks || !globalThis.indexedDB) {
      throw new Error("This browser needs IndexedDB and Web Locks to use a persistent FiniteChat Device.");
    }
    const fingerprint = await crypto.subtle.digest("SHA-256", bytes(nsec));
    const scope = JSON.stringify(["finitechat-browser-v1", server, agent, Array.from(new Uint8Array(fingerprint))]);
    const material = await crypto.subtle.importKey("raw", bytes(nsec), "HKDF", false, ["deriveKey"]);
    const key = await crypto.subtle.deriveKey({ name: "HKDF", hash: "SHA-256", salt: bytes(scope), info: bytes("sealed-checkpoint-v1") },
      material, { name: "AES-GCM", length: 256 }, false, ["encrypt", "decrypt"]);
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const request = indexedDB.open("finitechat-wasm", 1);
      request.onupgradeneeded = () => request.result.createObjectStore("devices", { keyPath: "id" });
      request.onerror = () => reject(request.error);
      request.onblocked = () => reject(new Error("Close older FiniteChat tabs to open browser storage."));
      request.onsuccess = () => resolve(request.result);
    });
    db.onversionchange = () => db.close();
    // A best-effort request to reduce storage eviction; denial doesn't mean durability is absent.
    void navigator.storage?.persist?.().catch(() => false);
    return new BrowserChatStore(scope, db, key);
  }

  async exclusive<T>(operation: () => Promise<T>): Promise<T> {
    return await navigator.locks.request(`finitechat:${this.scope}`, { mode: "exclusive" }, operation);
  }

  read(): Promise<SealedCheckpoint | undefined> {
    return new Promise((resolve, reject) => {
      const tx = this.db.transaction("devices", "readonly");
      const request = tx.objectStore("devices").get(this.scope);
      tx.oncomplete = () => resolve(request.result);
      tx.onabort = () => reject(tx.error ?? new Error("Browser storage read aborted"));
      tx.onerror = () => reject(tx.error);
    });
  }

  async openCheckpoint(record: SealedCheckpoint): Promise<BrowserCheckpoint> {
    try {
      if (!Number.isSafeInteger(record.revision) || record.revision < 1) throw new Error("Invalid checkpoint revision");
      const plaintext = await crypto.subtle.decrypt({ name: "AES-GCM", iv: record.iv,
        additionalData: bytes(this.scope) }, this.key, record.ciphertext);
      const saved = JSON.parse(new TextDecoder().decode(plaintext)) as BrowserCheckpoint;
      if (!saved.deviceId || !saved.wasm || typeof saved.topic !== "string" || typeof saved.chat !== "string") throw new Error("Invalid checkpoint");
      return saved;
    } catch {
      throw new Error("Stored FiniteChat state could not be opened. Refusing to replace this Device or overwrite its history.");
    }
  }

  async write(saved: BrowserCheckpoint, expectedRevision: number): Promise<number> {
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const ciphertext = await crypto.subtle.encrypt({ name: "AES-GCM", iv, additionalData: bytes(this.scope) },
      this.key, bytes(JSON.stringify(saved)));
    const revision = expectedRevision + 1;
    await new Promise<void>((resolve, reject) => {
      // Complete, not request.success, is the durability boundary. Compare-and-swap
      // also catches an unexpected writer that bypassed the Web Lock.
      const tx = this.db.transaction("devices", "readwrite", { durability: "strict" });
      const store = tx.objectStore("devices");
      const current = store.get(this.scope);
      current.onsuccess = () => {
        if ((current.result?.revision ?? 0) !== expectedRevision) { tx.abort(); return; }
        try { store.put({ id: this.scope, revision, iv, ciphertext } satisfies SealedCheckpoint); }
        catch { try { tx.abort(); } catch { /* The failing write may already have aborted it. */ } }
      };
      tx.oncomplete = () => resolve();
      tx.onabort = () => reject(new Error("Checkpoint write failed or another writer changed the Device. Sending is paused until its durable state can be reopened."));
      tx.onerror = () => reject(tx.error);
    });
    return revision;
  }
}
