import assert from "node:assert/strict";
import test from "node:test";

import {
  approveResponseMetadata,
  decodeApproveEnvelope,
  parseApproveQuestion,
  parseApproveResponse,
} from "@/lib/brain-approval-metadata";

const question = {
  metadata_json: JSON.stringify({
    approve: {
      service: "brain",
      requests: [{ brainId: "brain-1", requestId: "approval-1" }],
    },
  }),
};

const response = (choice: "approved" | "denied", artifactId?: string) => ({
  metadata_json: approveResponseMetadata(
    choice,
    [{ brainId: "brain-1", requestId: "approval-1" }],
    artifactId,
  ),
});

test("parseApproveQuestion parses the reference-only envelope", () => {
  assert.deepEqual(parseApproveQuestion(question), {
    service: "brain",
    requests: [{ brainId: "brain-1", requestId: "approval-1" }],
  });
});

test("parseApproveQuestion rejects absent, malformed, or non-brain metadata", () => {
  assert.equal(parseApproveQuestion({}), null);
  assert.equal(parseApproveQuestion({ metadata_json: "not json" }), null);
  assert.equal(parseApproveQuestion({ metadata_json: "[1,2,3]" }), null);
  assert.equal(
    parseApproveQuestion({
      metadata_json: JSON.stringify({ approve: { service: "sites", requests: [] } }),
    }),
    null,
  );
  assert.equal(
    parseApproveQuestion({
      metadata_json: JSON.stringify({
        approve: { service: "brain", requests: [{ brainId: "brain-1" }] },
      }),
    }),
    null,
  );
});

test("parseApproveQuestion treats a response envelope as not a question", () => {
  assert.equal(parseApproveQuestion(response("approved")), null);
});

test("parseApproveQuestion ignores payload fields rather than consuming them", () => {
  const withPayload = {
    metadata_json: JSON.stringify({
      approve: {
        service: "brain",
        nonce: "evil-attempt",
        requests: [{ brainId: "brain-1", requestId: "approval-1" }],
      },
    }),
  };
  assert.deepEqual(parseApproveQuestion(withPayload)?.requests, [
    { brainId: "brain-1", requestId: "approval-1" },
  ]);
});

test("parseApproveResponse parses the recorded choice with its artifact reference", () => {
  assert.deepEqual(parseApproveResponse(response("approved", "deadbeef")), {
    service: "brain",
    choice: "approved",
    requests: [{ brainId: "brain-1", requestId: "approval-1" }],
    artifactId: "deadbeef",
  });
  assert.equal(parseApproveResponse(response("denied"))?.choice, "denied");
});

test("parseApproveResponse treats a question envelope as not a response", () => {
  assert.equal(parseApproveResponse(question), null);
});

test("parseApproveResponse fails closed on unknown choices", () => {
  assert.equal(
    parseApproveResponse({
      metadata_json: JSON.stringify({
        approve: {
          service: "brain",
          choice: "maybe",
          requests: [{ brainId: "brain-1", requestId: "approval-1" }],
        },
      }),
    }),
    null,
  );
});

test("decodeApproveEnvelope is identical to the accessor pair on every shape", () => {
  const cases: { metadata_json?: string }[] = [
    // absent / malformed / non-brain
    {},
    { metadata_json: "" },
    { metadata_json: "   " },
    { metadata_json: "not json" },
    { metadata_json: "[1,2,3]" },
    { metadata_json: JSON.stringify({ approve: null }) },
    { metadata_json: JSON.stringify({ approve: "approve" }) },
    { metadata_json: JSON.stringify({ unrelated: true }) },
    {
      metadata_json: JSON.stringify({ approve: { service: "sites", requests: [] } }),
    },
    // question-side rejections
    {
      metadata_json: JSON.stringify({
        approve: { service: "brain", requests: [{ brainId: "brain-1" }] },
      }),
    },
    {
      metadata_json: JSON.stringify({ approve: { service: "brain", requests: [] } }),
    },
    {
      metadata_json: JSON.stringify({
        approve: {
          service: "brain",
          nonce: "evil-attempt",
          requests: [{ brainId: "brain-1", requestId: "approval-1" }],
        },
      }),
    },
    // question
    question,
    // responses
    response("approved"),
    response("approved", "deadbeef"),
    response("denied"),
    // response-side rejections (unknown choice, empty requests, non-string choice)
    {
      metadata_json: JSON.stringify({
        approve: {
          service: "brain",
          choice: "maybe",
          requests: [{ brainId: "brain-1", requestId: "approval-1" }],
        },
      }),
    },
    {
      metadata_json: JSON.stringify({
        approve: { service: "brain", choice: "approved", requests: [] },
      }),
    },
    {
      metadata_json: JSON.stringify({
        approve: {
          service: "brain",
          choice: 42,
          requests: [{ brainId: "brain-1", requestId: "approval-1" }],
        },
      }),
    },
  ];
  for (const message of cases) {
    const envelope = decodeApproveEnvelope(message);
    assert.deepEqual(envelope.question, parseApproveQuestion(message));
    assert.deepEqual(envelope.response, parseApproveResponse(message));
  }
});
