# Finite Commercial Register

Finite's private Twenty application for structured organizations, contacts,
opportunities, purchases, payments, and recurring revenue. FiniteBrain remains
the source for meeting notes, Organization Wants, and narrative context.

This is intentionally not Salesforce and not a general ledger. The MVP answers:

- Who is the organization and who matters there?
- What did it buy, under what actual terms, and what is it receiving now?
- What did it pay Finite, independently of what Finite charged?
- What follow-on opportunity remains?
- What is current MRR, without counting one-time or bundled lines twice?

The domain language and authority boundaries live in
[`docs/commercial-relationships/CONTEXT.md`](../docs/commercial-relationships/CONTEXT.md).
The decision to use Twenty is recorded in
[`ADR-0008`](../docs/adr/0008-use-twenty-for-structured-commercial-relationships.md).

## MVP shape

Twenty's standard Company, Person, and Opportunity objects supply the familiar
organization, contact, and small-pipeline UI. The app adds this purchase chain:

```text
Company -> Commercial Account -> Commercial Arrangement
                                      |
                                      v
                               Purchased Package -> Offering Line
                                      |
                                      v
                                   Charge -> Incoming Payment
```

Charges and Incoming Payments are deliberately separate. A Package owns one
shared price, so included Offering Lines do not invent extra revenue. The app
also installs the Organization directory, Current customers, and Open
opportunities views.

The first implemented path is the simple NED case: one organization, one
account, a won Agent Camp purchase, and a separate exploring follow-on
opportunity. Sponsorship, payment allocation, supplier-cost attribution,
contributions, and accounting remain later slices.

## Developer commands

Run these from the monorepo root; the recipes enter the pinned Nix environment:

```console
just commercial check
just commercial test
just commercial app-build
```

After configuring a Twenty CLI remote, preview every schema change before
applying it:

```console
just commercial app-plan
just commercial app-apply
```

Do not use `--force` for ordinary app application. The digest-pinned production
service, backup, restore drill, DNS, and rollback boundary are declared under
[`infra/commercial-register`](../infra/commercial-register/README.md). The
production register becomes an authority only after the first off-host archive
has passed that empty-target restore and the real application has been applied
without seeding the synthetic NED fixture.

## Agent interface

The command accepts a versioned JSON update document. It searches only within
the obvious parent scope, preflights every match before the first write, creates
or updates matching records, rejects ambiguous matches, never deletes records
omitted from the document, rebuilds derived totals from stored records, and
prints a change report.

Two environment variables are required. Store their values outside git:

- `FINITE_COMMERCIAL_TWENTY_URL`: private Twenty base URL
- `FINITE_COMMERCIAL_TWENTY_API_KEY`: API key assigned to an appropriately
  scoped Twenty role

Apply an update from a file or standard input:

```console
just commercial agent apply --file update.json
just commercial agent apply --file - < update.json
```

Read the current organization summary:

```console
just commercial agent show --organization NED
```

Rebuild only the calculated Company totals after an effective date passes (or
after repairing source records):

```console
just commercial agent refresh --organization NED
just commercial agent refresh --all
```

The refresh operation never changes source records. It recalculates current
MRR, lifetime net cash, and current-customer status and skips Company records
whose projections are already current. Run the all-organizations form daily
once the register is deployed so the directory views follow effective dates
without requiring an unrelated update.

List reads follow Twenty's cursor pages rather than accepting the first page as
complete. A full page with missing or repeated pagination metadata fails before
a projection write, so a partial payment history cannot silently become an
authoritative total.

[`tests/fixtures/ned-update.json`](tests/fixtures/ned-update.json) is an
executable, explicitly synthetic contract example. Its contact and amounts are
not real NED facts and must never be used to seed the live register. Real
financial records require their actual Source Reference; incomplete facts use
the inline reconciliation marker rather than an invented value.

Recurring Package updates require a stable `priceTermKey`, effective start, a
real price, and a monthly, quarterly, or annual cadence. Updating the same key
changes that term; a new key creates a new effective-dated term, so history is
not overwritten. USD MRR is normalized automatically and activates according
to the effective dates. A non-USD recurring term supplies
`sourcedMonthlyPriceUsd`; the read-only `monthlyRecurringRevenueUsd` field is
always calculated and is never accepted in an update document.

Incoming digital-asset payments preserve native amount, asset code, network,
transaction reference, and an optional receipt-time USD value; later
exchange-rate changes do not rewrite it. If a received non-USD payment lacks a
sourced USD value, the USD lifetime-cash result is `null` with an unresolved
fact, never a fabricated zero.

Each Commercial Account starts with `cashHistoryReconciled: false`. Lifetime
cash remains unknown until an agent has checked the known historical period and
sets the field to true with a Source Reference. A reconciled account with no
Incoming Payments is then a real zero; an unreconciled account with no Payment
records is not.

Price-bearing Packages, Charges, and Incoming Payments require either a Source
Reference or an explicit reconciliation warning. The `show` response nests
Arrangements, Packages, Offering Lines, Charges, and Payments and returns those
references with the material answers.

Brain-projected relationship summaries are read-only in Twenty and include the
time at which the projection was refreshed.

The command's API credential is never printed. It uses Twenty's generated
`/rest/` record endpoints; the versioned app remains the only schema mutation
path.
