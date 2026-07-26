"use strict";

const DEFAULT_POLL_INTERVAL_MS = 750;
const DEFAULT_ENROLLMENT_TIMEOUT_MS = 30 * 60 * 1_000;

function defaultDelay(delayMs) {
  return new Promise((resolve) => setTimeout(resolve, delayMs));
}

function manifestKey(manifest) {
  return `${manifest.bootstrap_id}\u0000${manifest.room_id}\u0000${manifest.manifest_sha256}`;
}

function targetEnrollmentIsReady(
  state,
  expectedAccountId,
  expectedDeviceId,
  expectedManifests
) {
  if (
    !state
    || typeof state !== "object"
    || !Array.isArray(expectedManifests)
    || expectedManifests.length < 1
    || state.identity?.account_id !== expectedAccountId
    || state.identity?.device_id !== expectedDeviceId
    || !Array.isArray(state.rooms)
    || !Array.isArray(state.device_link_bootstrap_receipts)
    || !state.paired_agent
  ) {
    return false;
  }
  const actual = new Set(state.device_link_bootstrap_receipts.map(manifestKey));
  if (!expectedManifests.every((manifest) => actual.has(manifestKey(manifest)))) {
    return false;
  }
  const canonicalRoomId = state.paired_agent.canonical_room_id;
  return state.rooms.some((room) =>
    room.room_id === canonicalRoomId && room.is_agent_chat === true
  );
}

async function waitForTargetEnrollment({
  expectedAccountId,
  expectedDeviceId,
  expectedManifests,
  readState,
  assertActive = () => {},
  reportStatus = () => {},
  now = () => Date.now(),
  delay = defaultDelay,
  pollIntervalMs = DEFAULT_POLL_INTERVAL_MS,
  timeoutMs = DEFAULT_ENROLLMENT_TIMEOUT_MS,
}) {
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0) {
    throw new Error("Finite Chat enrollment timeout is invalid");
  }
  const deadline = now() + timeoutMs;
  while (true) {
    assertActive();
    if (now() >= deadline) {
      throw new Error(
        "This desktop could not finish syncing its complete chat history. Try again."
      );
    }
    const state = await readState();
    assertActive();
    if (
      targetEnrollmentIsReady(
        state,
        expectedAccountId,
        expectedDeviceId,
        expectedManifests
      )
    ) {
      return state;
    }
    reportStatus({ status: "joining_rooms" });
    await delay(pollIntervalMs);
  }
}

async function waitForSourceEnrollment({
  request,
  pollEnrollment,
  parseResponse,
  isRetryableError = () => true,
  assertActive = () => {},
  reportStatus = () => {},
  now = () => Date.now(),
  delay = defaultDelay,
  pollIntervalMs = DEFAULT_POLL_INTERVAL_MS,
  timeoutMs = DEFAULT_ENROLLMENT_TIMEOUT_MS,
}) {
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0) {
    throw new Error("Finite Chat enrollment timeout is invalid");
  }
  const deadline = now() + timeoutMs;
  while (true) {
    assertActive();
    if (now() >= deadline) {
      throw new Error(
        "This desktop could not finish syncing its complete chat history. Try again."
      );
    }
    try {
      const current = parseResponse(await pollEnrollment(request), request);
      assertActive();
      if (current.status === "ready") return current;
      if (
        current.status !== "awaiting_key_package"
        && current.status !== "joining_rooms"
      ) {
        throw new Error("Finite Chat enrollment returned an invalid state");
      }
      reportStatus({ status: "joining_rooms" });
    } catch (error) {
      // Enrollment is durable. A transient dashboard failure is not evidence
      // that source-side fanout stopped, so keep the explicit terminal bound.
      assertActive();
      if (!isRetryableError(error)) throw error;
      if (now() >= deadline) throw error;
    }
    await delay(pollIntervalMs);
  }
}

/**
 * Drive the authenticated source while the Rust target completes the bounded
 * NIP-AB secret grant.
 *
 * Hosted Device reconciles the target offer when its status endpoint is
 * called. The status pump must therefore run concurrently with the target:
 * awaiting target completion before polling creates a deterministic deadlock.
 *
 * This function ends at the local commit point: the target has durably stored
 * one encrypted identity envelope containing the secret and resume grant.
 * Publishing NIP Complete happens afterward as best-effort cleanup and cannot
 * retract that durable success.
 */
async function completeDeviceSecretGrant({
  link,
  request,
  approved,
  pollStatus,
  parseResponse,
  isRetryableError = () => true,
  assertActive = () => {},
  reportStatus = () => {},
  now = () => Date.now(),
  delay = defaultDelay,
  pollIntervalMs = DEFAULT_POLL_INTERVAL_MS,
}) {
  if (!approved?.source_descriptor) {
    throw new Error("Finite Chat pairing approval omitted its source descriptor");
  }
  if (!Number.isSafeInteger(pollIntervalMs) || pollIntervalMs < 0) {
    throw new Error("Finite Chat pairing poll interval is invalid");
  }

  link.acceptSourceDescriptor(approved.source_descriptor);
  assertActive();

  let targetComplete = false;
  let targetFailure = null;
  let targetEnrollmentGrant = null;
  const targetDurability = Promise.resolve(link.durable).then(
    (grant) => {
      targetEnrollmentGrant = grant;
      targetComplete = true;
    },
    (error) => {
      targetFailure = error;
      targetComplete = true;
    }
  );
  // The child continues with NIP Complete after the durable commit. Keep its
  // cleanup observable to the supervisor without making enrollment depend on
  // a relay round-trip which may fail after the target is already recoverable.
  Promise.resolve(link.completion).catch(() => {});

  let current = parseResponse(approved, request);
  while (!targetComplete) {
    assertActive();
    if (
      current.status === "expired"
      || now() >= current.expires_at_unix_seconds * 1_000
    ) {
      throw new Error(
        "This desktop's secure Device grant expired. Restart Finite and try again."
      );
    }
    reportStatus({ status: "linking" });

    // Poll immediately after delivering the descriptor. The target publishes
    // its offer asynchronously, so an early awaiting_offer response is normal.
    const poll = Promise.resolve()
      .then(() => pollStatus(request))
      .then(
        (value) => ({ type: "status", value }),
        (error) => ({ type: "error", error })
      );
    const outcome = await Promise.race([
      targetDurability.then(() => ({ type: "durable" })),
      poll,
    ]);
    if (outcome.type === "durable") {
      break;
    }
    if (outcome.type === "status") {
      current = parseResponse(outcome.value, request);
    } else {
      const { error } = outcome;
      // A transient dashboard failure must not abandon a target which may
      // already be holding the authenticated response. The grant deadline is
      // the explicit bound; target failure remains immediately observable.
      if (targetComplete) break;
      if (!isRetryableError(error)) throw error;
      if (now() >= current.expires_at_unix_seconds * 1_000) throw error;
    }
    if (targetComplete) break;
    await Promise.race([targetDurability, delay(pollIntervalMs)]);
  }

  await targetDurability;
  if (targetFailure) throw targetFailure;
  if (!targetEnrollmentGrant) {
    throw new Error("Finite Chat pairing omitted its enrollment capability");
  }
  return targetEnrollmentGrant;
}

module.exports = {
  completeDeviceSecretGrant,
  targetEnrollmentIsReady,
  waitForSourceEnrollment,
  waitForTargetEnrollment,
};
