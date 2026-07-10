export const AGENT_CONTACT_FETCH_TIMEOUT_MS = 5_000;

// The runtime contact document exposes the Agent Principal without coupling
// the dashboard to chat admission or a particular Device implementation.
export async function fetchRuntimeAgentNpub(
  runtimeContactUrl: string | null | undefined,
  options: { timeoutMs?: number } = {}
): Promise<string | null> {
  if (!safeHttpUrl(runtimeContactUrl)) {
    return null;
  }

  try {
    const response = await fetch(runtimeContactUrl, {
      cache: "no-store",
      headers: { accept: "application/json" },
      signal: AbortSignal.timeout(options.timeoutMs ?? AGENT_CONTACT_FETCH_TIMEOUT_MS),
    });
    if (!response.ok) {
      return null;
    }
    const payload = JSON.parse(await response.text()) as unknown;
    if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
      return null;
    }
    const agentNpub = optionalTrimmedString(
      (payload as Record<string, unknown>).agent_npub
    );
    if (!agentNpub || !agentNpub.toLowerCase().startsWith("npub1") || agentNpub.length > 256) {
      return null;
    }
    return agentNpub;
  } catch {
    return null;
  }
}

// Middle-truncates an npub for display: the bech32 prefix and checksum tail
// are what humans eyeball-compare.
export function truncateNpub(value: string, headLength = 12, tailLength = 6) {
  if (value.length <= headLength + tailLength + 1) {
    return value;
  }
  return `${value.slice(0, headLength)}…${value.slice(-tailLength)}`;
}

function optionalTrimmedString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function safeHttpUrl(value: string | null | undefined): value is string {
  if (!value?.trim()) {
    return false;
  }
  try {
    const url = new URL(value);
    return url.protocol === "https:" || url.protocol === "http:";
  } catch {
    return false;
  }
}
