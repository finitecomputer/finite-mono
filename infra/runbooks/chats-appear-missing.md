# Chats appear missing

Treat an empty or smaller sidebar as an availability incident. Do not create a
Room, run `/fresh`, delete state, rewrite selection, or mutate production while
diagnosing.

1. Keep the user on the current Hosted Web Device. Record only count totals and
   purpose-scoped pseudonyms; do not paste live ids, email, message bodies, or
   attachment contents into logs or tickets.
2. Read service health and recent errors:

   ```sh
   systemctl status finitechat-hosted-device finitechat-server
   journalctl -u finitechat-hosted-device -u finitechat-server --since -30min
   ```

3. Confirm the Hosted Web Device identity file and `client.sqlite3` either both
   exist or both do not. A partial pair is recovery-required and must not mint a
   replacement identity.
4. Through the authenticated dashboard, use `Retry load`. A valid binding must
   open retained history without Agent Runtime contact and without rewriting
   the binding. `Retry load` cannot authorize a missing binding. `Retry claim`
   is only for mutation authority and must not gate reading.
5. If the Agent replies in only some retained Rooms, inspect the generated
   Hermes platform configuration read-only. The normal adapter serves every
   Room joined by the Agent Device; an explicit `extra.room_id` or
   `FINITECHAT_ROOM_ID` is the only supported Room filter. Record only whether
   a filter exists, not its live value. A Hermes home-channel is a routing
   preference and is not a subscription filter.
6. Compare count-only Room, Topic, and Chat totals in the client store with the
   canonical plus `Previous conversations` projection. Any retained-versus-
   visible mismatch blocks release/admission.
7. If the binding is missing, corrupt, or membership-invalid, do not choose a
   Room from `selected_room_id`, display position, timestamps, identifier order,
   or the fact that only one candidate is currently visible. There is no
   automatic Room reconciliation or legacy binding migration. A corrupt or
   invalid existing binding is immutable to ordinary product flows; stop and
   use the separately authorized recovery workflow.
8. A missing binding may show the user `Finish chat setup` only for the narrow
   case where Core committed creation but the dashboard lost the authorization
   response. Ordinary load must not invoke it automatically. If the user
   invokes it, the product must use a fresh Core read and require the exact
   Account-owned Project plus exactly one durable creation request in
   `requested`, `launching`, or `running` state before writing the omitted
   bootstrap authorization. This action never scans or chooses Rooms; retained
   candidate Room state still fails closed. If the action is absent or refuses
   the state, stop rather than manufacture an authorization.
9. Reproduce the symptom without mutating the affected state, then reproduce it
   with synthetic state before proposing a repair. Record which observations
   support the cause and which explanations remain hypotheses.
10. Escalate before any production migration, repair, restore, service restart,
   deploy, or traffic change. Obtain explicit authorization and name the
   preserved backup, rollback procedure, and stop boundary first.

The implementation invariant, proved with synthetic state rather than by
decrypting a production journal during diagnosis, is that a legitimately
authorized first bootstrap seals the exact Room create request, including its
intended Room id and MLS group id, before any server mutation. It next seals the
claimed Agent KeyPackage before Room creation and the exact prepared add-member
commit before submit. If the server accepted Room creation but the Device did
not save the matching local MLS group, restart must replay the exact journaled
request and group id. An interrupted add-member may resume only the journaled
Room and commit; it must not claim again after the claim is recorded, generate a
different group or commit, or reconcile retained Rooms.

Ordinary durable protocol sync may process already-authorized membership and
messages after a reconnect. Do not disable or describe that convergence as a
migration or recovery repair; it must not choose or rewrite the canonical
binding.

Resolution requires the same retained conversations to be reachable through
the product. A green timer, database integrity check, or confirmation that rows
exist is not resolution.
