// Curated Hermes slash commands for the web chat composer (FIN-114 catalog
// v2). Discovery tier and execution are separate: only `restricted` commands
// are refused. `unlisted` commands and anything Hermes knows that is not in
// this catalog (skills, plugins) still send normally. The catalog array is
// exported so the agent-side policy can be cross-checked against it.

export type SlashCommandTier =
  | "suggested"
  | "available"
  | "not_recommended"
  | "unlisted"
  | "restricted";

export type SlashCommand = {
  name: string;
  aliases: readonly string[];
  args: string | null;
  description: string;
  tier: SlashCommandTier;
  reason?: string;
};

const SOFTWARE_REASON = "Finite manages your agent's software and restarts.";
const APPROVALS_REASON = "Approval settings can't be changed from chat.";
const COMMANDS_REASON = "Type / in the message box to see the commands you can use.";
const UNAVAILABLE_REASON = "This command isn't available in Finite chat.";

export const SLASH_COMMANDS: readonly SlashCommand[] = [
  // Starts a new Hermes session in the same chat; the UI does not switch.
  // When idle, the agent asks for confirmation with /approve.
  {
    name: "new",
    aliases: ["reset"],
    args: "[name]",
    description:
      "Start a new session in this chat. Your agent starts fresh; earlier messages stay on screen.",
    tier: "suggested",
  },
  {
    name: "stop",
    aliases: [],
    args: null,
    description: "Stop what your agent is doing in this conversation.",
    tier: "suggested",
  },
  {
    name: "steer",
    aliases: [],
    args: "<prompt>",
    description: "Nudge your agent mid-task. Your note is added after its next tool call.",
    tier: "suggested",
  },
  {
    name: "btw",
    aliases: [],
    args: "<question>",
    description: "Ask a quick side question without interrupting the current task.",
    tier: "suggested",
  },
  {
    name: "status",
    aliases: [],
    args: null,
    description: "Show the model, token use, and context size for this conversation.",
    tier: "suggested",
  },
  {
    name: "queue",
    aliases: ["q"],
    args: "<prompt>",
    description: "Line up a message to run after the current task.",
    tier: "available",
  },
  {
    name: "retry",
    aliases: [],
    args: null,
    description: "Send your last message again. Only works when your agent is idle.",
    tier: "available",
  },
  {
    name: "context",
    aliases: ["ctx"],
    args: "[all]",
    description: "See what is filling your agent's context window.",
    tier: "available",
  },
  {
    name: "compress",
    aliases: ["compact"],
    args: null,
    description: "Summarize older messages to free up context.",
    tier: "available",
  },
  {
    name: "usage",
    aliases: [],
    args: null,
    description: "Show token usage for this conversation.",
    tier: "available",
  },
  {
    name: "plan",
    aliases: [],
    args: "[task]",
    description: "Have your agent write a plan without doing the work yet.",
    tier: "available",
  },
  {
    name: "learn",
    aliases: [],
    args: "<what to learn from>",
    description: "Teach your agent a reusable skill from a URL, notes, or this chat.",
    tier: "available",
  },
  {
    name: "bg",
    aliases: [],
    args: "<prompt>",
    description:
      "Run a prompt as a separate background task. The result comes back to this chat.",
    tier: "available",
  },
  {
    name: "goal",
    aliases: [],
    args: "[text | show | pause | resume | clear]",
    description: "Give your agent a standing goal it keeps working toward across turns.",
    tier: "available",
  },
  {
    name: "reasoning",
    aliases: [],
    args: "[level]",
    description: "Change how hard your agent thinks before answering.",
    tier: "available",
  },
  {
    name: "personality",
    aliases: [],
    args: "[name]",
    description:
      "Switch your agent's personality. Applies to every chat, including Telegram.",
    tier: "available",
  },
  {
    name: "reload-skills",
    aliases: ["reload_skills"],
    args: null,
    description: "Refresh your agent's skill commands after adding or removing skills.",
    tier: "available",
  },
  {
    name: "version",
    aliases: ["v"],
    args: null,
    description: "Show your agent's Hermes version.",
    tier: "available",
  },
  {
    name: "model",
    aliases: [],
    args: "[model]",
    description: "Change the AI model for this conversation.",
    tier: "not_recommended",
    reason:
      "Applies to this conversation only and won't show in Connections. Change models in Connections instead.",
  },
  {
    name: "undo",
    aliases: [],
    args: "[N]",
    description: "Remove your last N turns from your agent's memory.",
    tier: "not_recommended",
    reason:
      "The messages stay in this chat, so what you see and what your agent remembers will differ.",
  },
  {
    name: "pause",
    aliases: [],
    args: "[reason | off]",
    description: "Pause all new work until you send /pause off.",
    tier: "not_recommended",
    reason:
      "Pauses your agent everywhere, including Telegram and scheduled tasks, until you send /pause off.",
  },
  {
    name: "heartbeat",
    aliases: ["hb"],
    args: null,
    description: "Have your agent check back in on a schedule.",
    tier: "not_recommended",
    reason: "Stops whenever your agent restarts.",
  },

  // Runnable but never shown. Hermes owns their execution behavior.
  ...[
    "approve", "deny", "always", "cancel", "yes", "no",
    "resume", "sessions", "subgoal", "refine", "review",
  ].map((name) => unlisted(name)),
  unlisted("branch", ["fork"]),
  unlisted("agents", ["tasks"]),

  // Refused in web chat; the reason is shown in the composer and the refusal.
  restricted("update", SOFTWARE_REASON),
  restricted("restart", SOFTWARE_REASON),
  restricted(
    "debug",
    "It uploads your agent's logs to a public paste service. Contact Finite support instead."
  ),
  restricted("yolo", APPROVALS_REASON),
  restricted("approvals", APPROVALS_REASON),
  restricted("topup", "Billing is managed in your Finite account."),
  restricted("save", "Exporting from chat isn't supported yet."),
  restricted("title", "Rename chats from the sidebar."),
  restricted("help", COMMANDS_REASON),
  restricted("commands", COMMANDS_REASON),
  restricted("loop", "Recurring command loops aren't available in Finite chat.", ["proactive"]),
  ...[
    "rollback", "memory", "skills", "voice", "fast", "moa", "init", "bundles",
    "diff", "whoami", "profile", "insights", "kanban", "curator", "busy", "footer",
    "verbose", "topic", "start", "platform", "egress",
  ].map((name) => restricted(name)),
  restricted("reload-mcp", UNAVAILABLE_REASON, ["reload_mcp"]),
  restricted("sethome", UNAVAILABLE_REASON, ["set-home"]),
  restricted("codex-runtime", UNAVAILABLE_REASON, ["codex_runtime"]),
  restricted("suggestions", UNAVAILABLE_REASON, ["suggest"]),
  restricted("blueprint", UNAVAILABLE_REASON, ["bp"]),
];

function unlisted(name: string, aliases: string[] = []): SlashCommand {
  return { name, aliases, args: null, description: "", tier: "unlisted" };
}

function restricted(
  name: string,
  reason = UNAVAILABLE_REASON,
  aliases: string[] = []
): SlashCommand {
  return { name, aliases, args: null, description: "", tier: "restricted", reason };
}

const TIER_RANK: Record<SlashCommandTier, number> = {
  suggested: 0,
  available: 1,
  not_recommended: 2,
  unlisted: 3,
  restricted: 4,
};

// Python str.isspace(), which Hermes uses to strip and split commands. Exact
// parity: unlike JavaScript \s it includes U+001C..U+001F and U+0085 and
// excludes U+FEFF.
const PY_SPACE =
  "\\t\\n\\v\\f\\r\\x1c-\\x1f \\x85\\xa0\\u1680\\u2000-\\u200a\\u2028\\u2029\\u202f\\u205f\\u3000";
const SLASH_QUERY = new RegExp(`^/([^${PY_SPACE}/]*)$`);
const COMMAND_TOKEN = new RegExp(`^[${PY_SPACE}]*/([^${PY_SPACE}]+)`);
const LOOP_STOP_OR_STATUS = new RegExp(
  `^[${PY_SPACE}]*(?:status|pause|stop|clear|cancel)[${PY_SPACE}]*$`,
  "i"
);
const INSERTED_COMMAND = new RegExp(`^/([^${PY_SPACE}/]+) $`);

const LISTED_COMMANDS = SLASH_COMMANDS
  .filter((command) => TIER_RANK[command.tier] < TIER_RANK.unlisted)
  .sort((a, b) => TIER_RANK[a.tier] - TIER_RANK[b.tier]);

/** The command token being typed: the draft is `/` plus non-space text only. */
export function slashQuery(draft: string): string | null {
  const match = SLASH_QUERY.exec(draft);
  return match ? match[1] : null;
}

/** Name and alias prefix matches first, then description matches; shown tiers only. */
export function matchSlashCommands(query: string): SlashCommand[] {
  const needle = query.toLowerCase();
  const byName = LISTED_COMMANDS.filter((command) =>
    commandNames(command).some((name) => name.startsWith(needle))
  );
  const byDescription = LISTED_COMMANDS.filter(
    (command) =>
      !byName.includes(command) && command.description.toLowerCase().includes(needle)
  );
  return [...byName, ...byDescription];
}

/** The catalog entry a command name or alias names, underscores equal to hyphens. */
export function findSlashCommand(name: string): SlashCommand | null {
  const wanted = normalizeCommandName(name);
  return (
    SLASH_COMMANDS.find((command) =>
      commandNames(command).some((candidate) => normalizeCommandName(candidate) === wanted)
    ) ?? null
  );
}

/**
 * The restricted command a whole message invokes, parsed the way Hermes reads
 * commands: leading whitespace ignored, first token, case-insensitive,
 * `@bot` suffix dropped, underscores equal to hyphens.
 */
export function restrictedSlashCommand(text: string): SlashCommand | null {
  const match = COMMAND_TOKEN.exec(text);
  if (!match) return null;
  const token = match[1];
  const name = token.split("@", 1)[0];
  if (!name || name.includes("/")) return null;
  const command = findSlashCommand(name);
  // Existing loops survive upgrades. Keep their inspection/shut-off controls
  // reachable without allowing creation or resumption of recurring work.
  if (command?.name === "loop" && LOOP_STOP_OR_STATUS.test(text.slice(match[0].length))) {
    return null;
  }
  return command?.tier === "restricted" ? command : null;
}

export function restrictedSlashCommandMessage(command: SlashCommand) {
  return `/${command.name} isn't available in Finite. ${command.reason ?? ""}`.trim();
}

/** Argument signature to show after the caret right after a command is inserted. */
export function slashArgsHint(draft: string): string | null {
  const token = INSERTED_COMMAND.exec(draft)?.[1]?.toLowerCase();
  if (!token) return null;
  const command = LISTED_COMMANDS.find((candidate) => commandNames(candidate).includes(token));
  return command?.args ?? null;
}

/**
 * Whether the picker shows for this draft. A restricted command always shows
 * why Send is disabled. A slash draft that came straight from a paste, or
 * that names an unlisted command, keeps Enter sending as before.
 */
export function slashPickerVisible(
  draft: string,
  { dismissed, pastedDraft }: { dismissed: boolean; pastedDraft: string | null }
) {
  if (restrictedSlashCommand(draft)) return true;
  const query = slashQuery(draft);
  if (dismissed || query === null || draft === pastedDraft) return false;
  return findSlashCommand(query)?.tier !== "unlisted";
}

/** Esc keeps the picker closed only while the same slash query grows. */
export function keepSlashDismissed(previousDraft: string, nextDraft: string) {
  return (
    slashQuery(previousDraft) !== null
    && slashQuery(nextDraft) !== null
    && nextDraft.startsWith(previousDraft)
  );
}

/**
 * Whether Enter inserts the highlighted command instead of sending. The
 * person is choosing when they moved the highlight, or when the highlight
 * matched by name or alias prefix. A fully typed listed command and
 * description-only matches send what was typed.
 */
export function slashEnterInserts(
  query: string,
  highlighted: SlashCommand | undefined,
  moved: boolean
) {
  if (!highlighted) return false;
  if (moved) return true;
  const exact = findSlashCommand(query);
  if (exact && TIER_RANK[exact.tier] < TIER_RANK.unlisted) return false;
  const needle = query.toLowerCase();
  return commandNames(highlighted).some((name) => name.startsWith(needle));
}

/** Input events that insert text the person did not type key by key. */
export function isPastedInput(inputType: string | undefined) {
  return (
    inputType === "insertFromPaste"
    || inputType === "insertFromPasteAsQuotation"
    || inputType === "insertFromDrop"
  );
}

export type SlashPickerKeyAction = "dismiss" | "previous" | "next" | "insert";

/** What a composer keydown does to the picker; null leaves the key alone. */
export function slashPickerKeyAction(
  event: { key: string; shiftKey: boolean; isComposing: boolean; keyCode: number },
  picker: { open: boolean; navigable: boolean; blocked: boolean; enterInserts: boolean }
): SlashPickerKeyAction | null {
  // IME candidate keys belong to the input method. 229 covers browsers
  // that report composition keydowns without isComposing.
  if (!picker.open || event.isComposing || event.keyCode === 229) return null;
  if (event.key === "Escape") return picker.blocked ? null : "dismiss";
  if (!picker.navigable || event.shiftKey) return null;
  if (event.key === "ArrowDown") return "next";
  if (event.key === "ArrowUp") return "previous";
  if (event.key === "Tab") return "insert";
  if (event.key === "Enter" && picker.enterInserts) return "insert";
  return null;
}

function commandNames(command: SlashCommand) {
  return [command.name, ...command.aliases];
}

function normalizeCommandName(name: string) {
  return name.toLowerCase().replaceAll("_", "-");
}
