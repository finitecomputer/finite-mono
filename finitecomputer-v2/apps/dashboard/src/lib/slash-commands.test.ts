import assert from "node:assert/strict";
import test from "node:test";

import {
  SLASH_COMMANDS,
  findSlashCommand,
  matchSlashCommands,
  restrictedSlashCommand,
  restrictedSlashCommandMessage,
  slashArgsHint,
  slashQuery,
  type SlashCommand,
} from "@/lib/slash-commands";

const names = (query: string) => matchSlashCommands(query).map((command) => command.name);
const LISTED = new Set(["suggested", "available", "not_recommended"]);

test("slashQuery returns the command token only while it is being typed", () => {
  assert.equal(slashQuery("/"), "");
  assert.equal(slashQuery("/qu"), "qu");
  assert.equal(slashQuery("/Model"), "Model");
  assert.equal(slashQuery("/reload_skills"), "reload_skills");
});

test("slashQuery ignores prose, URLs, paths, args, and pasted text", () => {
  for (const draft of [
    "",
    "hello",
    "a/b",
    "see /new",
    "https://example.com/new",
    " /new",
    "\n/new",
    "/new trip",
    "/queue ",
    "/new\nsecond line",
    "/usr/local/bin",
    "//comment",
  ]) {
    assert.equal(slashQuery(draft), null, JSON.stringify(draft));
  }
});

test("an empty query lists suggested, then available, then not recommended", () => {
  const listed = matchSlashCommands("");
  const tiers = listed.map((command) => command.tier);
  assert.deepEqual(tiers, [...tiers].sort((a, b) => tierRank(a) - tierRank(b)));
  assert.deepEqual(
    listed.slice(0, 5).map((command) => command.name),
    ["new", "stop", "steer", "btw", "status"]
  );
  assert.deepEqual(
    listed.filter((command) => command.tier === "not_recommended").map((command) => command.name),
    ["model", "undo", "pause", "heartbeat"]
  );
  assert.ok(listed.every((command) => LISTED.has(command.tier)));
});

test("name prefix matches rank before description matches", () => {
  const matched = names("mo");
  assert.equal(matched[0], "model");
  assert.ok(matched.includes("status"), "description mentions the model");
  assert.ok(matched.indexOf("model") < matched.indexOf("status"));
});

test("/mo leads with /model as not recommended, with its reason", () => {
  const [model] = matchSlashCommands("mo");
  assert.equal(model.name, "model");
  assert.equal(model.tier, "not_recommended");
  assert.equal(
    model.reason,
    "Applies to this conversation only and won't show in Connections. Change models in Connections instead."
  );
});

test("matching is case-insensitive and includes aliases", () => {
  assert.deepEqual(names("Q").slice(0, 1), ["queue"]);
  assert.equal(names("ctx")[0], "context");
  assert.equal(names("compact")[0], "compress");
  assert.equal(names("reload_")[0], "reload-skills");
  assert.equal(names("v")[0], "version");
  assert.equal(names("hb")[0], "heartbeat");
  assert.equal(names("reset")[0], "new");
});

test("tier order holds within the name group and within the description group", () => {
  for (const query of ["", "s", "e", "r", "o", "work", "new", "your"]) {
    const matched = matchSlashCommands(query);
    const byName = matched.filter((command) => nameMatches(command, query));
    const byDescription = matched.filter((command) => !nameMatches(command, query));
    assert.deepEqual(matched, [...byName, ...byDescription], query);
    for (const group of [byName, byDescription]) {
      const ranks = group.map((command) => tierRank(command.tier));
      assert.deepEqual(ranks, [...ranks].sort((a, b) => a - b), query);
    }
  }
});

test("restricted and unlisted commands are never matched", () => {
  const hidden = SLASH_COMMANDS.filter((command) => !LISTED.has(command.tier));
  assert.ok(hidden.some((command) => command.tier === "unlisted"));
  for (const command of hidden) {
    for (const name of [command.name, ...command.aliases]) {
      assert.ok(
        matchSlashCommands(name).every((match) => LISTED.has(match.tier)),
        name
      );
      assert.ok(!names(name).includes(command.name), name);
    }
  }
  assert.deepEqual(names("update"), []);
  assert.ok(!names("app").includes("approve"));
  assert.ok(!names("sess").includes("sessions"));
});

test("every name and alias resolves to its own catalog entry", () => {
  const seen = new Map<string, string>();
  for (const command of SLASH_COMMANDS) {
    for (const name of [command.name, ...command.aliases]) {
      const key = name.toLowerCase().replaceAll("_", "-");
      const owner = seen.get(key);
      assert.ok(!owner || owner === command.name, `${name} is claimed by ${owner} and ${command.name}`);
      seen.set(key, command.name);
      assert.equal(findSlashCommand(name), command, name);
      assert.equal(findSlashCommand(name.toUpperCase()), command, name);
    }
  }
  assert.equal(findSlashCommand("set_home")?.name, "sethome");
  assert.equal(findSlashCommand("reload-mcp")?.name, "reload-mcp");
  assert.equal(findSlashCommand("reload_mcp")?.name, "reload-mcp");
  assert.equal(findSlashCommand("tasks")?.name, "agents");
  assert.equal(findSlashCommand("nonexistent"), null);
});

test("shown and restricted commands carry the copy the picker needs", () => {
  for (const command of SLASH_COMMANDS) {
    if (LISTED.has(command.tier)) assert.ok(command.description, command.name);
    const needsReason = command.tier === "restricted" || command.tier === "not_recommended";
    assert.equal(Boolean(command.reason), needsReason, command.name);
  }
});

test("user-facing catalog copy has no em dashes", () => {
  for (const command of SLASH_COMMANDS) {
    for (const text of [command.description, command.reason ?? "", command.args ?? ""]) {
      assert.ok(!text.includes("\u2014"), command.name);
    }
  }
});

test("busy follows Hermes: reject means refused while the agent works", () => {
  const busy = (name: string) => findSlashCommand(name)?.busy;
  assert.equal(busy("new"), "interrupt_then_dispatch");
  assert.equal(busy("stop"), "interrupt_then_dispatch");
  for (const name of ["steer", "btw", "queue", "status", "context", "bg", "goal", "pause", "heartbeat"]) {
    assert.equal(busy(name), "dispatch", name);
  }
  for (const name of ["retry", "compress", "usage", "plan", "learn", "reasoning", "personality", "model", "undo", "reload-skills"]) {
    assert.equal(busy(name), "reject", name);
  }
  assert.equal(busy("yes"), null, "confirm replies are not registry commands");
});

test("restrictedSlashCommand recognizes restricted commands the way Hermes parses them", () => {
  for (const text of [
    "/update",
    "/UPDATE",
    "/Update now please",
    "/update@finite_bot",
    "  /restart",
    "/debug\nmore",
    "/codex_runtime auto",
    "/codex-runtime",
    "/CODEX_RUNTIME",
    "/approvals off",
    "/yolo",
    "/topup",
    "/save md",
    "/help",
    "/commands 2",
    "/title Lisbon trip",
    "/loop 5m check the build",
    "/proactive",
    "/memory pending",
    "/skills",
    "/rollback 2",
    "/reload_mcp",
    "/sethome",
    "/set-home",
    "/set_home",
    "/suggest",
    "/bp daily",
    "/busy queue",
    "/whoami",
  ]) {
    assert.ok(restrictedSlashCommand(text), JSON.stringify(text));
  }
  assert.equal(restrictedSlashCommand("/codex_runtime")?.name, "codex-runtime");
  assert.equal(restrictedSlashCommand("/proactive")?.name, "loop");
  assert.equal(
    restrictedSlashCommand("/update")?.reason,
    "Finite manages your agent's software and restarts."
  );
  assert.equal(
    restrictedSlashCommand("/help")?.reason,
    "Type / in the message box to see the commands you can use."
  );
  assert.equal(
    restrictedSlashCommand("/verbose")?.reason,
    "This command isn't available in Finite chat."
  );
});

test("the blocked message names the canonical command and its reason", () => {
  const command = restrictedSlashCommand("/update@finite_bot now");
  assert.ok(command);
  assert.equal(
    restrictedSlashCommandMessage(command),
    "/update isn't available in Finite. Finite manages your agent's software and restarts."
  );
  const alias = restrictedSlashCommand("/proactive");
  assert.ok(alias);
  assert.equal(
    restrictedSlashCommandMessage(alias),
    "/loop isn't available in Finite. Recurring command loops aren't available in Finite chat."
  );
});

test("ordinary text, shown commands, and unlisted commands are never restricted", () => {
  for (const text of [
    "",
    "/",
    "update",
    "please /update",
    "Can you /restart the server?",
    "https://example.com/update",
    "/update/notes.md",
    "/updates",
    "/new",
    "/reset",
    "/queue /update",
    "/model",
    "/pause",
    "/undo 2",
    "/hb status",
    "/approve",
    "/deny",
    "/always",
    "/cancel",
    "/yes",
    "/no",
    "/resume",
    "/sessions",
    "/fork",
    "/tasks",
    "/somethingelse",
    "a/update",
  ]) {
    assert.equal(restrictedSlashCommand(text), null, JSON.stringify(text));
  }
});

test("slashArgsHint shows the argument signature only right after insertion", () => {
  assert.equal(slashArgsHint("/queue "), "<prompt>");
  assert.equal(slashArgsHint("/q "), "<prompt>");
  assert.equal(slashArgsHint("/new "), "[name]");
  assert.equal(slashArgsHint("/model "), "[model]");
  assert.equal(slashArgsHint("/stop "), null);
  assert.equal(slashArgsHint("/queue"), null);
  assert.equal(slashArgsHint("/queue x"), null);
  assert.equal(slashArgsHint("/queue  "), null);
  assert.equal(slashArgsHint("/update "), null);
  assert.equal(slashArgsHint("/resume "), null);
  assert.equal(slashArgsHint("hello "), null);
});

function nameMatches(command: SlashCommand, query: string) {
  const needle = query.toLowerCase();
  return [command.name, ...command.aliases].some((name) => name.startsWith(needle));
}

function tierRank(tier: string) {
  return ["suggested", "available", "not_recommended", "restricted", "unlisted"].indexOf(tier);
}
