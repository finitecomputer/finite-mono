use super::*;

impl CoreStore {
    pub async fn runtime_artifact(&self, id: &str) -> CoreResult<Option<RuntimeArtifact>> {
        let id = trim_to_option(Some(id)).ok_or(CoreError::MissingRuntimeArtifactId)?;
        let client = self.connection().await?;
        select_runtime_artifact(&**client, &id).await
    }

    pub async fn upsert_runtime_artifact(
        &self,
        input: UpsertRuntimeArtifactInput,
    ) -> CoreResult<RuntimeArtifact> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let artifact = postgres_upsert_runtime_artifact(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(artifact)
    }
}

pub(super) async fn select_runtime_artifact<C>(
    client: &C,
    artifact_id: &str,
) -> CoreResult<Option<RuntimeArtifact>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT id, kind, reference, version_label, source_git_sha, finitec_version,
                    hermes_source_ref, finite_platform_plugin_ref, state_schema_version,
                    base_image, recover_known_good_chat, canary_runtime_id,
                    core_rfc3339(created_at) AS created_at, core_rfc3339(promoted_at) AS promoted_at, core_rfc3339(retired_at) AS retired_at
             FROM runtime_artifacts WHERE id = $1",
            &[&artifact_id],
        )
        .await
        .map_err(store_error)?
        .map(|row| runtime_artifact_from_row(&row))
        .transpose()
}

pub(super) async fn select_latest_launchable_runtime_artifact<C>(
    client: &C,
) -> CoreResult<RuntimeArtifact>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT id, kind, reference, version_label, source_git_sha, finitec_version,
                    hermes_source_ref, finite_platform_plugin_ref, state_schema_version,
                    base_image, recover_known_good_chat, canary_runtime_id,
                    core_rfc3339(created_at) AS created_at, core_rfc3339(promoted_at) AS promoted_at, core_rfc3339(retired_at) AS retired_at
             FROM runtime_artifacts
             WHERE promoted_at IS NOT NULL AND retired_at IS NULL AND kind = 'oci_image'
             -- Qualified: a bare name here would bind the rendered-text output
             -- column and sort lexicographically, which is not chronological
             -- once fractional seconds vary.
             ORDER BY runtime_artifacts.promoted_at DESC,
                      runtime_artifacts.created_at DESC, id DESC
             LIMIT 1",
            &[],
        )
        .await
        .map_err(store_error)?
        .map(|row| runtime_artifact_from_row(&row))
        .transpose()?
        .filter(|artifact| runtime_artifact_reference_is_immutable_oci(&artifact.reference))
        .ok_or(CoreError::RuntimeArtifactUnavailable)
}

pub(super) fn ensure_artifact_launchable(artifact: &RuntimeArtifact) -> CoreResult<()> {
    if artifact.promoted_at.is_none() {
        return Err(CoreError::RuntimeArtifactNotPromoted);
    }
    if artifact.retired_at.is_some() {
        return Err(CoreError::RuntimeArtifactRetired);
    }
    Ok(())
}

pub(super) fn ensure_runtime_upgrade_target_compatible(
    runtime: &AgentRuntime,
    artifact: &RuntimeArtifact,
) -> CoreResult<()> {
    if artifact.canary_runtime_id.as_deref() == Some(runtime.id.as_str()) {
        if artifact.retired_at.is_some() {
            return Err(CoreError::RuntimeArtifactRetired);
        }
    } else {
        ensure_artifact_launchable(artifact)?;
    }
    ensure_runtime_upgrade_target_material(runtime, artifact)
}

pub(super) fn ensure_runtime_upgrade_target_material(
    runtime: &AgentRuntime,
    artifact: &RuntimeArtifact,
) -> CoreResult<()> {
    if artifact.kind != crate::RuntimeArtifactKind::OciImage
        || !runtime_artifact_reference_is_immutable_oci(&artifact.reference)
    {
        return Err(CoreError::RuntimeUpgradeUnsupported);
    }
    if runtime.state_schema_version.as_deref() != Some(artifact.state_schema_version.as_str()) {
        return Err(CoreError::RuntimeUpgradeStateSchemaIncompatible);
    }
    Ok(())
}

async fn postgres_upsert_runtime_artifact<C>(
    client: &C,
    input: UpsertRuntimeArtifactInput,
) -> CoreResult<RuntimeArtifact>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let id = trim_to_option(Some(&input.id)).ok_or(CoreError::MissingRuntimeArtifactId)?;
    let reference =
        trim_to_option(Some(&input.reference)).ok_or(CoreError::MissingRuntimeArtifactReference)?;
    let version_label = trim_to_option(Some(&input.version_label))
        .ok_or(CoreError::MissingRuntimeArtifactVersionLabel)?;
    let state_schema_version = trim_to_option(Some(&input.state_schema_version))
        .ok_or(CoreError::MissingRuntimeArtifactStateSchemaVersion)?;
    // Lock the existing row (if any) so created_at/promoted_at/retired_at are
    // preserved deterministically under concurrent upserts.
    let existing = client
        .query_opt(
            "SELECT id, kind, reference, version_label, source_git_sha, finitec_version,
                    hermes_source_ref, finite_platform_plugin_ref, state_schema_version,
                    base_image, recover_known_good_chat, canary_runtime_id,
                    core_rfc3339(created_at) AS created_at, core_rfc3339(promoted_at) AS promoted_at, core_rfc3339(retired_at) AS retired_at
             FROM runtime_artifacts WHERE id = $1 FOR UPDATE",
            &[&id],
        )
        .await
        .map_err(store_error)?
        .map(|row| runtime_artifact_from_row(&row))
        .transpose()?;
    let existing_created_at = existing
        .as_ref()
        .map(|artifact| artifact.created_at.clone());
    let existing_promoted_at = existing
        .as_ref()
        .and_then(|artifact| artifact.promoted_at.clone());
    let existing_retired_at = existing
        .as_ref()
        .and_then(|artifact| artifact.retired_at.clone());
    let created_at = existing_created_at.unwrap_or_else(|| now.clone());
    let promoted_at = if input.promoted {
        existing_promoted_at.or_else(|| Some(now.clone()))
    } else {
        existing_promoted_at
    };
    let artifact = RuntimeArtifact {
        id: id.clone(),
        kind: input.kind,
        reference,
        version_label,
        source_git_sha: trim_to_option(input.source_git_sha.as_deref()),
        finitec_version: trim_to_option(input.finitec_version.as_deref()),
        hermes_source_ref: trim_to_option(input.hermes_source_ref.as_deref()),
        finite_platform_plugin_ref: trim_to_option(input.finite_platform_plugin_ref.as_deref()),
        state_schema_version,
        base_image: trim_to_option(input.base_image.as_deref()),
        canary_runtime_id: trim_to_option(input.canary_runtime_id.as_deref()),
        recover_known_good_chat: input.recover_known_good_chat,
        created_at,
        promoted_at,
        retired_at: existing_retired_at,
    };
    if artifact.canary_runtime_id.is_some() {
        // A canary must never enter default launch selection, even through
        // an idempotent retry of an already-promoted record.
        if artifact.promoted_at.is_some() {
            return Err(CoreError::RuntimeArtifactImmutable);
        }
        if !runtime_artifact_reference_is_immutable_oci(&artifact.reference) {
            return Err(CoreError::RuntimeUpgradeUnsupported);
        }
    }
    if let Some(existing) = existing.as_ref() {
        let referenced: bool = client
            .query_one(
                "SELECT EXISTS (
                   SELECT 1 FROM agent_runtimes WHERE runtime_artifact_id = $1
                 ) AS referenced",
                &[&id],
            )
            .await
            .map_err(store_error)?
            .get("referenced");
        if (existing.promoted_at.is_some() || existing.canary_runtime_id.is_some() || referenced)
            && !runtime_artifact_material_matches(existing, &artifact)
        {
            return Err(CoreError::RuntimeArtifactImmutable);
        }
    }
    let row = client
        .query_one(
            "INSERT INTO runtime_artifacts (
               id, kind, reference, version_label, source_git_sha, finitec_version,
               hermes_source_ref, finite_platform_plugin_ref, state_schema_version,
               base_image, recover_known_good_chat, canary_runtime_id, created_at, promoted_at, retired_at
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                     $11, $15, $12::text::timestamptz, $13::text::timestamptz,
                     $14::text::timestamptz)
             ON CONFLICT (id) DO UPDATE SET
               kind = EXCLUDED.kind,
               reference = EXCLUDED.reference,
               version_label = EXCLUDED.version_label,
               source_git_sha = EXCLUDED.source_git_sha,
               finitec_version = EXCLUDED.finitec_version,
               hermes_source_ref = EXCLUDED.hermes_source_ref,
               finite_platform_plugin_ref = EXCLUDED.finite_platform_plugin_ref,
               state_schema_version = EXCLUDED.state_schema_version,
               base_image = EXCLUDED.base_image,
               recover_known_good_chat = EXCLUDED.recover_known_good_chat,
               canary_runtime_id = EXCLUDED.canary_runtime_id,
               promoted_at = EXCLUDED.promoted_at,
               retired_at = EXCLUDED.retired_at
             RETURNING id, kind, reference, version_label, source_git_sha, finitec_version,
                       hermes_source_ref, finite_platform_plugin_ref, state_schema_version,
                       base_image, recover_known_good_chat, canary_runtime_id,
                       core_rfc3339(created_at) AS created_at, core_rfc3339(promoted_at) AS promoted_at, core_rfc3339(retired_at) AS retired_at",
            &[
                &artifact.id,
                &artifact.kind.as_str(),
                &artifact.reference,
                &artifact.version_label,
                &artifact.source_git_sha,
                &artifact.finitec_version,
                &artifact.hermes_source_ref,
                &artifact.finite_platform_plugin_ref,
                &artifact.state_schema_version,
                &artifact.base_image,
                &artifact.recover_known_good_chat,
                &artifact.created_at,
                &artifact.promoted_at,
                &artifact.retired_at,
                &artifact.canary_runtime_id,
            ],
        )
        .await
        .map_err(store_error)?;
    runtime_artifact_from_row(&row)
}
