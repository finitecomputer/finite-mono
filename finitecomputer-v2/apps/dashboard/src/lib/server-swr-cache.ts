type ServerSwrEntry<T> = {
  hasValue: boolean;
  value?: T;
  updatedAtMs: number;
  inFlight?: Promise<T>;
};

type ServerSwrOptions = {
  freshMs: number;
  staleMs: number;
  nowMs?: number;
};

const cache = new Map<string, ServerSwrEntry<unknown>>();

export async function readThroughServerSwr<T>(
  key: string,
  options: ServerSwrOptions,
  load: () => Promise<T>
): Promise<T> {
  const nowMs = options.nowMs ?? Date.now();
  const entry = cache.get(key) as ServerSwrEntry<T> | undefined;

  if (entry?.hasValue) {
    const ageMs = nowMs - entry.updatedAtMs;
    if (ageMs <= options.freshMs) {
      return entry.value as T;
    }
    if (ageMs <= options.staleMs) {
      refreshServerSwrEntry(key, entry, load);
      return entry.value as T;
    }
  }

  return refreshServerSwrEntry(key, entry, load);
}

export function invalidateServerSwrCache(prefix?: string) {
  if (!prefix) {
    cache.clear();
    return;
  }
  for (const key of cache.keys()) {
    if (key.startsWith(prefix)) {
      cache.delete(key);
    }
  }
}

function refreshServerSwrEntry<T>(
  key: string,
  entry: ServerSwrEntry<T> | undefined,
  load: () => Promise<T>
) {
  if (entry?.inFlight) {
    return entry.inFlight;
  }

  const nextEntry: ServerSwrEntry<T> =
    entry ?? { hasValue: false, updatedAtMs: 0 };
  const inFlight = load()
    .then((value) => {
      cache.set(key, {
        hasValue: true,
        value,
        updatedAtMs: Date.now(),
      });
      return value;
    })
    .catch((error) => {
      if (nextEntry.hasValue) {
        cache.set(key, {
          hasValue: true,
          value: nextEntry.value,
          updatedAtMs: nextEntry.updatedAtMs,
        });
      } else {
        cache.delete(key);
      }
      throw error;
    });

  nextEntry.inFlight = inFlight;
  cache.set(key, nextEntry);

  if (nextEntry.hasValue) {
    inFlight.catch(() => {});
  }

  return inFlight;
}
