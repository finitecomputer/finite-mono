# Managed Agent identity conformance

This gate proves the composition required by issue #221. Finite Identity is the
single source of truth for readable identity aliases; Core remains the source
of truth for account-to-Agent ownership; Chat, Sites, and Brain keep independent
product state and permissions.

Run the deterministic local gate from an exact checkout:

```sh
just identity-conformance
```

## What the gate proves

| Boundary | Required proof |
|---|---|
| Runner → Identity | Creation reads the Runtime public `agent_npub`, binds the Core-assigned managed email before completion, accepts an exact retry, and fails closed on a different Principal. |
| Identity public resolution | NIP-05 and product grant resolution return the same hex public key for the managed email. |
| Chat | Hosted Device registers the human User Principal separately, and the durable Agent conversation remains bound to the exact Agent Principal across duplicate selection and restart. |
| Sites | Sites asks Identity only whether an already-stored Project grant and an actor Principal are equivalent. Identity does not create the Project grant or Viewer Session. |
| Brain | Brain resolves the managed email through Identity, verifies the Agent belongs to the Personal Brain owner through Core, and stores an explicit Personal Agent relationship. A navigation-supplied npub cannot override either source. |
| Credentials | Runner, Brain, and Hosted Device may read the temporary backend-only operator environment. Sites receives only the loopback Authority URL. Generated devfinity configuration contains no operator token value, browser environment, or Runtime environment entry. |

The tests are compositional on purpose. The Authority test anchors one email,
npub, and hex public key across its public grant, NIP-05, and trusted Brain
resolution surfaces. The product tests then prove each consumer uses the
bounded surface it owns. This avoids inventing a universal product permission
API merely to make an end-to-end test easier.

## Production canary acceptance

Use a newly created managed Agent whose account owner is the canary user. Do
not create a synthetic immutable production binding.

1. Record the Core Project ID, managed Agent email, Runtime ID, Runner class,
   and Runtime `/contact` `agent_npub`. Do not record credentials.
2. Confirm the creation completed only after the Runner binding call.
3. Fetch both NIP-05 origins and require the managed localpart to map to the
   Runtime public key:

   ```sh
   curl --fail --silent \
     "https://identity.finite.vip/.well-known/nostr.json?name=LOCALPART"
   curl --fail --silent \
     "https://finite.vip/.well-known/nostr.json?name=LOCALPART"
   ```

4. Open the canonical Chat conversation and confirm its Agent npub matches the
   Runtime contact. Restart the Hosted Device and confirm the binding and
   transcript remain unchanged.
5. Create a dedicated Sites Project grant for the managed email. Confirm the
   Agent can use that Project and cannot use an ungranted Project.
6. Create the canary Personal Brain as the owning account. Confirm Brain shows
   the same managed email/npub as its explicit Personal Agent. Confirm an
   unrelated account cannot select or replace it.
7. Restart Identity, Core, Chat, Hosted Device, Sites, and Brain. Repeat steps
   3–6 without creating another binding or permission.
8. Inspect process configuration by variable name only. Fail acceptance if
   `FINITE_IDENTITY_OPERATOR_TOKEN` appears in dashboard/container
   configuration, Runtime environment, response bodies, logs, or command
   arguments.

Identity equivalence is not authorization. A passing canary must include the
negative ungranted Sites Project and unrelated-account Brain checks.
