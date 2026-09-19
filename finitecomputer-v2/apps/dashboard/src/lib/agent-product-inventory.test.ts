import assert from "node:assert/strict";
import { test } from "node:test";
import { parseBrainInventory, parseSitesInventory } from "./agent-product-inventory";
import { HostedHermesStatusError } from "./hosted-hermes-status";

const brain = { id: "org", name: "Organization", kind: "organization", role: "guest", folders: [{ id: "folder", name: "Shared folder" }] };
const site = { id: "project", name: "Project site", url: "https://project.finite.site", visibility: "shared", status: "published", published: true, canEdit: true, repositoryUrl: "https://finite.site/project.git" };
const incompatible = (action: () => unknown) => assert.throws(action, error => error instanceof HostedHermesStatusError && error.kind === "unsupported");

test("Brain inventory separates invitations, projects only metadata, and never turns unavailable folders into zero", () => {
  const parsed = parseBrainInventory({ version: 1, brains: [brain,
    { ...brain, id: "pending", role: "invited", folders: null, inviteCode: "must-not-project" },
    { ...brain, id: "missing-details", role: "member", folders: null },
  ] });
  assert.equal(parsed.brains.filter(item => !item.pending).length, 2);
  assert.equal(parsed.brains.find(item => item.id === "org")?.role, "Guest");
  assert.equal(parsed.brains.find(item => item.id === "missing-details")?.folders, null);
  assert(!JSON.stringify(parsed).includes("must-not-project"));
  incompatible(() => parseBrainInventory({ version: 1, brains: [brain, brain] }));
  incompatible(() => parseBrainInventory({ version: 2, brains: [] }));
  incompatible(() => parseBrainInventory({ version: 1, brains: [{ ...brain, role: "invited" }] }));
  incompatible(() => parseBrainInventory({ version: 1, brains: [{ ...brain, role: "constructor" }] }));
});

test("Sites reads preserve actual publishing/edit metadata without invented dates or thumbnails", () => {
  const parsed = parseSitesInventory({ version: 1, sites: [site, { ...site, id: "draft", published: false, status: "claimed_unpublished", canEdit: false }], sourceOnlyProjects: 2 });
  assert.equal(parsed.sourceOnlyProjects, 2);
  assert.equal(parsed.sites.find(item => item.id === "project")?.published, true);
  const draft = parsed.sites.find(item => item.id === "draft")!;
  assert.equal(draft.statusLabel, "Not published");
  assert.equal(draft.canEdit, false);
  assert.equal(draft.updatedLabel, undefined);
  assert.equal(draft.thumbnailUrl, undefined);
  const disabled = parseSitesInventory({ version: 1, sites: [{ ...site, status: "disabled" }], sourceOnlyProjects: 0 }).sites[0];
  assert.equal(disabled.published, false);
  for (const url of ["javascript:alert(1)", "http://example.com", "https://user:password@example.com"]) {
    incompatible(() => parseSitesInventory({ version: 1, sites: [{ ...site, url }], sourceOnlyProjects: 0 }));
  }
  incompatible(() => parseSitesInventory({ version: 1, sites: [site, site], sourceOnlyProjects: 0 }));
  incompatible(() => parseSitesInventory({ version: 1, sites: [], sourceOnlyProjects: -1 }));
});
