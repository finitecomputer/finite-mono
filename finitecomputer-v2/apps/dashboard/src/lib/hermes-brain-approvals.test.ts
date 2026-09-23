import assert from "node:assert/strict";
import test from "node:test";
import { hermesBrainApprovals } from "./hermes-brain-approvals";
import { parseApproveQuestion } from "./brain-approval-metadata";
import type { HostedChatMessage } from "./hosted-web-device";

const marker = "finite-brain-approval-filed brain=brain-1 request=approval-1";
const row = (kind: string, text: string, is_mine = false, final_delivery = true) =>
  ({ kind, text, display_content: text, is_mine, final_delivery } as HostedChatMessage);

test("native tool history reconstructs one reference-only card after reload", () => {
  const history = [row("tool", `${marker}\n${marker}`), row("message", "Please review the request.")];
  const projected = hermesBrainApprovals(history);
  assert.deepEqual(parseApproveQuestion(projected[1]), {
    service: "brain", requests: [{ brainId: "brain-1", requestId: "approval-1" }],
  });
  assert.equal(history[1].metadata_json, undefined, "projection does not mutate Hermes history");
  assert.deepEqual(hermesBrainApprovals(JSON.parse(JSON.stringify(history))), projected);
  assert.equal(hermesBrainApprovals([...history, ...history])[3].metadata_json, undefined);
});

test("prose and interrupted turns cannot lend references to another answer", () => {
  assert.equal(hermesBrainApprovals([row("message", marker)])[0].metadata_json, undefined);
  const history = [row("tool", marker), row("message", "Partial", false, false), row("message", "New turn", true), row("message", "Another answer")];
  assert(hermesBrainApprovals(history).every(message => !message.metadata_json));
  for (const malformed of [marker.replace("brain-1", "../brain"), marker.replace("approval-1", "x".repeat(129))]) {
    assert.equal(hermesBrainApprovals([row("tool", malformed), row("message", "Done")])[1].metadata_json, undefined);
  }
});
