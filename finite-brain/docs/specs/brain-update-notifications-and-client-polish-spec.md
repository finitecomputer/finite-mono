# Brain Update Notifications And Trusted-Client Polish

## Problem Statement

FiniteBrain already has an authoritative encrypted sync log, sequence-based
catch-up, conflict handling, Brain Working Trees, and a browser Product Client,
but collaboration does not yet feel continuously synchronized. Hosted agents
can report an Agent Sync Daemon as running when no resident process is active,
and the Product Client refreshes primarily on open or explicit actions. A
second client can therefore commit valid Brain changes while another open
client remains stale.

Three smaller trusted-client gaps compound that friction. The Agent CLI skill
does not make the post-`open` Working Tree boundary hard to misuse; generated
Working Tree instructions can mislabel an Organization Brain as personal; and
readable Assets are decrypted by the Product Client but absent from its
sidebar, leaving users unable to discover or download them.

## Solution

FiniteBrain will deliver content-free **Brain Update Notifications** over one
authenticated Server-Sent Events connection per active client identity. A
notification names the affected Brain, its latest sequence, and whether
content or access changed. It is only a hint: clients always reconcile through
the existing authenticated, encrypted, sequence-based sync contract, and
reconnect recovery compares authoritative sequences instead of replaying
notifications.

Each hosted Agent Runtime will supervise one identity-level Brain sync process
covering its existing open Brain Working Trees. The Product Client will listen
while its Brain session is active and automatically reconcile the Brain open
on screen. Content bursts for the same Brain will be briefly coalesced; access
updates will be handled immediately. Unsaved work will never be overwritten.

The Agent CLI and FiniteBrain skill will retain explicit Working Tree context
while making the required post-`open` command location unmistakable. Generated
Working Tree instructions will identify the Brain kind, Brain ID, acting
Member Identity, and acting role accurately. The Product Client will show
readable Assets in the Folder sidebar and allow local, client-decrypted
downloads without adding a plaintext server download route.

## User Stories

1. As a human editing an open Brain, I want remote changes to arrive automatically, so that I do not work from stale knowledge.
2. As a hosted agent editing an open Brain Working Tree, I want remote changes to arrive automatically, so that collaboration does not depend on remembering `sync now`.
3. As a human collaborating with agents, I want browser and agent clients to use the same update signal, so that their behavior is predictable.
4. As a privacy-conscious user, I want update notifications to contain no Brain plaintext, so that the server notification channel does not weaken the encryption boundary.
5. As a client, I want notifications to identify the affected Brain and latest sequence, so that I reconcile only relevant state.
6. As a client, I want content and access changes distinguished, so that access loss is handled more urgently than ordinary edits.
7. As a client, I want notifications treated as hints rather than durable records, so that a missed notification cannot lose data.
8. As a reconnecting client, I want to compare authoritative Brain sequences, so that I safely catch up without notification replay.
9. As a user with many Brains, I want one notification connection per active identity, so that connection count does not grow with Brain count.
10. As a user, I want rapid edits to one Brain combined into one reconciliation, so that collaboration remains efficient without feeling delayed.
11. As a user, I want access changes handled immediately, so that removed access is not left active during a debounce window.
12. As a user, I want notifications for different Brains handled independently, so that activity in one Brain does not delay another.
13. As a hosted agent owner, I want one background Brain sync process per agent identity, so that all open Working Trees are managed consistently.
14. As a hosted agent owner, I want the Brain sync process restarted with the Agent Runtime, so that automatic sync survives runtime restart.
15. As a hosted agent owner, I want sync health based on live process evidence, so that `running` never means only that a flag was stored.
16. As a hosted agent, I want only existing open Working Trees reconciled, so that joining a Brain does not create unwanted plaintext projections.
17. As a browser user, I want automatic reconciliation only for the Brain open on screen, so that inactive Brains are not decrypted in the background.
18. As a browser user, I want the notification connection closed when the Brain session locks, signs out, closes, or loses connectivity, so that connection lifetime follows trusted-session lifetime.
19. As a browser user returning to an active Brain, I want sequence catch-up before live notifications resume, so that the view is current after absence.
20. As an agent with a local edit, I want a remote update to another file applied normally, so that one edit does not block the whole Brain.
21. As an agent with a conflicting local edit, I want both versions preserved, so that automatic sync never destroys my work.
22. As a browser user with an unsaved draft, I want a conflicting remote revision surfaced without overwriting my draft, so that I can resolve it deliberately.
23. As a client with one conflict, I want unrelated objects to keep synchronizing, so that conflict scope remains narrow.
24. As a browser user who loses Brain access, I want the Brain locked and decrypted session state cleared immediately, so that the active client respects revocation.
25. As a browser user who loses Folder access, I want no-longer-readable content removed from the active projection immediately, so that the client reflects current grants.
26. As a hosted agent that loses access, I want synchronization paused and future writes rejected, so that the server access boundary is enforced.
27. As a hosted agent that loses access, I want my persistent Working Tree and unsynced edits preserved, so that revocation does not falsely promise recall or silently delete existing plaintext work.
28. As an operator of the current single-server deployment, I want notifications broadcast in process, so that automatic sync does not require premature infrastructure.
29. As a future operator of multiple FiniteBrain server instances, I want the notification broadcaster behind an interface, so that a shared broker can be introduced before horizontal scaling.
30. As an agent opening a Brain, I want the CLI response to state where subsequent commands must run, so that I do not immediately hit `no Brain Working Tree found`.
31. As an agent following the FiniteBrain skill, I want every post-`open` operation anchored to the returned Working Tree, so that the intended Brain is explicit.
32. As a user with several open Brains, I do not want a hidden global last-opened Brain, so that commands cannot silently target the wrong Brain.
33. As an agent reading Working Tree instructions, I want the heading to name Personal or Organization Brain correctly, so that I understand the context.
34. As an agent reading Working Tree instructions, I want to see the Brain ID, acting Member Identity, and acting role, so that my authority and scope are explicit.
35. As a user whose Brain contains an Asset, I want the Asset visible in its Folder sidebar, so that I can tell it exists.
36. As a user selecting an Asset, I want to see its filename, path, content type, and size, so that I can identify it before downloading.
37. As a user selecting an Asset, I want to download the client-decrypted bytes, so that the stored evidence is usable.
38. As a privacy-conscious user, I want Asset downloads built locally in the Product Client, so that no plaintext download route is added to the server.
39. As a user, I want Assets visually distinguishable from Pages, so that the sidebar communicates object type.
40. As a user, I want Asset access to follow current Folder grants, so that an inaccessible Asset never appears or downloads.

## Implementation Decisions

- The canonical notification term is **Brain Update Notification**.
- Brain Update Notifications use authenticated Server-Sent Events, not polling or WebSockets.
- One active client identity owns one notification connection covering all Brains visible to that identity.
- A notification contains no Brain content or Folder Keys. It names the affected Brain, latest authoritative sequence, and one of two reasons: `content_updated` or `access_updated`.
- Notifications are coalescible delivery hints, not authoritative state, durable sync records, or a replay log.
- Reconnection refreshes accessible Brains and compares authoritative sequences for locally active Brains before resuming live delivery.
- Content notifications for the same Brain are coalesced for approximately 250 milliseconds. Access notifications bypass coalescing and are handled immediately. Different Brains are scheduled independently.
- The current single-server SQLite deployment uses an in-process broadcaster behind a notification interface. A shared broker is required before running multiple FiniteBrain server instances.
- Brain writes and access mutations publish notifications only after their authoritative transaction succeeds.
- Hosted Agent Runtimes supervise one identity-level Brain sync process. It owns one notification connection, discovers existing open Working Trees, watches local files, routes notifications, prevents duplicate writers, restarts with the runtime, and exposes live per-Brain health.
- Hosted agents reconcile only Brains with an existing explicit Brain Working Tree. Notifications never create a Working Tree.
- The Product Client listens only while its Brain session is active and reconciles only the open Brain. Other-Brain notifications may update a non-plaintext change indicator but do not decrypt content.
- Automatic reconciliation preserves unsaved browser drafts and unpushed agent files. Same-object divergence becomes an explicit conflict while unrelated changes continue.
- On access loss, the Product Client locks or leaves the affected Brain, clears no-longer-authorized decrypted session state, refreshes visible Brains and grants, and rejects further writes.
- On access loss, the hosted agent pauses the affected Working Tree and rejects further server writes while preserving its durable local plaintext and unsynced edits.
- Agent daemon state distinguishes a live supervised process from stopped, paused, reconnecting, and blocked states. A persisted marker alone cannot establish liveness.
- `fbrain open` retains the explicit Working Tree model and does not create global last-opened-Brain routing. Its result and the FiniteBrain skill make the required subsequent command location explicit.
- Generated root Working Tree instructions name the Brain kind, Brain ID, acting Member Identity, and acting Brain role. Folder instructions remain Folder-scoped.
- The Product Client projection retains readable Asset objects as well as Pages.
- Readable Assets appear in the Folder sidebar with a filename and type indicator. Selection shows path, content type, and size.
- Asset download uses locally decrypted bytes and a short-lived browser object URL. It obeys the same session and Folder-access boundaries as Page decryption.
- The first Asset client slice is read-only: browser upload, replacement, rename, move, deletion, and inline preview are excluded.
- The Agent Runtime research dependency issue observed during smoke testing is unrelated and excluded from FiniteBrain work.

## Testing Decisions

- The primary release-level seam is the existing disposable full-product Brain matrix, extended to run one real Brain server, two independent agent identities, their hosted sync lifecycle, and the Product Client.
- The matrix must prove agent-to-agent, agent-to-browser, and browser-to-agent convergence without polling or explicit refresh.
- The matrix must disconnect and reconnect each client class and prove sequence-based catch-up without notification replay.
- The matrix must prove same-Brain notification coalescing, immediate access handling, independent Brain scheduling, duplicate-notification tolerance, and missed-notification recovery.
- The matrix must prove that unrelated updates continue around one browser or Working Tree conflict and that both conflicting versions remain recoverable.
- The matrix must prove browser access loss clears the active decrypted projection and agent access loss pauses sync while preserving the Working Tree.
- The matrix must restart an Agent Runtime and prove its one identity-level sync process resumes without duplicate watchers or lost changes.
- The matrix must add a binary Asset through an agent Working Tree, observe it in the Product Client sidebar, select it, and verify downloaded bytes exactly match the source.
- The matrix must inspect generated Personal and Organization Brain Working Tree instructions for correct kind, ID, identity, and role.
- The matrix must execute the skill-documented `open` workflow and prove no post-open command fails for missing Working Tree context.
- Focused server tests should exercise authorization, event shape, post-commit emission, connection cleanup, access filtering, and in-process broadcaster behavior.
- Focused CLI/runtime tests should exercise Working Tree discovery, local file watching, notification routing, liveness states, crash restart, pause-on-revocation, and conflict preservation.
- Focused Product Client tests should exercise connection lifetime, active-Brain filtering, coalescing, reconnect catch-up, draft preservation, access loss, Asset presentation, and exact local download bytes.
- Tests assert external behavior and durable safety outcomes, not thread layout, timer implementation, DOM implementation details, or broadcaster internals.
- Existing sync-engine process acceptance tests, Product Client deterministic seams, server route tests, and the two-independent-home collaboration acceptance test are the preferred prior art.

## Out of Scope

- Polling for automatic sync.
- WebSockets or bidirectional notification transport.
- Durable notification replay or a second event log.
- A shared notification broker before FiniteBrain runs multiple server instances.
- Automatically opening or decrypting every accessible Brain.
- Creating Brain Working Trees in response to notifications.
- A hidden global last-opened Brain for context-free CLI commands.
- Browser Asset upload, replacement, rename, move, or deletion.
- Inline image, audio, PDF, or other rich Asset previews.
- A plaintext server Asset download route.
- Installing optional Python research libraries or changing Agent Runtime research tooling.

## Further Notes

- ADR 0043 records the notification architecture and trusted-client behavior.
- The existing encrypted sync log and latest sequence remain authoritative. Brain Update Notifications improve freshness but never weaken recovery or correctness when delivery fails.
- Access revocation prevents future authorization but cannot recall plaintext or keys already obtained by an authorized Agent Working Tree.
- Before horizontal server scaling, production readiness must add a shared broadcaster implementation and prove reconnect behavior across instance drain and replacement.
