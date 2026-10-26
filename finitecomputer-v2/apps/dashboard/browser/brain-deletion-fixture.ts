import type { BrainEventTemplate } from "../src/lib/brain-deletion";

// Only used by deletion proofs to change the Brain after its preview.
export function renameBrainFixtureEvent(input: {
  brainId: string;
  adminNpub: string;
  note: string;
  changeId: string;
  createdAt: number;
}): BrainEventTemplate {
  return {
    kind: 30_078,
    created_at: input.createdAt,
    tags: [
      ["d", `finite-brain-admin-access-change:${input.brainId}:${input.changeId}`],
      ["brain", input.brainId],
      ["action", "rename-brain"],
    ],
    content: JSON.stringify({
      version: "finite-brain-admin-access-change-v1",
      brainId: input.brainId,
      changeId: input.changeId,
      action: "rename-brain",
      adminNpub: input.adminNpub,
      note: input.note,
      createdAt: new Date(input.createdAt * 1000).toISOString().replace(/\.\d{3}Z$/u, "Z"),
    }),
  };
}
