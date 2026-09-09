export type HostedGatewayStatus = { enabled: boolean; ready: boolean; token: string | null; url: string };

/** Server-only deployment configuration, keyed by Core's attributed runner. */
export function hostedGatewayUrl(runtimeId: string, hostId: string, domains = process.env.FC_HOSTED_GATEWAY_RUNNER_DOMAINS): string | null {
  if (!/^runtime_[0-9a-f]{20}$/.test(runtimeId) || !domains) return null;
  try {
    const domain: unknown = JSON.parse(domains)[hostId];
    if (typeof domain !== "string" || !/^(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+finite\.computer$/.test(domain)) return null;
    return `wss://r-${runtimeId.slice(8)}.${domain}/api/ws`;
  } catch { return null; }
}

export function parseHostedGatewayStatus(value: unknown): Omit<HostedGatewayStatus, "url"> {
  if (!value || typeof value !== "object") throw new Error("Invalid gateway status");
  const status = value as Record<string, unknown>;
  if (typeof status.enabled !== "boolean" || typeof status.ready !== "boolean" ||
      (status.ready && !status.enabled) ||
      (status.enabled ? typeof status.token !== "string" || !/^[0-9a-f]{64}$/.test(status.token) : status.token !== null)) {
    throw new Error("Invalid gateway status");
  }
  return { enabled: status.enabled, ready: status.ready, token: status.token as string | null };
}
