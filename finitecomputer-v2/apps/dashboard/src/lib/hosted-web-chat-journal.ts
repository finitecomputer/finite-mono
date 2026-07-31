import type { HostedChatSelection } from "@/lib/hosted-web-chat-selection";

const MAX_NAVIGATION_JOURNAL_ENTRIES = 100;
const NAVIGATION_JOURNAL_KEY = "__finiteChatNavigationJournal";

export type HostedChatSnapshotSource =
  | "http"
  | "mutation"
  | "pending_refresh"
  | "sse"
  | "electron_stream"
  | "navigation";

export type HostedChatNavigationJournalEntry = {
  order: number;
  observed_at_ms: number;
  source: HostedChatSnapshotSource;
  snapshot_sequence: number;
  snapshot_rev: number;
  snapshot_selection: HostedChatSelection;
  visible_selection: HostedChatSelection;
  navigation_intent_generation: number;
  decision: "initial" | "preserved" | "fallback" | "navigation";
};

type NavigationJournalWindow = Window & {
  [NAVIGATION_JOURNAL_KEY]?: HostedChatNavigationJournalEntry[];
};

/**
 * Bounded, content-free development diagnostics for navigation ownership.
 * Message bodies, attachment names, identities, and email never enter it.
 */
export function recordHostedChatNavigation(
  entry: Omit<HostedChatNavigationJournalEntry, "order" | "observed_at_ms">
) {
  if (process.env.NODE_ENV === "production" || typeof window === "undefined") return;
  const target = window as NavigationJournalWindow;
  const journal = target[NAVIGATION_JOURNAL_KEY] ?? [];
  const previousOrder = journal[journal.length - 1]?.order ?? 0;
  journal.push({
    ...entry,
    order: previousOrder + 1,
    observed_at_ms: Date.now(),
  });
  if (journal.length > MAX_NAVIGATION_JOURNAL_ENTRIES) {
    journal.splice(0, journal.length - MAX_NAVIGATION_JOURNAL_ENTRIES);
  }
  target[NAVIGATION_JOURNAL_KEY] = journal;
}

export function hostedChatNavigationJournalForTest() {
  if (typeof window === "undefined") return [];
  return [
    ...((window as NavigationJournalWindow)[NAVIGATION_JOURNAL_KEY] ?? []),
  ];
}
