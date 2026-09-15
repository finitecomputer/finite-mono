# FIN-89 Brain overview prototype

Question: does a categorized membership list make the Brain tab useful before a content viewer exists?

Run from the repository root in the pinned development environment:

```sh
just dashboard brain-prototype
```

Open http://127.0.0.1:13089/dashboard/machines/runtime_web_design/brain-prototype.
The existing design fixture supplies the dashboard shell and its example agent, Moss.
The Brain page uses in-memory synthetic memberships and makes no Brain requests.

Compare `?variant=cards` (default), `?variant=rows`, and `?variant=columns` with the bottom arrows or keyboard left/right. Use the preview controls to change identity and inspect empty, unavailable, stale, and access-lost states. Changing identity clears the displayed result before loading the new example. Refresh shows loading and updates the successful-fetch time; the failure scenarios remain failures until another scenario is selected. Expand “Prototype state” for the full example state.

The route requires development mode, the existing local-account fixture switch, and the fixture machine ID. Production requests return not found, and production navigation stays disabled. No new authentication bypass or Brain integration is introduced.

This is a throwaway review artifact on `prototype/brain-overview`, tracked in [FIN-89](https://linear.app/finitecomputer/issue/FIN-89/add-a-basic-brain-overview-with-categorized-membership-cards-and). The layout preference is pending user review. The shipping identity scope and authorized agent inventory path remain unresolved; sample memberships do not validate that contract. File counts and storage size are deferred.
