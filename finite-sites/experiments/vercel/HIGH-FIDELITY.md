# Higher-fidelity proof boundary

The wildcard prototype keeps one Vercel project and adds real Finite NIP-98 verification,
owner-scoped control operations, managed Git publication, and source recovery.

- Existing Finite identities sign requests. The Rust `fsite` client is an
  interoperability test, not a new identity system. A prototype allowlist models
  publisher admission; it does not invent account enrollment or billing.
- The Sites service owns native Share rows and checks them on every request.
  The hosted-device exchange accepts the existing native proof envelope using
  a separate service credential. No dashboard or chat production deployment.
- A private GitHub fixture repository supplies managed source hosting. A push
  runs a publisher with a credential scoped to exactly one prebound Site.
  It preserves a complete Git bundle alongside each immutable content version.
  Source credentials and editor access remain GitHub-owned for this slice;
  this does not pretend to implement `fsite auth git` or a Finite Git gateway.
- The recovery set contains relational authority, complete published content,
  and source bundles. Viewer sessions/handoffs intentionally expire on restore.
  Restored grants determine future access; recovery never grants new access.
- Restore into a fresh database namespace and fresh Blob namespace, then serve
  using a new process with no access to original object paths. This proves an
  empty logical target, not independent-provider availability or production RPO.
- The wildcard host is an isolated, explicitly selected experiment suffix.
  Existing production Sites and account services remain unchanged.

Acceptance: actual signed fsite requests; cross-owner denial and signature/body
binding; ordinary managed Git push updates one Site; complete source restoration
with Git verification; rollback preserves revoked native Shares; wildcard TLS
and unknown-host denial where the experiment DNS suffix is available.

## Results — 2026-09-15

- Real Rust `fsite` registration, Project Init configuration validation/replay,
  status, and native Share changes work against the hosted prototype. Project
  bindings are provisioned by the experiment operator; Init does not create a
  managed repository. Native owners have no implicit authority over other Sites.
- 13 hosted native/publisher checks cover exact signed bytes, unauthorized
  owners, revoked admission, scoped publisher credentials, missing-source
  nonactivation, native nonce/callback replay, and revocation through rollback.
- Two successful GitHub Actions runs (`35016773249`, `35017022928`) followed
  ordinary pushes to the private fixture. The second changed content without
  changing the Vercel platform deployment. Each ready version has a source bundle.
- A real browser completed the native identity handoff, rendered private HTML
  and CSS, then lost access to both after native Share revocation.
- Six recovery checks passed: corrupt-object rejection; anonymous denial; revoked
  Share still denied; retained Share usable; restored content served; source
  cloned from its bundle with the original commit and complete reachable history
  verified by `git fsck`. The target has new database/Blob namespaces and a new
  serving process. Original viewer sessions are not restored.
- Nine input/auth tests and workspace clippy passed. The 49 hosted
  gate/version checks now target Wild-alpha and Wild-beta.
- Live wildcard `*.sites-poc.lwn.lol` points to the same project with a trusted
  wildcard certificate. Eleven checks prove DNS/TLS before Site creation,
  dynamic publication without Vercel domain entries or another deployment,
  browser storage and host-only cookie isolation, denial of sibling-origin
  proof redemption, control-host isolation, and revocation through rollback.
  See `evidence/wildcard-live.json`. The local console exposes Wild-alpha/Beta.

Evidence: `evidence/fidelity-{native,git,browser,recovery}.json` and browser
screenshots. These checks do not establish capacity or cost.

## Boundaries still requiring work

- **Account login:** the protocol and Finite identity are real; test principals
  are isolated. The bridge exercises the hosted-device exchange wire shape, but
  production WorkOS/dashboard/Hosted Device integration has not been deployed.
  The actual dashboard currently restricts its allowed Site origins; changing
  that boundary requires its own explicit configuration and tests.
- **Wildcard operations:** real DNS, initial certificate issuance and request
  routing are proven on a Vercel-managed zone. Certificate renewal and delegated
  external DNS remain untested. The native identity fixture now uses
  `https://gamma.sites-poc.lwn.lol`; all three demo Sites use the wildcard.
- **Git authorization:** GitHub owns editor access. The scoped publisher is a
  trusted CI actor that checks Git objects locally; the service does not run a
  Git parser to independently attest the uploaded bundle at publish time.
  `fsite auth git`, collaborator synchronization, and automatic repo provisioning
  are not implemented. The recovery drill independently verifies the Git bundle.
- **Recovery operations:** local checksums detect accidental corruption; they are
  not a signed backup authority. The export is one Site and excludes unfinished
  versions and transient sessions. Publisher credential hashes are preserved;
  issuance/rotation of replacement secrets is a separate operator concern.
  No provider-loss drill, scheduled backup policy, RPO/RTO, database migration
  compatibility, or whole-fleet restore is claimed.
- **Resource cleanup:** restore namespaces and objects remain as inspectable
  scratch evidence. The prototype has explicit limits and no automatic garbage
  collector. Only declared scratch resources may be removed.
