import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import { prepareFixture } from "./fixture.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const fixtureFile = path.join(root, "fixtures/coffee-shop-homepage.json");

test("A/B plans preserve both skill inputs and earlier runs", (t) => {
  const dir = mkdtempSync(path.join(tmpdir(), "finite-fixture-test-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const candidateFile = path.join(dir, "candidate.md");
  writeFileSync(candidateFile, "---\nname: website-building-finite\n---\nA small candidate change.\n");
  const first = prepareFixture({ fixtureFile, candidateFile, runsDir: dir });
  const second = prepareFixture({ fixtureFile, candidateFile, runsDir: dir });
  assert.notEqual(first.runDir, second.runDir);
  assert.notEqual(first.plan.variants[0].sha256, first.plan.variants[1].sha256);
  writeFileSync(candidateFile, "Edited after preparation");
  assert.match(readFileSync(first.plan.variants[1].skillPath, "utf8"), /A small candidate change/);
  const config = JSON.parse(readFileSync(first.configPath, "utf8"));
  assert.equal(config.providers.length, 2);
  assert.equal(config.tests[0].vars.brief, first.plan.fixture.brief);
  for (const provider of config.providers) {
    assert.equal(provider.config.runner, "devfinity");
    assert.ok(provider.config.skillPath.startsWith(first.runDir));
    assert.ok(provider.config.outputDir.startsWith(first.runDir));
    assert.ok(readFileSync(path.join(path.dirname(provider.config.skillPath), "references/shared/01-design-tokens.md"), "utf8"));
  }
});

test("a plan without a candidate is explicitly an A/A control", (t) => {
  const dir = mkdtempSync(path.join(tmpdir(), "finite-fixture-test-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const { plan } = prepareFixture({ fixtureFile, runsDir: dir });
  assert.match(plan.comparison, /A\/A control/);
  assert.equal(plan.variants[0].sha256, plan.variants[1].sha256);
  assert.equal(plan.status, "prepared");
});

test("rejects invalid fixtures before preparing a run", (t) => {
  const dir = mkdtempSync(path.join(tmpdir(), "finite-fixture-test-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const invalidFile = path.join(dir, "invalid.json");
  const fixture = JSON.parse(readFileSync(fixtureFile, "utf8"));
  for (const patch of [{ id: "../escape" }, { version: 2 }, { requiredVisibleText: [] }, { reviewCriteria: [null] }]) {
    writeFileSync(invalidFile, JSON.stringify({ ...fixture, ...patch }));
    assert.throws(() => prepareFixture({ fixtureFile: invalidFile, runsDir: dir }));
  }
});
