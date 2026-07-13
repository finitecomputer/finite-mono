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
   open retained history without Agent Runtime contact. `Retry claim` is only
   for mutation authority and must not gate reading.
5. Compare count-only Room, Topic, and Chat totals in the client store with the
   canonical plus `Previous conversations` projection. Any retained-versus-
   visible mismatch blocks release/admission.
6. If the binding is missing, corrupt, or membership-invalid, stop and use the
   recovery workflow. Do not guess from `selected_room_id`. Legacy migration is
   the only authorized binding creation path and must preserve preflight and
   postflight identifier sets.
7. Escalate before any production migration, repair, restore, service restart,
   deploy, or traffic change. Preserve the current snapshot first.

Resolution requires the same retained conversations to be reachable through
the product. A green timer, database integrity check, or confirmation that rows
exist is not resolution.
