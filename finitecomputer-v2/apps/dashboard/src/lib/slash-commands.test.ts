import assert from "node:assert/strict";
import test from "node:test";

import {
  SLASH_COMMANDS,
  findSlashCommand,
  isPastedInput,
  keepSlashDismissed,
  matchSlashCommands,
  restrictedSlashCommand,
  restrictedSlashCommandMessage,
  slashArgsHint,
  slashEnterInserts,
  slashPickerKeyAction,
  slashPickerVisible,
  slashQuery,
  type SlashCommand,
} from "@/lib/slash-commands";

const names = (query: string) => matchSlashCommands(query).map((command) => command.name);
const LISTED = new Set(["suggested", "available", "not_recommended"]);
// Every code point Python 3.13 str.isspace() accepts; Hermes strips and
// splits commands with it.
const PYTHON_WHITESPACE = [
  0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x85, 0xa0, 0x1680,
  0x2000, 0x2001, 0x2002, 0x2003, 0x2004, 0x2005, 0x2006, 0x2007, 0x2008, 0x2009,
  0x200a, 0x2028, 0x2029, 0x202f, 0x205f, 0x3000,
].map((code) => String.fromCodePoint(code));
const hex = (text: string) => JSON.stringify(text).replace(/[^\x20-\x7e]/g, (c) =>
  `\\u${c.charCodeAt(0).toString(16).padStart(4, "0")}`
);

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

test("restricted detection uses Python whitespace to strip and split", () => {
  for (const space of PYTHON_WHITESPACE) {
    assert.equal(restrictedSlashCommand(`${space}/debug`)?.name, "debug", hex(`${space}/debug`));
    assert.equal(
      restrictedSlashCommand(`/debug${space}anything`)?.name,
      "debug",
      hex(`/debug${space}anything`)
    );
    assert.equal(
      restrictedSlashCommand(`${space}${space}/update@bot${space}now`)?.name,
      "update",
      hex(space)
    );
    assert.equal(restrictedSlashCommand(`/new${space}/update`), null, hex(space));
  }
  // U+FEFF is whitespace to JavaScript but not to Python, so Hermes does not
  // treat these as the debug command either.
  assert.equal(restrictedSlashCommand("\ufeff/debug"), null);
  assert.equal(restrictedSlashCommand("/debug\ufeffanything"), null);
});

test("slashQuery ends the command token at Python whitespace", () => {
  for (const space of PYTHON_WHITESPACE) {
    assert.equal(slashQuery(`/qu${space}`), null, hex(space));
    assert.equal(slashQuery(`${space}/qu`), null, hex(space));
  }
});

test("picker keys do nothing while an IME is composing", () => {
  const open = { open: true, navigable: true, blocked: false, enterInserts: true };
  for (const key of ["ArrowDown", "ArrowUp", "Enter", "Tab", "Escape"]) {
    assert.equal(
      slashPickerKeyAction({ key, shiftKey: false, isComposing: true, keyCode: 0 }, open),
      null,
      key
    );
    assert.equal(
      slashPickerKeyAction({ key, shiftKey: false, isComposing: false, keyCode: 229 }, open),
      null,
      key
    );
  }
});

test("picker keys navigate, insert, and dismiss only when the picker can use them", () => {
  type Picker = Parameters<typeof slashPickerKeyAction>[1];
  const press = (key: string, picker: Picker, shiftKey = false) =>
    slashPickerKeyAction({ key, shiftKey, isComposing: false, keyCode: 0 }, picker);
  const choosing = { open: true, navigable: true, blocked: false, enterInserts: true };
  const typedOut = { open: true, navigable: true, blocked: false, enterInserts: false };
  const empty = { open: true, navigable: false, blocked: false, enterInserts: false };
  const blocked = { open: true, navigable: false, blocked: true, enterInserts: false };
  const closed = { open: false, navigable: false, blocked: false, enterInserts: false };

  assert.equal(press("ArrowDown", choosing), "next");
  assert.equal(press("ArrowUp", choosing), "previous");
  assert.equal(press("Enter", choosing), "insert");
  assert.equal(press("Tab", choosing), "insert");
  assert.equal(press("Escape", choosing), "dismiss");
  assert.equal(press("Enter", choosing, true), null, "Shift+Enter keeps its newline");
  assert.equal(press("Tab", choosing, true), null);
  assert.equal(press("a", choosing), null);

  assert.equal(press("Enter", typedOut), null, "Enter sends what was typed");
  assert.equal(press("Tab", typedOut), "insert", "Tab still inserts");
  assert.equal(press("ArrowDown", typedOut), "next");

  assert.equal(press("Escape", empty), "dismiss");
  assert.equal(press("Enter", empty), null, "no match: Enter sends as before");
  assert.equal(press("ArrowDown", empty), null);

  assert.equal(press("Escape", blocked), null, "the blocked reason stays while Send is disabled");
  assert.equal(press("Enter", blocked), null);

  for (const key of ["Escape", "Enter", "Tab", "ArrowDown"]) {
    assert.equal(press(key, closed), null, key);
  }
});

test("Enter inserts only when the person is clearly choosing a command", () => {
  const first = (query: string) => matchSlashCommands(query)[0];
  const inserts = (query: string, moved = false, highlighted = first(query)) =>
    slashEnterInserts(query, highlighted, moved);

  // A fully typed listed name or alias sends, as before the picker.
  for (const query of ["stop", "STOP", "new", "status", "q", "ctx", "reload_skills", "reload-skills", "hb"]) {
    assert.equal(inserts(query), false, query);
  }
  // Name and alias prefixes insert the highlighted command.
  for (const query of ["", "st", "sto", "mo", "reload_", "que"]) {
    assert.equal(inserts(query), true, query);
  }
  // Description-only matches never rewrite what was typed.
  for (const query of ["no", "ok", "summarize", "schedule", "session", "chat"]) {
    const highlighted = first(query);
    assert.ok(highlighted, `${query} has a description match`);
    assert.ok(!nameMatches(highlighted, query), query);
    assert.equal(inserts(query), false, query);
  }
  // Moving the highlight with the arrows is a choice, even on an exact match.
  assert.equal(inserts("no", true), true);
  assert.equal(inserts("stop", true, findSlashCommand("steer") ?? undefined), true);
  assert.equal(inserts("zzz", false, undefined), false);
});

test("a pasted slash draft keeps the picker closed until the person edits it", () => {
  const typed = { dismissed: false, pastedDraft: null };
  assert.equal(slashPickerVisible("/qu", typed), true);
  assert.equal(slashPickerVisible("/qu", { dismissed: false, pastedDraft: "/qu" }), false);
  assert.equal(slashPickerVisible("/que", { dismissed: false, pastedDraft: "/qu" }), true);
  assert.equal(slashPickerVisible("/qu", { dismissed: true, pastedDraft: null }), false);
  assert.equal(slashPickerVisible("hello", typed), false);
  // A pasted restricted command still explains why Send is disabled.
  assert.equal(slashPickerVisible("/update", { dismissed: false, pastedDraft: "/update" }), true);
  assert.equal(slashPickerVisible("/update now", typed), true);
});

test("the blocked explanation cannot be dismissed", () => {
  assert.equal(slashPickerVisible("/update", { dismissed: true, pastedDraft: null }), true);
  assert.equal(slashPickerVisible("/help me", { dismissed: true, pastedDraft: null }), true);
});

test("unlisted commands send without the picker", () => {
  const typed = { dismissed: false, pastedDraft: null };
  for (const draft of ["/approve", "/deny", "/always", "/cancel", "/yes", "/no", "/YES", "/resume", "/fork", "/tasks"]) {
    assert.equal(slashPickerVisible(draft, typed), false, draft);
  }
  // Prefixes of unlisted names and unknown commands still get the picker.
  assert.equal(slashPickerVisible("/appr", typed), true);
  assert.equal(slashPickerVisible("/summarize", typed), true);
});

test("Esc stays dismissed only while the person keeps extending the same slash query", () => {
  assert.equal(keepSlashDismissed("/mo", "/mod"), true);
  assert.equal(keepSlashDismissed("/mo", "/mo"), true);
  assert.equal(keepSlashDismissed("/mo", "/m"), false, "shortened");
  assert.equal(keepSlashDismissed("/mo", "/"), false, "back to /");
  assert.equal(keepSlashDismissed("/mo", ""), false, "cleared");
  assert.equal(keepSlashDismissed("/mo", "/mo "), false, "no longer a slash query");
  assert.equal(keepSlashDismissed("/mo", "/xy"), false, "replaced");
  assert.equal(keepSlashDismissed("", "/"), false, "a fresh slash after a send");
  assert.equal(keepSlashDismissed("/help me", "/"), false);
});

test("isPastedInput recognizes clipboard and drop insertions only", () => {
  for (const inputType of ["insertFromPaste", "insertFromPasteAsQuotation", "insertFromDrop"]) {
    assert.equal(isPastedInput(inputType), true, inputType);
  }
  for (const inputType of ["insertText", "insertReplacementText", "deleteContentBackward", "insertCompositionText", "", undefined]) {
    assert.equal(isPastedInput(inputType), false, String(inputType));
  }
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
  return ["suggested", "available", "not_recommended", "unlisted", "restricted"].indexOf(tier);
}
