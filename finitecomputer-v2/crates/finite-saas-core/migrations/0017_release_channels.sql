-- Release channels and bundle artifacts (payload-generations plan, M1).
--
-- Bundle artifacts (skills_bundle, payload_bundle) are fetched-and-verified
-- content, never launchable compute; their tarball is pinned by
-- content_sha256. A release channel head names the artifact agents
-- subscribed to that channel converge on, per artifact kind. Channel
-- membership lives on the agent, never here.

ALTER TABLE runtime_artifacts
  ADD COLUMN IF NOT EXISTS content_sha256 TEXT;

COMMENT ON COLUMN runtime_artifacts.content_sha256 IS
  'Sha256 of the bundle tarball for bundle kinds; NULL for OCI images, whose digest lives in the immutable reference.';

CREATE TABLE IF NOT EXISTS release_channel_heads (
  channel TEXT NOT NULL CHECK (channel IN ('stable', 'canary')),
  artifact_kind TEXT NOT NULL,
  artifact_id TEXT NOT NULL REFERENCES runtime_artifacts(id),
  updated_at TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (channel, artifact_kind)
);
