import assert from "node:assert/strict";
import { test } from "node:test";
import {
  AgentExportError, defaultChoices, deleteExport, estimatedBytes, exportErrorMessage, exportPagePath,
  formatBytes, isExportCommand, parseExportStatus, readExportStatus, startExport, startSurvey,
} from "./agent-export";
import { HostedHermesStatusError, type HostedHermesChange } from "./hosted-hermes-status";

const summary = {
  conversations: 214, memories: 2, skills: 3, scheduledJobs: 2, files: 5, fileBytes: 38 * 1024 * 1024,
  brains: [
    { id: "personal-0123456789abcdef", name: "Moss notes", kind: "personal", bytes: 31 },
    { id: "team-garden", name: "Team", kind: "organization", bytes: null },
  ],
  sites: [
    { slug: "recipes", url: "https://recipes.finite.site", owned: true },
    { slug: "team-site", url: "https://team-site.finite.site", owned: false },
  ],
  problems: 0,
};
const ready = {
  state: "ready", startedAt: "2026-10-29T18:00:00Z", finishedAt: "2026-10-29T18:04:00Z",
  expiresAt: "2026-11-05T18:04:00Z", bytes: 39_000_000, fileName: "finite-export-moss-20261029-1804.zip", problems: 1,
};
const status = {
  version: 1,
  survey: { state: "ready", startedAt: "2026-10-29T17:59:00Z", finishedAt: "2026-10-29T17:59:05Z", summary },
  export: ready,
};

type Sent = { path: string; change: HostedHermesChange };
function agent(responses: Array<{ status: number; body: unknown } | Error>) {
  const sent: Sent[] = [];
  const read: string[] = [];
  return {
    sent, read,
    dependencies: {
      send: async (_runtime: string, path: string, change: HostedHermesChange) => {
        sent.push({ path, change });
        const next = responses.shift();
        if (!next) throw new Error("unexpected change");
        if (next instanceof Error) throw next;
        return next;
      },
      read: async (_runtime: string, path: string) => {
        read.push(path);
        return status;
      },
    },
  };
}
const signal = () => new AbortController().signal;

test("the agent's status parses whole, and anything unexpected reads as an agent that needs an update", async () => {
  const parsed = parseExportStatus(status);
  assert.equal(parsed.survey?.summary?.brains[1].kind, "organization");
  assert.equal(parsed.export?.fileName, ready.fileName);
  assert.deepEqual(parseExportStatus({ version: 1, survey: null, export: null }), { survey: null, export: null });
  const running = parseExportStatus({ version: 1, survey: null, export: { state: "running", startedAt: ready.startedAt, finishedAt: null } });
  assert.deepEqual(running.export, {
    state: "running", startedAt: ready.startedAt, finishedAt: null, expiresAt: null, bytes: null, fileName: null, problems: null,
  });
  const fixture = agent([]);
  assert.equal((await readExportStatus("runtime", signal(), fixture.dependencies)).export?.bytes, ready.bytes);
  assert.deepEqual(fixture.read, ["api/plugins/finite-export/status"]);
  for (const broken of [
    { ...status, version: 2 },
    { ...status, export: { ...ready, state: "done" } },
    { ...status, export: { ...ready, fileName: "../../etc/passwd" } },
    { ...status, export: { ...ready, bytes: -1 } },
    { ...status, survey: { ...status.survey, summary: { ...summary, brains: [{ ...summary.brains[0], id: "--out" }] } } },
    { ...status, survey: { ...status.survey, summary: { ...summary, sites: [{ slug: "recipes", url: null }] } } },
    "not an object",
  ]) {
    assert.throws(() => parseExportStatus(broken), HostedHermesStatusError);
  }
  const insecure = parseExportStatus({ ...status, survey: { ...status.survey, summary: {
    ...summary, sites: [{ slug: "recipes", url: "http://recipes.finite.site", owned: true }],
  } } });
  assert.equal(insecure.survey?.summary?.sites[0].url, null);
});

test("starts and deletes are sent once to the plugin's fixed routes", async () => {
  const fixture = agent([{ status: 202, body: {} }, { status: 202, body: {} }, { status: 200, body: { version: 1 } }]);
  await startSurvey("runtime", signal(), fixture.dependencies);
  await startExport("runtime", { brains: ["personal-0123456789abcdef"], sites: ["recipes"] }, signal(), fixture.dependencies);
  await deleteExport("runtime", signal(), fixture.dependencies);
  assert.deepEqual(fixture.sent, [
    { path: "api/plugins/finite-export/survey", change: { method: "POST" } },
    { path: "api/plugins/finite-export/export", change: { method: "POST", body: { brains: ["personal-0123456789abcdef"], sites: ["recipes"] } } },
    { path: "api/plugins/finite-export/export", change: { method: "DELETE" } },
  ]);
  const refused = agent([]);
  await assert.rejects(startExport("runtime", { brains: ["--out=/tmp"], sites: [] }, signal(), refused.dependencies),
    (error: unknown) => error instanceof AgentExportError && error.code === "notOffered");
  assert.equal(refused.sent.length, 0);
});

test("each answer maps to one plain outcome", async () => {
  const cases: Array<[{ status: number; body: unknown } | Error, string]> = [
    [{ status: 409, body: { error: "busy" } }, "busy"],
    [{ status: 409, body: { error: "surveyFirst" } }, "surveyFirst"],
    [{ status: 422, body: { error: "notOffered" } }, "notOffered"],
    [{ status: 507, body: { error: "notEnoughSpace" } }, "notEnoughSpace"],
    [{ status: 404, body: null }, "unavailable"],
    [{ status: 405, body: null }, "unavailable"],
    [{ status: 401, body: null }, "access"],
    [{ status: 500, body: { error: "private diagnostics" } }, "unknown"],
    [new HostedHermesStatusError("Hosted access is unavailable for this account or agent.", "access"), "access"],
    [new Error("socket hang up"), "unknown"],
  ];
  for (const [answer, code] of cases) {
    const fixture = agent([answer]);
    await assert.rejects(startSurvey("runtime", signal(), fixture.dependencies),
      (error: unknown) => error instanceof AgentExportError && error.code === code, code);
    assert.equal(fixture.sent.length, 1);
  }
  for (const code of ["busy", "surveyFirst", "notOffered", "notEnoughSpace", "unavailable", "access", "unknown"] as const) {
    assert.ok(exportErrorMessage(code, "Moss").length > 0);
  }
  assert.match(exportErrorMessage("notEnoughSpace", "Moss"), /Moss’s computer/u);
});

test("the person's own brains and sites start chosen; the estimate follows the choice", () => {
  const parsed = parseExportStatus(status).survey!.summary!;
  const choices = defaultChoices(parsed);
  assert.deepEqual(choices, { brains: ["personal-0123456789abcdef"], sites: ["recipes"] });
  assert.equal(estimatedBytes(parsed, choices), 38 * 1024 * 1024 + 31);
  assert.equal(estimatedBytes(parsed, { brains: ["team-garden"], sites: [] }), 38 * 1024 * 1024);
  assert.equal(formatBytes(512), "512 B");
  assert.equal(formatBytes(1536), "1.5 KB");
  assert.equal(formatBytes(38 * 1024 * 1024), "38 MB");
  assert.equal(formatBytes(5.5 * 1024 ** 3), "5.5 GB");
});

test("only /export on its own opens the Export page", () => {
  assert.equal(isExportCommand("/export"), true);
  assert.equal(isExportCommand("  /export \n"), true);
  for (const text of ["/export my notes", "/exports", "export", "please /export"]) assert.equal(isExportCommand(text), false);
  assert.equal(exportPagePath("runtime/1"), "/dashboard/machines/runtime%2F1/export");
});
