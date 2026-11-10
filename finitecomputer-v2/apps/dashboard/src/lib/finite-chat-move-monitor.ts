/** FIN-463: an open dashboard rechecks, once on each navigation, whether
 * its person now goes to finite.chat. The layout decides on a full load, but
 * layouts don't re-render on client navigation. A tab left idle follows on
 * its next navigation or reload. */
export const FINITE_CHAT_MOVE_ROUTE = "/api/finite-chat-move";

export function startFiniteChatMoveMonitor({
  replace,
  fetcher = fetch,
}: {
  replace: (url: string) => void;
  fetcher?: typeof fetch;
}) {
  const controller = new AbortController();
  let leaving = false;
  async function check() {
    if (leaving || controller.signal.aborted) return;
    try {
      const response = await fetcher(FINITE_CHAT_MOVE_ROUTE, { cache: "no-store", signal: controller.signal });
      if (!response.ok) return;
      const status = await response.json();
      const redirect = status?.redirect;
      if (controller.signal.aborted || typeof redirect !== "string" || !/^https?:\/\//.test(redirect)) return;
      leaving = true;
      replace(redirect);
    } catch {
      /* The next navigation checks again. */
    }
  }
  void check();
  return () => controller.abort();
}
