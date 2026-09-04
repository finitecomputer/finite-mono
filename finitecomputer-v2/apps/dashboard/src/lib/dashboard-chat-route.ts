export function dashboardChatMachineIdFromPath(pathname: string): string | null {
  const match = pathname.match(/^\/dashboard\/machines\/([^/]+)\/(?:chat|gateway-chat)\/?$/u);
  if (!match?.[1]) {
    return null;
  }

  try {
    return decodeURIComponent(match[1]);
  } catch {
    return null;
  }
}

/**
 * Spike: the gateway-chat surface mounts the SAME chat components over the
 * hermes tui_gateway WebSocket instead of the finitechat hosted device.
 */
export function dashboardGatewayChatFromPath(pathname: string): boolean {
  return /\/gateway-chat\/?$/u.test(pathname);
}
