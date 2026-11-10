/** FIN-463: an open dashboard rechecks, on each navigation and every 30
 * seconds, whether its person now goes to finite.chat. The layout decides on
 * a full load, but layouts don't re-render on client navigation. */
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
      /* The next navigation or tick checks again. */
    }
  }
  void check();
  const timer = setInterval(() => void check(), 30_000);
  return () => {
    clearInterval(timer);
    controller.abort();
  };
}
