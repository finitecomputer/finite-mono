# Legacy Cleanup Manifest

Status: proposed cleanup after v2 split.

The original `finitecomputer` repo should become exactly the product already
shipping to box1/TRF while those users are unmigrated.

## Keep In Legacy finitecomputer

- existing whiteglove dashboard and support flows;
- host workspaces and deployment runbooks for box1/TRF/smoke;
- `finited` and broad `finitec` commands needed by those hosts;
- existing runtime upgrade and rollback paths;
- migration/import bridge code needed to move users to v2.

## Remove From Legacy Once v2 Owns It

- WorkOS self-serve SaaS dashboard code;
- Core SaaS service/deploy manifests;
- SaaS runner service/deploy manifests;
- Phala/confidential-runner launch experiments;
- Tinfoil/fstore abandoned spike artifacts except archived docs;
- finite-lat-1/finite-lat-2 SaaS deploy docs that no longer apply to legacy;
- Finite Private grant/key issuance code that is v2-only;
- docs that describe the new self-serve product as if it lives in legacy.

## Cleanup Rule

Do not delete legacy code that is still used by box1/TRF deployments. Move v2
code only after this repo has an equivalent file, owner, and test/deploy path.
