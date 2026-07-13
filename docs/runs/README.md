# Run documents
Status: ACTIVE

Standing rules: work only the one ACTIVE run, top-down; parking-lot everything
else. Every commit body names the active run and exact queue bullet. The win
condition is Ready-for-Paul: hand Paul an Acceptance Request he can execute in
the stated number of minutes. If the queue is exhausted, emit it and stop.
During incidents, fix causes and fail closed; add no user-facing recovery
control without Paul's one-line authorization.

1. A run doc scopes one bounded outcome. Its header names status, owner, opened
   date, acceptance, and expiry.
2. PROPOSED drafts have no work authority; Paul flips them to ACTIVE. PAUSED
   preserves a queue but removes its work authority. Exactly one run may be
   ACTIVE. Record an explicit owner-directed sequence change in both affected
   run docs.
3. Work the ACTIVE queue top-down. Every retained item is a closure
   prerequisite; priority controls order, not whether it may be skipped. Move
   optional or discovered work to `parking-lot.md`, one line per idea.
4. Every feature or fix commit body names `Run:` and `Queue:` with the ACTIVE
   run path and exact bullet it serves. If no bullet authorizes the work, do not
   make the change.
5. The final implementation item is an **Acceptance Request**, not a vague
   "Paul acceptance" gate. It states the exact steps Paul performs, where
   (URL, host, and account), what he should observe after each step, the exact
   deployed revision, and an estimated duration. If those fields cannot be
   filled, their prerequisites remain queue items.
6. Once everything left requires Paul, emit the Acceptance Request and stop.
   Reaching **Ready-for-Paul** is the implementation session's successful end;
   do not route around acceptance by starting unrelated work.
7. During an incident, start read-only and separate observations from
   hypotheses. A migration, repair, restart, restore, deploy, or other
   production mutation requires a reproduced cause, synthetic proof, a named
   backup and rollback boundary, and Paul's explicit authorization. Selection,
   display order, timestamps, and identifier order are not authority to choose
   or rewrite user state. Ambiguity fails closed without mutation. A proposed
   escape hatch never ships as an improvised user-facing recovery feature.
8. Link to doctrine and ADRs; do not restate them. Expiry is a
   stop-and-rescope boundary, not acceptance.
9. A run closes only after every retained item and the named human acceptance
   pass. Extract durable decisions and delete the run doc in the closing
   commit; Git history is the archive and `parking-lot.md` is the sole
   permanent resident inside this directory.

## Acceptance Request template

- **Revision:** exact deployed Git revision and component release/image ids.
- **Where:** URL/host and dedicated account; name secrets only by location.
- **Time:** realistic elapsed minutes.
- **Steps and observations:** numbered, one expected observation per action.
- **Pass:** the acceptance statement in the run, including retained identifier
  or recovery evidence where applicable.
- **Fail/stop:** the first unsafe or ambiguous observation, with the read-only
  evidence to capture and the rollback or escalation boundary.
