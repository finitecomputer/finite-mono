/**
 * Resolve browser-facing redirects from configured public origin rather than
 * Next's loopback request URL behind Caddy/host networking.
 */
export function dashboardBaseUrl(
  requestUrl: string,
  env: Record<string, string | undefined> = process.env
) {
  for (const candidate of [
    env.FC_DASHBOARD_PUBLIC_URL,
    env.NEXT_PUBLIC_APP_URL,
    env.FC_DASHBOARD_BASE_URL,
    env.NEXT_PUBLIC_WORKOS_REDIRECT_URI,
    requestUrl,
  ]) {
    if (!candidate?.trim()) continue;
    try {
      const parsed = new URL(candidate);
      if (parsed.protocol === "http:" || parsed.protocol === "https:") {
        return parsed.origin;
      }
    } catch {
      // Try the next configured public URL.
    }
  }
  throw new Error("Dashboard URL is unavailable.");
}
