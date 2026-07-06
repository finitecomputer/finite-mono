# RFC 0002: Finite Blob Shared Substrate

Status: draft for Friends Alpha

## Problem

Finite Chat, Finite Sites, and future Finite Brain all need durable blob
storage. Chat already has encrypted attachment blob references, and Finite
Sites already stores content-addressed site assets. Finite Brain wants a
similar capability without receiving broad bucket credentials or depending on
chat-specific attachment semantics.

## Decision

Define Finite Blob as a provider-neutral service contract. Product callers ask
for scoped upload and download capabilities; the service decides authorization,
records usage, and hides the backing provider.

The first production-shaped backend should be S3-compatible object storage.
Latitude Object Storage is the first canary candidate because it is
S3-compatible and can be tested without coupling the protocol to one provider.
Local disk remains the dev/test backend.

## Non-Goals

- Do not give agents direct S3 or Latitude bucket credentials.
- Do not make every blob inherit chat attachment encryption rules.
- Do not make profile avatars part of the encrypted blob policy.
- Do not make billing required before Friends Alpha.

## Terms

- Finite Blob: provider-neutral blob service contract.
- Principal: the actor authorization attaches to.
- Scoped Capability: a bounded upload or download authorization.
- Blob Ref: product-stored reference to immutable blob bytes and metadata.
- Central Allowlist: the initial Finite Computer core table that permits
  principals to use products and blob capabilities.

## Product Policies

Finite Blob owns byte storage and capability issuance. Products own meaning:

| Product | Blob policy |
| --- | --- |
| Finite Chat attachment | Encrypted before upload; room/message capability; no plaintext metadata in blob service. |
| Finite Sites asset | Site/project ACL; may be public or capability-gated; versioned through site manifests. |
| Finite Brain artifact | Agent/user delegation ACL; private by default unless explicitly shared. |
| Profile avatar | Public/profile-derived cache; separate namespace and invalidation policy, not encrypted chat blob storage. |

## Initial Auth Model

Friends Alpha uses a central allowlist in Finite Computer core:

```text
principal npub/account id
enabled products: chat | sites | brain | blob
optional byte limits
optional expiry
environment
```

The blob service checks the allowlist and any product-provided delegation or
resource grant before minting a Scoped Capability.

## Capability Shape

An upload capability should bind:

- principal;
- product;
- resource scope;
- expected content hash if known;
- maximum bytes;
- expiry;
- permitted method/path or signed URL;
- idempotency key.

A download capability should bind:

- principal;
- product;
- blob ref or content hash;
- resource scope;
- expiry;
- permitted method/path or signed URL.

All capability decisions should be auditable enough to support later monthly
npub authorization or pay-as-you-go usage.

## Provider Boundary

Provider details live behind the service:

```text
Finite Blob API
  local filesystem backend
  S3-compatible backend
    Latitude canary
    AWS/R2/MinIO possible later
```

Product callers store Finite Blob refs and capabilities, never bucket names,
bucket credentials, or provider-specific URLs as authority.

## Evaluation

Unit tests should cover:

- wrong principal;
- disabled product;
- expired capability;
- exceeded byte limit;
- wrong hash;
- wrong method/path;
- replayed idempotency key;
- revoked allowlist entry;
- product policy mismatch.

Integration tests should cover:

- local disk backend;
- S3-compatible backend;
- chat encrypted attachment upload/download;
- sites asset upload/download;
- a finite-brain-style private artifact flow.

Canary measurements should cover:

- upload/download latency from iPhone;
- upload/download latency from agent runtime;
- upload/download latency from Finite Sites server;
- finite-brain workload shape;
- storage and egress cost under expected Friends Alpha usage.
