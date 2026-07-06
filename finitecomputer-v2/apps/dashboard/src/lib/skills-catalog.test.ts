import assert from "node:assert/strict";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

import { loadBaselineSkillsCatalog } from "./skills-catalog";

test("skills catalog can load from the GitHub-style finite-skills checkout", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "finite-skills-catalog-"));
  const previousSourceDir = process.env.FC_FINITE_SKILLS_SOURCE_DIR;

  try {
    const skillDir = path.join(root, "skills", "browser", "ui-polish");
    await mkdir(skillDir, { recursive: true });
    await writeFile(
      path.join(skillDir, "SKILL.md"),
      [
        "---",
        "name: ui-polish",
        "description: Tighten dashboard UI details.",
        "---",
        "",
        "Use this skill for UI polish.",
      ].join("\n"),
      "utf8"
    );

    process.env.FC_FINITE_SKILLS_SOURCE_DIR = root;

    const catalog = await loadBaselineSkillsCatalog();

    assert.equal(catalog.totalSkillCount, 1);
    assert.equal(catalog.categoryCount, 1);
    assert.equal(catalog.skills[0]?.name, "ui-polish");
    assert.equal(catalog.skills[0]?.category, "Browser");
    assert.equal(catalog.skills[0]?.managedRelpath, ".hermes/skills/browser/ui-polish");
  } finally {
    if (previousSourceDir === undefined) {
      delete process.env.FC_FINITE_SKILLS_SOURCE_DIR;
    } else {
      process.env.FC_FINITE_SKILLS_SOURCE_DIR = previousSourceDir;
    }
    await rm(root, { force: true, recursive: true });
  }
});

test("skills catalog enumerates finite-skills through GitHub and fetches skill bodies as raw files", async () => {
  const previousSourceDir = process.env.FC_FINITE_SKILLS_SOURCE_DIR;
  const previousRepo = process.env.FC_FINITE_SKILLS_GITHUB_REPO;
  const previousRef = process.env.FC_FINITE_SKILLS_REF;
  const previousRepoRoot = process.env.FC_REPO_ROOT;
  const previousFetch = globalThis.fetch;
  const seenUrls: string[] = [];

  try {
    delete process.env.FC_FINITE_SKILLS_SOURCE_DIR;
    process.env.FC_REPO_ROOT = rootWithoutSiblingFiniteSkills();
    process.env.FC_FINITE_SKILLS_GITHUB_REPO = "finitecomputer/finite-skills-test";
    process.env.FC_FINITE_SKILLS_REF = "catalog-test";
    globalThis.fetch = (async (input: RequestInfo | URL) => {
      const url = String(input);
      seenUrls.push(url);

      if (url === "https://api.github.com/repos/finitecomputer/finite-skills-test/git/trees/catalog-test?recursive=1") {
        return Response.json({
          tree: [
            { path: "README.md", type: "blob" },
            { path: "skills/browser/ui-polish/SKILL.md", type: "blob" },
            { path: "skills/productivity/google-workspace-finite/SKILL.md", type: "blob" },
            { path: "skills/productivity/google-workspace-finite/references/setup.md", type: "blob" },
          ],
        });
      }

      assert.match(
        url,
        /^https:\/\/raw\.githubusercontent\.com\/finitecomputer\/finite-skills-test\/catalog-test\/skills\/.+\/SKILL\.md$/
      );
      return new Response(
        [
          "---",
          `name: ${path.basename(path.dirname(url))}`,
          "description: Loaded from raw GitHub.",
          "---",
          "",
          "Raw GitHub fallback.",
        ].join("\n"),
        { status: 200, headers: { "content-type": "text/plain" } }
      );
    }) as typeof fetch;

    const catalog = await loadBaselineSkillsCatalog();

    assert.equal(catalog.totalSkillCount, 2);
    assert.equal(seenUrls.filter((url) => url.includes("api.github.com")).length, 1);
    assert.equal(seenUrls.filter((url) => url.includes("raw.githubusercontent.com")).length, 2);
  } finally {
    if (previousSourceDir === undefined) {
      delete process.env.FC_FINITE_SKILLS_SOURCE_DIR;
    } else {
      process.env.FC_FINITE_SKILLS_SOURCE_DIR = previousSourceDir;
    }
    if (previousRepo === undefined) {
      delete process.env.FC_FINITE_SKILLS_GITHUB_REPO;
    } else {
      process.env.FC_FINITE_SKILLS_GITHUB_REPO = previousRepo;
    }
    if (previousRef === undefined) {
      delete process.env.FC_FINITE_SKILLS_REF;
    } else {
      process.env.FC_FINITE_SKILLS_REF = previousRef;
    }
    if (previousRepoRoot === undefined) {
      delete process.env.FC_REPO_ROOT;
    } else {
      process.env.FC_REPO_ROOT = previousRepoRoot;
    }
    globalThis.fetch = previousFetch;
  }
});

function rootWithoutSiblingFiniteSkills() {
  return path.join(tmpdir(), "finitecomputer-v2-test-root");
}
