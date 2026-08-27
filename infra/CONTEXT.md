# Finite Deployment

Finite Deployment is the language for turning reviewed Finite source and
artifacts into production state. It distinguishes user-consumable releases,
server-side deploys, Agent Runtime eligibility, and existing-Agent migration.

## Language

**Product Release**:
A signed, user-consumable product artifact published for installation or
download.
_Avoid_: Release when referring to server-side production state

**Production Deploy**:
A change to production server-side infrastructure or service state.
_Avoid_: Deploy when referring to Agent Runtime rollout

**Runtime Artifact Promotion**:
Making an immutable Agent Runtime artifact eligible for future Agent launches.
_Avoid_: Runtime deploy, image tag

**Runtime Rollout**:
Moving existing Agents to a promoted Runtime Artifact while preserving their
durable state and identity.
_Avoid_: Bot rollout, deploy

**Deployment Manifest**:
The desired production-state record used to decide what a deployment should
attempt and what evidence must be recorded.
_Avoid_: Deployment queue, release notes

**Production Branch**:
The git branch whose tip represents the source revision intended for the
production environment.
_Avoid_: Release branch, deploy queue

**Production Bootstrap**:
The hard-cut setup act that establishes the Production Branch and GitHub
production governance as authoritative from a named source revision.
_Avoid_: Rollback, migration, evidence of previous production state

**Deployment Record**:
The durable evidence left by a deployment attempt, including the intended
source revision, observed production state, verification results, and outcome.
_Avoid_: Grafana dashboard, workflow log

**Deploy Principal**:
The actor authorized to perform production deployment mutations.
_Avoid_: Operator laptop, root shell

**Mutation Boundary**:
The point in a deployment attempt after which cancellation becomes an
interrupted production mutation that must be reconciled before later deploys.
_Avoid_: Cancel point, deploy start

**Deployment Classification**:
The declared risk class of a deployment attempt, especially whether ordinary
binary rollback remains a valid response.
_Avoid_: Change type, release type

**Risky Path Set**:
A conservative list of source paths that require stronger Deployment
Classification when they change.
_Avoid_: Blocklist, forbidden paths

**Deployment Plan**:
The pre-mutation preview produced for a production promotion, naming the
intended source revision, classification, gates, artifact build, and expected
mutation.
_Avoid_: Deployment Record
