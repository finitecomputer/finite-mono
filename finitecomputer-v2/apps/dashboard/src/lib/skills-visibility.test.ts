import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("Skills navigation and catalog are available to ordinary dashboard users", async () => {
  const [agentNavigationSource, dashboardShellSource, skillsPageSource] = await Promise.all([
    readFile(new URL("../components/agent-navigation.tsx", import.meta.url), "utf8"),
    readFile(new URL("../components/dashboard-shell.tsx", import.meta.url), "utf8"),
    readFile(new URL("../app/dashboard/skills/page.tsx", import.meta.url), "utf8"),
  ]);

  assert.match(agentNavigationSource, /href: `\/dashboard\/skills\?machine=/u);
  assert.match(dashboardShellSource, /label: "Skills",[\s\S]*href: skillsHref/u);

  for (const source of [agentNavigationSource, dashboardShellSource, skillsPageSource]) {
    assert.doesNotMatch(source, /showSkills/u);
  }
  assert.doesNotMatch(skillsPageSource, /viewer\.isAdmin|redirect\("\/dashboard"\)/u);
});
