# Use Brain As The Product Language For Knowledge Spaces

Status: accepted 2026-07-21.

FiniteBrain will make a product-language hard cut from **Vault** to **Brain**
for each contained knowledge space. The canonical terms are **Personal Brain**
for the user's single personal space and **Organization Brain** for a shared
space, commonly spoken by its name such as “Acme Brain”; **FiniteBrain** remains
the product name and **Folder** remains the access boundary inside a Brain.

The hard cut applies to every product-facing contract: Product Client language,
managed agent guidance, CLI commands and output, public APIs, errors, docs, and
tests. The product will not teach or indefinitely alias both vocabularies.
Private implementation details such as database table names may retain `vault`
temporarily when renaming them adds migration risk without leaking into a
product-facing contract, but new product-facing work uses Brain exclusively.
The product has reached development boxes but has no production users, so
current development and deployed test data may be reset and all first-party
callers change together. No Vault-to-Brain data migration, dual API, or
backward-compatibility layer will be built without evidence of a real external
consumer.

“Vault” was rejected because users naturally describe both their personal
knowledge and shared company knowledge as Brains, while switching between
Brain and Vault makes the client, agent skill, and CLI feel like different
products. A UI-only relabel was rejected because agents and developers would
continue exposing the conflicting term through commands, APIs, and errors.
