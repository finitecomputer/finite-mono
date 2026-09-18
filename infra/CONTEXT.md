# Finite Deployment

- **Product Release**: a versioned artifact published for user installation.
- **Production Deploy**: a change to running infrastructure or service state.
- **Runtime Artifact Promotion**: making an immutable image eligible for new
  Agent launches. It does not change existing Agents.
- **Runtime Rollout**: explicitly upgrading selected existing Agents while
  preserving their identity and durable state.
- **Deployment Record**: the source revision, artifact digest, observed state
  and verification evidence for one deployment attempt.
- **Mutation Boundary**: the first production write; interruption after it
  requires reconciliation before another deployment.

Use [the runbooks](runbooks/README.md) and executable configuration for current
operations. A source merge, promotion or successful build is not a deployment.
