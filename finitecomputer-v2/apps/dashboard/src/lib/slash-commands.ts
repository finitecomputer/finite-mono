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

// Hermes 0.21 busy-session policy (hermes_cli/commands.py). `reject` means
// Hermes refuses the command while the agent is working; it does not wait.
// `null` marks bare approval replies, which are not registry commands.
export type SlashCommandBusy = "dispatch" | "interrupt_then_dispatch" | "reject";

export type SlashCommand = {
  name: string;
  aliases: readonly string[];
  args: string | null;
  description: string;
  tier: SlashCommandTier;
  reason?: string;
  busy: SlashCommandBusy | null;
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
    busy: "interrupt_then_dispatch",
  },
  {
    name: "stop",
    aliases: [],
    args: null,
    description: "Stop what your agent is doing in this conversation.",
    tier: "suggested",
    busy: "interrupt_then_dispatch",
  },
  {
    name: "steer",
    aliases: [],
    args: "<prompt>",
    description: "Nudge your agent mid-task. Your note is added after its next tool call.",
    tier: "suggested",
    busy: "dispatch",
  },
  {
    name: "btw",
    aliases: [],
    args: "<question>",
    description: "Ask a quick side question without interrupting the current task.",
    tier: "suggested",
    busy: "dispatch",
  },
  {
    name: "status",
    aliases: [],
    args: null,
    description: "Show the model, token use, and context size for this conversation.",
    tier: "suggested",
    busy: "dispatch",
  },
  {
    name: "queue",
    aliases: ["q"],
    args: "<prompt>",
    description: "Line up a message to run after the current task.",
    tier: "available",
    busy: "dispatch",
  },
  {
    name: "retry",
    aliases: [],
    args: null,
    description: "Send your last message again. Only works when your agent is idle.",
    tier: "available",
    busy: "reject",
  },
  {
    name: "context",
    aliases: ["ctx"],
    args: "[all]",
    description: "See what is filling your agent's context window.",
    tier: "available",
    busy: "dispatch",
  },
  {
    name: "compress",
    aliases: ["compact"],
    args: null,
    description: "Summarize older messages to free up context.",
    tier: "available",
    busy: "reject",
  },
  {
    name: "usage",
    aliases: [],
    args: null,
    description: "Show token usage for this conversation.",
    tier: "available",
    busy: "reject",
  },
  {
    name: "plan",
    aliases: [],
    args: "[task]",
    description: "Have your agent write a plan without doing the work yet.",
    tier: "available",
    busy: "reject",
  },
  {
    name: "learn",
    aliases: [],
    args: "<what to learn from>",
    description: "Teach your agent a reusable skill from a URL, notes, or this chat.",
    tier: "available",
    busy: "reject",
  },
  {
    name: "bg",
    aliases: [],
    args: "<prompt>",
    description:
      "Run a prompt as a separate background task. The result comes back to this chat.",
    tier: "available",
    busy: "dispatch",
  },
  {
    name: "goal",
    aliases: [],
    args: "[text | show | pause | resume | clear]",
    description: "Give your agent a standing goal it keeps working toward across turns.",
    tier: "available",
    busy: "dispatch",
  },
  {
    name: "reasoning",
    aliases: [],
    args: "[level]",
    description: "Change how hard your agent thinks before answering.",
    tier: "available",
    busy: "reject",
  },
  {
    name: "personality",
    aliases: [],
    args: "[name]",
    description:
      "Switch your agent's personality. Applies to every chat, including Telegram.",
    tier: "available",
    busy: "reject",
  },
  {
    name: "reload-skills",
    aliases: ["reload_skills"],
    args: null,
    description: "Refresh your agent's skill commands after adding or removing skills.",
    tier: "available",
    busy: "reject",
  },
  {
    name: "version",
    aliases: ["v"],
    args: null,
    description: "Show your agent's Hermes version.",
    tier: "available",
    busy: "dispatch",
  },
  {
    name: "model",
    aliases: [],
    args: "[model]",
    description: "Change the AI model for this conversation.",
    tier: "not_recommended",
    reason:
      "Applies to this conversation only and won't show in Connections. Change models in Connections instead.",
    busy: "reject",
  },
  {
    name: "undo",
    aliases: [],
    args: "[N]",
    description: "Remove your last N turns from your agent's memory.",
    tier: "not_recommended",
    reason:
      "The messages stay in this chat, so what you see and what your agent remembers will differ.",
    busy: "reject",
  },
  {
    name: "pause",
    aliases: [],
    args: "[reason | off]",
    description: "Pause all new work until you send /pause off.",
    tier: "not_recommended",
    reason:
      "Pauses your agent everywhere, including Telegram and scheduled tasks, until you send /pause off.",
    busy: "dispatch",
  },
  {
    name: "heartbeat",
    aliases: ["hb"],
    args: null,
    description: "Have your agent check back in on a schedule.",
    tier: "not_recommended",
    reason: "Stops whenever your agent restarts.",
    busy: "dispatch",
  },

  // Runnable but never shown. Descriptions are maintainer notes, not UI copy.
  unlisted("approve", [], "dispatch", "Approve a pending dangerous command."),
  unlisted("deny", [], "dispatch", "Deny a pending dangerous command."),
  unlisted("always", [], null, "Approval reply: approve always."),
  unlisted("cancel", [], null, "Approval reply: deny."),
  unlisted("yes", [], null, "Approval reply: approve."),
  unlisted("no", [], null, "Approval reply: deny."),
  unlisted("resume", [], "reject", "Resume a previously named session."),
  unlisted("sessions", [], "reject", "Browse and resume previous sessions."),
  unlisted("branch", ["fork"], "reject", "Branch the current session."),
  unlisted("subgoal", [], "dispatch", "Add or manage extra criteria on the active goal."),
  unlisted("refine", [], "reject", "Save lessons from this conversation to memory or skills."),
  unlisted("review", [], "reject", "Have a subagent review the work just discussed."),
  unlisted("agents", ["tasks"], "dispatch", "Show active agents and running tasks."),

  // Refused in web chat; the reason is shown in the composer and the refusal.
  restricted("update", [], "dispatch", "Update Hermes.", SOFTWARE_REASON),
  restricted("restart", [], "dispatch", "Restart the gateway.", SOFTWARE_REASON),
  restricted(
    "debug",
    [],
    "reject",
    "Upload a debug report with logs.",
    "It uploads your agent's logs to a public paste service. Contact Finite support instead."
  ),
  restricted("yolo", [], "dispatch", "Skip dangerous-command approvals.", APPROVALS_REASON),
  restricted("approvals", [], "reject", "Set the approval mode.", APPROVALS_REASON),
  restricted(
    "topup",
    [],
    "reject",
    "Manage Nous billing.",
    "Billing is managed in your Finite account."
  ),
  restricted(
    "save",
    [],
    "reject",
    "Export the current conversation.",
    "Exporting from chat isn't supported yet."
  ),
  restricted("title", [], "reject", "Set the session title.", "Rename chats from the sidebar."),
  restricted("help", [], "dispatch", "Show available commands.", COMMANDS_REASON),
  restricted("commands", [], "dispatch", "Browse commands and skills.", COMMANDS_REASON),
  restricted(
    "loop",
    ["proactive"],
    "dispatch",
    "Re-run a prompt on an interval.",
    "Recurring command loops aren't available in Finite chat."
  ),
  restricted("rollback", [], "reject", "Restore filesystem checkpoints.", UNAVAILABLE_REASON),
  restricted("memory", [], "reject", "Review pending memory writes.", UNAVAILABLE_REASON),
  restricted("skills", [], "reject", "Search, install, or manage skills.", UNAVAILABLE_REASON),
  restricted("voice", [], "reject", "Toggle voice mode.", UNAVAILABLE_REASON),
  restricted("fast", [], "reject", "Toggle fast mode.", UNAVAILABLE_REASON),
  restricted("moa", [], "reject", "Run a Mixture of Agents prompt.", UNAVAILABLE_REASON),
  restricted("init", [], "reject", "Generate AGENTS.md from a repo scan.", UNAVAILABLE_REASON),
  restricted("bundles", [], "reject", "List skill bundles.", UNAVAILABLE_REASON),
  restricted("diff", [], "reject", "Show git changes.", UNAVAILABLE_REASON),
  restricted("whoami", [], "reject", "Show slash command access.", UNAVAILABLE_REASON),
  restricted("profile", [], "dispatch", "Show the active profile.", UNAVAILABLE_REASON),
  restricted("insights", [], "reject", "Show usage insights.", UNAVAILABLE_REASON),
  restricted("kanban", [], "dispatch", "Multi-profile task board.", UNAVAILABLE_REASON),
  restricted("curator", [], "reject", "Background skill maintenance.", UNAVAILABLE_REASON),
  restricted("reload-mcp", ["reload_mcp"], "reject", "Reload MCP servers.", UNAVAILABLE_REASON),
  restricted("busy", [], "dispatch", "Set busy-message behavior.", UNAVAILABLE_REASON),
  restricted("footer", [], "dispatch", "Toggle the runtime footer.", UNAVAILABLE_REASON),
  restricted("verbose", [], "dispatch", "Cycle tool progress display.", UNAVAILABLE_REASON),
  restricted("topic", [], "reject", "Telegram DM topic sessions.", UNAVAILABLE_REASON),
  restricted("start", [], "dispatch", "Acknowledge platform start pings.", UNAVAILABLE_REASON),
  restricted("platform", [], "reject", "Pause or resume gateway platforms.", UNAVAILABLE_REASON),
  restricted("sethome", ["set-home"], "reject", "Set the home channel.", UNAVAILABLE_REASON),
  restricted("egress", [], "dispatch", "Show egress proxy status.", UNAVAILABLE_REASON),
  restricted(
    "codex-runtime",
    ["codex_runtime"],
    "reject",
    "Toggle the Codex app-server runtime.",
    UNAVAILABLE_REASON
  ),
  restricted(
    "suggestions",
    ["suggest"],
    "reject",
    "Review suggested automations.",
    UNAVAILABLE_REASON
  ),
  restricted(
    "blueprint",
    ["bp"],
    "reject",
    "Set up an automation from a blueprint.",
    UNAVAILABLE_REASON
  ),
];

function unlisted(
  name: string,
  aliases: string[],
  busy: SlashCommandBusy | null,
  description: string
): SlashCommand {
  return { name, aliases, args: null, description, tier: "unlisted", busy };
}

function restricted(
  name: string,
  aliases: string[],
  busy: SlashCommandBusy,
  description: string,
  reason: string
): SlashCommand {
  return { name, aliases, args: null, description, tier: "restricted", reason, busy };
}

const TIER_RANK: Record<SlashCommandTier, number> = {
  suggested: 0,
  available: 1,
  not_recommended: 2,
  unlisted: 3,
  restricted: 4,
};

const LISTED_COMMANDS = SLASH_COMMANDS
  .filter((command) => TIER_RANK[command.tier] < TIER_RANK.unlisted)
  .sort((a, b) => TIER_RANK[a.tier] - TIER_RANK[b.tier]);

/** The command token being typed: the draft is `/` plus non-space text only. */
export function slashQuery(draft: string): string | null {
  const match = /^\/([^\s/]*)$/.exec(draft);
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
  const token = /^\/(\S+)/.exec(text.trimStart())?.[1];
  if (!token) return null;
  const name = token.split("@", 1)[0];
  if (!name || name.includes("/")) return null;
  const command = findSlashCommand(name);
  return command?.tier === "restricted" ? command : null;
}

export function restrictedSlashCommandMessage(command: SlashCommand) {
  return `/${command.name} isn't available in Finite. ${command.reason ?? ""}`.trim();
}

/** Argument signature to show after the caret right after a command is inserted. */
export function slashArgsHint(draft: string): string | null {
  const token = /^\/([^\s/]+) $/.exec(draft)?.[1]?.toLowerCase();
  if (!token) return null;
  const command = LISTED_COMMANDS.find((candidate) => commandNames(candidate).includes(token));
  return command?.args ?? null;
}

function commandNames(command: SlashCommand) {
  return [command.name, ...command.aliases];
}

function normalizeCommandName(name: string) {
  return name.toLowerCase().replaceAll("_", "-");
}
