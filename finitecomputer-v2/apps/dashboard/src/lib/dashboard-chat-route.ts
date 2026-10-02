export type DashboardChatSurface = "chat" | "hermes-chat";

export function dashboardChatMachineIdFromPath(pathname: string): string | null {
  const match = pathname.match(/^\/dashboard\/machines\/([^/]+)\/(?:chat|hermes-chat)\/?$/u);
  if (!match?.[1]) {
    return null;
  }

  try {
    return decodeURIComponent(match[1]);
  } catch {
    return null;
  }
}

/** Which chat surface a path mounts: Finite Chat or the admin Hermes preview. */
export function dashboardChatSurfaceFromPath(pathname: string): DashboardChatSurface | null {
  const match = pathname.match(/^\/dashboard\/machines\/[^/]+\/(chat|hermes-chat)\/?$/u);
  return (match?.[1] as DashboardChatSurface | undefined) ?? null;
}
