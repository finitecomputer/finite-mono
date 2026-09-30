import assert from "node:assert/strict";
import path from "node:path";

export type RuntimeProof = {
  project: string; runtime: string; principal: string; room: string; topic: string; chat: string;
  fileHash: string; messageIds: string[]; running: boolean; startedAt: string;
};

export function assertRecovered(before: RuntimeProof, stopped: RuntimeProof, after: RuntimeProof) {
  assert(before.running && !stopped.running && after.running, "physical running/stopped/running sequence required");
  for (const field of ["project", "runtime", "principal", "room", "topic", "chat", "fileHash"] as const) {
    assert(before[field], `missing before ${field}`);
    assert.equal(after[field], before[field], `${field} changed`);
  }
  assert(before.startedAt && after.startedAt && after.startedAt !== before.startedAt, "no new physical start");
  assert(before.messageIds.length >= 2, "before chat evidence missing");
  const retained = after.messageIds.filter(id => before.messageIds.includes(id));
  assert.deepEqual(retained, before.messageIds, "durable history lost or reordered");
}

export function localOrigin(value: string) {
  const url = new URL(value);
  assert(url.protocol === "http:" && ["127.0.0.1", "localhost"].includes(url.hostname)
    && !url.username && !url.password, "isolated local service required");
  return url.origin;
}

type DockerOwner = { Name: string; Config: { Image: string; Labels?: Record<string, string> }; Mounts: { Type: string; Source: string }[] };
export function ownsContainer(item: DockerOwner, image: string, runRoot: string, run: string) {
  assert(/^[a-f0-9-]{36}$/.test(run), "invalid run identity");
  if (item.Config.Image !== image) return false;
  if (item.Name === `/billing-${run}-host-network-probe`) return true;
  return Boolean(item.Config.Labels?.["computer.finite.v2.project_id"] && item.Mounts.some(m =>
    m.Type === "bind" && path.resolve(m.Source).startsWith(path.resolve(runRoot, "stack") + path.sep)));
}
