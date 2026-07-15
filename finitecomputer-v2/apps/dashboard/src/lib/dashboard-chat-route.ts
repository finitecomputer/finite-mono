export function dashboardChatMachineIdFromPath(pathname: string): string | null {
  const match = pathname.match(/^\/dashboard\/machines\/([^/]+)\/chat\/?$/u);
  if (!match?.[1]) {
    return null;
  }

  try {
    return decodeURIComponent(match[1]);
  } catch {
    return null;
  }
}
