import assert from "node:assert/strict";
import test from "node:test";
import { groupAgentSkills, parseAgentSkills as parseInventory, readAgentSkills } from "./agent-skills";
import { HostedHermesStatusError } from "./hosted-hermes-status";

const parseAgentSkills = (skills: unknown) => parseInventory({ inventory_version: 1, skills });

const entry = { name: "zulu", description: "Research papers", category: "research", enabled: true };

test("native inventory validates every entry, includes disabled skills and drops private fields", () => {
  assert.deepEqual(parseAgentSkills([{ ...entry, enabled: false, category: null, path: "/private", content: "private" }]),
    [{ ...entry, enabled: false, category: "general" }]);
  assert.deepEqual(parseAgentSkills([]), []);
  for (const invalid of [{ skills: [] }, null, [entry, { ...entry }], [{ ...entry, enabled: "true" }],
    [{ ...entry, name: " " }], [{ ...entry, description: null }], [{ ...entry, category: {} }],
    [entry, {}], Array(4001).fill(entry)]) assert.throws(() => parseAgentSkills(invalid));
});

test("categories and skills sort alphabetically; search uses descriptions and readable categories", () => {
  const skills = parseAgentSkills([entry,
    { ...entry, name: "alpha", category: "software-development" },
    { ...entry, name: "beta", category: null },
    { ...entry, name: "aardvark" },
  ]);
  assert.deepEqual(groupAgentSkills(skills, "").map(g => [g.category, g.skills.map(s => s.name)]),
    [["general", ["beta"]], ["research", ["aardvark", "zulu"]], ["software-development", ["alpha"]]]);
  assert.equal(groupAgentSkills(skills, "software development")[0].skills[0].name, "alpha");
  assert.equal(groupAgentSkills(skills, " PAPERS ").length, 3);
  assert.deepEqual(groupAgentSkills(skills, "no match"), []);
});

test("skills uses only the fixed native read after an owner grant", async (t) => {
  const calls: { url: string; method: string | undefined }[] = [];
  t.mock.method(globalThis, "fetch", async (url: string | URL, init: RequestInit) => {
    calls.push({ url: String(url), method: init.method });
    return calls.length === 1
      ? Response.json({ baseUrl: "https://agent.test/a/", accessToken: "synthetic", expiresAt: 100 })
      : Response.json({ inventory_version: 1, skills: [entry] });
  });
  assert.deepEqual(await readAgentSkills("a", new AbortController().signal), [entry]);
  assert.deepEqual(calls, [{ url: "/api/agents/a/hermes-access", method: "POST" }, { url: "https://agent.test/a/api/skills?inventory=true", method: undefined }]);
});

test("access loss and unsupported endpoints are distinguishable from transient failures", async (t) => {
  let phase: "control" | "native" = "control";
  let status = 403;
  t.mock.method(globalThis, "fetch", async (url: string | URL) => {
    if (phase === "native" && String(url).startsWith("/api/agents/")) {
      return Response.json({ baseUrl: "https://agent.test/a/", accessToken: "synthetic", expiresAt: 100 });
    }
    return new Response("private upstream diagnostics", { status });
  });
  for (const source of ["control", "native"] as const) {
    phase = source;
    for (const [code, kind] of [[401, "access"], [403, "access"], [404, source === "native" ? "unsupported" : "access"], [503, "request"]] as const) {
      status = code;
      await assert.rejects(readAgentSkills("a", new AbortController().signal), error => {
        assert(error instanceof HostedHermesStatusError);
        assert.equal(error.kind, kind);
        assert(!error.message.includes("private upstream"));
        return true;
      });
    }
  }
});

test("a late native body cannot survive agent-switch cancellation", async (t) => {
  const controller = new AbortController();
  let calls = 0;
  t.mock.method(globalThis, "fetch", async () => {
    if (++calls === 1) return Response.json({ baseUrl: "https://agent.test/a/", accessToken: "synthetic", expiresAt: 100 });
    controller.abort();
    return Response.json({ inventory_version: 1, skills: [entry] });
  });
  await assert.rejects(readAgentSkills("a", controller.signal));
});

test("older runtimes cannot silently return an incomplete legacy list", () => {
  for (const response of [[], [entry], {}, { inventory_version: 2, skills: [entry] }]) {
    assert.throws(() => parseInventory(response), (error: unknown) => error instanceof HostedHermesStatusError && error.kind === "unsupported");
  }
});
