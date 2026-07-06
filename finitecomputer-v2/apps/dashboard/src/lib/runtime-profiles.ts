import type { RuntimeImageRevision } from "@/lib/control-plane";

export type RuntimeProfileConfig = {
  label?: string;
  image_name?: string;
  image_tag?: string;
  feature_set?: string;
};

type ResolvedRuntimeProfileConfig = {
  label: string;
  image_name: string;
  image_tag: string;
  feature_set: string;
};

type RuntimeProfileCluster = {
  default_runtime_profile?: string;
  runtime_profiles?: Record<string, RuntimeProfileConfig>;
};

type RuntimeProfileWorkload = {
  runtime_profile: string;
};

const DEFAULT_RUNTIME_PROFILE_LABEL = "Hermes Runtime";
const DEFAULT_RUNTIME_IMAGE_NAME = "fc-agent-runtime";
const DEFAULT_RUNTIME_FEATURE_SET = "hermes-local";
const DEFAULT_RUNTIME_PROFILE: RuntimeProfileConfig = {
  label: DEFAULT_RUNTIME_PROFILE_LABEL,
  image_name: DEFAULT_RUNTIME_IMAGE_NAME,
  feature_set: DEFAULT_RUNTIME_FEATURE_SET,
};

function resolveRuntimeProfileId(
  profiles: Record<string, RuntimeProfileConfig>,
  requested: string | undefined,
) {
  const profileId = requested?.trim() || "main";
  if (profileId in profiles) {
    return profileId;
  }

  return profileId;
}

export function runtimeProfilesFor(cluster: RuntimeProfileCluster): Record<string, RuntimeProfileConfig> {
  return (
    cluster.runtime_profiles ?? {
      main: DEFAULT_RUNTIME_PROFILE,
    }
  );
}

export function defaultRuntimeProfileFor(cluster: RuntimeProfileCluster) {
  const profiles = runtimeProfilesFor(cluster);
  const runtimeProfile = resolveRuntimeProfileId(cluster.runtime_profiles ?? profiles, cluster.default_runtime_profile);

  if (!(runtimeProfile in profiles)) {
    throw new Error(`Runtime profile '${runtimeProfile}' is not defined in cluster.json.`);
  }

  return runtimeProfile;
}

export function runtimeProfileConfigFor(cluster: RuntimeProfileCluster, runtimeProfile?: string) {
  const profiles = runtimeProfilesFor(cluster);
  const profileId = resolveRuntimeProfileId(profiles, runtimeProfile ?? defaultRuntimeProfileFor(cluster));
  const profile = profiles[profileId];

  if (!profile) {
    throw new Error(`Runtime profile '${profileId}' is not defined in cluster.json.`);
  }

  return {
    id: profileId,
    config: {
      label: profile.label ?? DEFAULT_RUNTIME_PROFILE_LABEL,
      image_name: profile.image_name ?? DEFAULT_RUNTIME_IMAGE_NAME,
      image_tag: profile.image_tag ?? profileId,
      feature_set: profile.feature_set ?? DEFAULT_RUNTIME_FEATURE_SET,
    } satisfies ResolvedRuntimeProfileConfig,
  };
}

export function runtimeBaseImageFor(cluster: RuntimeProfileCluster, workload: RuntimeProfileWorkload) {
  const { config } = runtimeProfileConfigFor(cluster, workload.runtime_profile);
  return `${config.image_name}:${config.image_tag}`;
}

export function runtimeImageFor(
  runtimeImageRevisions: Record<string, RuntimeImageRevision> | undefined,
  workload: RuntimeProfileWorkload,
  fallbackImage: string,
) {
  const revision = runtimeImageRevisions?.[workload.runtime_profile];
  return revision?.image
    ?? revision?.store_path
    ?? (revision?.image_name && revision?.image_tag ? `${revision.image_name}:${revision.image_tag}` : undefined)
    ?? fallbackImage;
}

export function runtimeImageForProfile(
  cluster: RuntimeProfileCluster,
  runtimeProfileId: string,
  runtimeImageRevisions: Record<string, RuntimeImageRevision> | undefined,
) {
  const { config } = runtimeProfileConfigFor(cluster, runtimeProfileId);
  const fallbackImage = `${config.image_name}:${config.image_tag}`;
  const revision = runtimeImageRevisions?.[runtimeProfileId];

  return revision?.image
    ?? revision?.store_path
    ?? (revision?.image_name && revision?.image_tag ? `${revision.image_name}:${revision.image_tag}` : undefined)
    ?? fallbackImage;
}
