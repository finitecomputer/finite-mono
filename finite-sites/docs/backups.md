# Sites backup implementation status

The selected design is [revision backups with shared-state checkpoints](adr/0029-revision-backups-with-shared-state-checkpoints.md).
Tracking: [FIN-54](https://linear.app/finitecomputer/issue/FIN-54).

## Local foundation

The operator can capture and restore a **local** Recovery Point:

```sh
finitesitesd backup capture --data /path/to/sites --repository /path/to/backup
finitesitesd backup restore --repository /path/to/backup --point POINT_SHA256 --target /path/to/new-target
```

Capture prints a JSON receipt with the point ID, capture timestamp, and count
of newly stored objects. Both parent directories must exist. The repository
must be outside the source tree. Restore requires a destination that does not
exist; it never overwrites a live registry. Inspect protected snapshot databases
only through `scripts/snapshot-sqlite` or disposable scratch copies.

The source registry is opened read-only and is not initialized or migrated.
SQLite snapshots preserve shared metadata while published blobs are stored
once by content hash. Git bundles preserve all captured refs and recorded
historical object IDs, including source-only projects and non-deploy branches.
Historical commits use `refs/finite-recovery/` in the restored repository so
later Git maintenance cannot discard them. Live branch names are unchanged.
Each operator invocation still walks the full catalog and rebuilds Git bundles;
deduplication reduces stored bytes, not capture work. Per-revision execution is
not implemented yet.

Capture checks for pending Git reconciliation and for observed Git/catalog
changes across the checkpoint. A busy or inconsistent generation fails and can
be retried; earlier completed points and immutable objects remain available.
This is optimistic validation, not a global serving freeze. It is not yet a
qualified production capture scheduler, and sustained writes can prevent a
complete checkpoint.

The current event log deduplicates Git transitions and has no authoritative
per-ref cursor. Capture therefore accepts only an unambiguous acyclic chain of
recorded transitions for each ref, with the observed ref at its terminal SHA.
Unrecorded tips, cycles (including some branch delete/recreate and rollback
histories), and branching histories fail closed. Do not repair or rewrite user
history to make capture succeed. Writer-coordinated checkpointing is still a
production gate; this conservative operator command is not its replacement.
Missing repositories are refused, not synthesized as empty. An initialized,
empty bare repository can be captured and restored.

Completion manifests are content-addressed and written last. Restore validates
object hashes, registry integrity, required blob coverage, project inventory,
Git bundles, registry-required historical objects, and refs in private scratch
space before creating the destination.
It does not start the daemon or send email. Boot a test restore with dev mail
and production network access disabled. Never start a failed restore.

On Linux and macOS, the verified tree's files and directories are synced before
an atomic no-replace rename, followed by syncing the destination parent.
Concurrent restores cannot overwrite one another. Interruption before
publication leaves the destination absent; scratch directories left by a killed
process are private but require operator cleanup. A parent-sync error after
rename is reported as a failed restore even though the complete target exists.
Keep that target offline and investigate; do not automatically delete or retry
over it. Other operating systems are not supported for atomic publication.

Current ceilings are 100,000 manifest inventory items (files, projects, refs,
and retained Git objects combined), 64 MiB serialized manifest, and 1 GiB per
object. Objects are currently read into bounded memory; allow more than 1 GiB
of memory plus Git working space. Each SQLite capture and each Git command has
a 60-second deadline, and Git stderr is capped at 1 MiB. These are safety
ceilings, not production sizing or throughput qualification.
The restored tree is also bounded to one million entries and depth 128;
capture applies a conservative expansion budget before completing a point.

The local repository is **not encrypted**. It contains private source, registry
credentials/auth state, and the cookie signing secret. Directories are private
and object files are created with private permissions. Do not commit it, attach
it to a ticket, or upload it through an unqualified transport. Keep it on a
protected filesystem with enough space for the source checkpoint, bundles, and
retained objects. There is no automated deletion; unfinished capture objects
remain reusable but consume disk.

## Local verification

Run `scripts/with-dev-env cargo test -p finitesitesd --locked` for operator
capture/restore, crash/retry, concurrent no-replace publication, bounded Git
process failures, and application HTTP tests. The recovery application test
publishes a first version, captures/restores it, then uses the original editor
credential to clone and publish Version 2 on the restored server. The original
owner changes sharing through the signed API; the restored bytes become
public while the source remains private at Version 1. Another recovery test
checks that an original viewer cookie still works and revocation still applies.

The workspace gate is `scripts/with-dev-env just test`, which supplies the
isolated Postgres environment required by Core tests. Local macOS success is
not Linux, AWS, Fly, or production recovery qualification.

## S3 operator transport (synthetic qualification only)

**Do not upload production Recovery Sets with this increment. Client-side
encryption and independently recoverable decryption keys are not implemented.**
The data is not only static HTML: it includes private Git/source, registry
credentials and authentication state, and the cookie signing secret. HTTPS
protects transport and S3 server-side encryption protects remote storage;
neither keeps plaintext from S3 or principals with decrypt/read access. The
loopback tests simulate S3 responses, not actual AWS encryption or retention.

For a disposable, synthetic Recovery Set and a separately authorized test
bucket, the explicit operator commands are:

```sh
finitesitesd backup capture-s3 --repository /scratch/synthetic-backup --point LOCAL_POINT_SHA256 --bucket TEST_BUCKET --prefix synthetic-recovery --region REGION
finitesitesd backup restore-s3 --point REMOTE_POINT_SHA256 --version-id REMOTE_VERSION --target /scratch/new-synthetic-target --bucket TEST_BUCKET --prefix synthetic-recovery --region REGION
```

The first command transports an already captured local point. It first restores
that point into private scratch space to run the existing full validator. It
then verifies every uploaded or reused dependency by a bounded GET and SHA-256,
using create-only `If-None-Match: *` PUTs with a SHA-256 request checksum. A
conditional-write loser verifies the winner's bytes before reuse. It never
treats ETags or caller-supplied metadata as content-integrity proof.

The completion manifest is written last and identifies exact non-null S3
versions for every dependency, including the local manifest. Capture prints a
JSON receipt containing the remote `id`, `version_id`, and `local_point`.
Keep the receipt, bucket, prefix and region with the independent recovery
records. Restore requires the exact receipt version; it never falls back to
the latest object or guesses a point. A failed or interrupted upload may leave
reusable objects; retry the same local point. A lost final response may leave a
complete point; retry returns its receipt. Failed verification never publishes
the local restore target. Existing targets remain protected by the same local
no-replace installer.

Prerequisites for eventual cloud qualification, not resources provisioned here:

- An independently administered private general-purpose S3 bucket with Block
  Public Access, TLS-only access, versioning **Enabled**, and an appropriate
  default encryption policy. Capture checks versioning before uploads and
  before completing the point. Every PUT/GET must report recognized SSE-S3,
  SSE-KMS or DSSE-KMS encryption; missing/unknown encryption or null/missing
  version IDs fail. The command honors bucket default encryption and does not
  override a KMS policy with SSE-S3. Policies requiring explicit encryption
  request headers need separate qualification; this command sets none.
- Capture access scoped to `s3:GetBucketVersioning`,
  `s3:GetLifecycleConfiguration`, and `s3:GetObject`, `s3:GetObjectVersion`,
  `s3:PutObject` under the chosen prefix. Separate restore access needs object
  version reads and independently available KMS decrypt permission when used.
  Deny uploader deletion/version deletion and bucket-policy/lifecycle changes.
  Do not place credential values in flags, receipts, docs or the repository.
- No enabled current-object or noncurrent-version expiration rules anywhere
  in that bucket. Capture rejects these conservatively even for unrelated
  prefixes, and fails if it cannot inspect the policy. There is no retention
  period, garbage collector, DELETE, lifecycle writer or source-deletion mirror
  in this implementation. Policy checks are observations, not a lock against
  later admin changes; independent retention/IAM/Object Lock qualification is
  still required. **Object age is never a safe proxy for dependency lifetime.**

Only an explicit S3 command initializes the AWS credential provider chain
(environment/profile, process, role/web-identity/container/instance providers;
SSO is not compiled in). Local capture/restore and daemon startup do not.
`--endpoint-url` supports HTTPS test endpoints and HTTP on loopback only.
Inherited shared or service-specific endpoint overrides cannot replace this
validated flag. Tests clear inherited configuration and supply synthetic
credentials; the endpoint-precedence regression injects only loopback overrides.
These commands reuse the 60-second backup operation ceiling for each complete
network operation, including GET bodies, with at most three SDK attempts.
They retain the existing local object/manifest/inventory limits and require
space for a full validation restore. GET verification reads reused bytes again:
deduplication saves uploads/storage, not download bandwidth or capture work.
On reuse, the local upload buffer and the downloaded verification buffer can
coexist: allow roughly **two object-sized RAM buffers**, plus SDK, manifest,
SQLite and Git overhead. During restore, downloaded repository objects and the
materialized restore tree coexist on disk; capture also retains its full local
validation tree during upload. Provision scratch space for both copies and Git
expansion. The **1 GiB per-object ceiling is not qualification for a 1 GiB Fly
Machine**, nor a measured peak-memory or remote-volume sizing guarantee.

Run `scripts/with-dev-env cargo test -p finitesitesd --locked --test backup_s3`
for the synthetic public CLI contract. SDK additions are pinned to
`aws-sdk-s3 1.115.0`, `aws-config 1.8.11`, and `aws-smithy-http-client 1.1.9`,
using the workspace's ring TLS provider without SSO, SigV4a, a legacy TLS stack,
or another native crypto library. The lock adds 46 packages without replacing
existing versions; declared added MSRVs are at most 1.88. Verification uses the
repo's pinned Nix Rust 1.93.1, not a separate minimum-version build.

AWS contract references: [conditional PUT and version IDs](https://docs.aws.amazon.com/AmazonS3/latest/API/API_PutObject.html),
[version-specific recovery](https://docs.aws.amazon.com/AmazonS3/latest/userguide/versioning-workflows.html),
[lifecycle expiration](https://docs.aws.amazon.com/AmazonS3/latest/userguide/lifecycle-expire-general-considerations.html),
[server-side encryption](https://docs.aws.amazon.com/AmazonS3/latest/userguide/UsingServerSideEncryption.html).

## Still required before production

- Revision-triggered durable work tracking and background execution, including
  source-only Git updates, retry/backoff, missed-work reconciliation, and an
  authoritative writer-coordinated Git checkpoint boundary.
- Qualify the S3 operator transport with independently recoverable credentials,
  client-side encryption/key custody and tested retention. Synthetic
  exact-version receipt tests do not complete these gates.
- Shared-state scheduling and dependency-aware retention. Do not configure an
  object-age lifecycle that deletes blobs needed by newer recovery points.
- Bounded source-host resource use, concurrency/crash qualification, freshness
  and failure reporting through `scripts/finite-status`, and an independent
  alert path.
- Real AWS contract tests and an actual remote restore onto an empty Fly volume,
  including the original publisher's clone/push/publish flow and preserved
  permissions. Local tests are not off-host recovery proof.
- Independently recoverable runtime configuration, mail/service credentials,
  encryption keys, and image access. These are not all files in the Sites data
  directory; the local capture includes the cookie key, not Fly secret values.

No production service, AWS resource, CLI fleet pin, DNS record, or migration
state is changed by implementing these commands. Option A / Latitude remains
intact. A local repository on the serving volume is not an independent backup.
