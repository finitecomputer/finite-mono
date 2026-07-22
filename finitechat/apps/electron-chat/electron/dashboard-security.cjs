const { URL } = require("node:url");

const DESKTOP_BRIDGE_CONTRACT_VERSION = 1;
const MAX_DESKTOP_ACTION_BYTES = 256 * 1024;
const MAX_DEVICE_LINK_TOKEN_BYTES = 256;
const NOSTR_ACCOUNT_ID = /^[0-9a-f]{64}$/;

const desktopChatActions = new Set([
  "OpenRoom",
  "OpenTopic",
  "OpenChat",
  "CreateTopic",
  "StartTopicChatIntent",
  "RenameChat",
  "SendMessage",
  "SendTopicMessage",
  "SendChatMessage",
  "LoadOlderMessages",
  "MarkRoomRead",
  "SetTyping",
  "RefreshDevices",
]);

function normalizeDashboardBaseUrl(value) {
  const parsed = new URL(String(value ?? ""));
  const loopbackHttp =
    parsed.protocol === "http:"
    && new Set(["127.0.0.1", "[::1]", "localhost"]).has(parsed.hostname);
  if (
    (parsed.protocol !== "https:" && !loopbackHttp)
    || parsed.username
    || parsed.password
    || (parsed.pathname !== "/" && parsed.pathname !== "")
    || parsed.search
    || parsed.hash
  ) {
    throw new Error("Finite dashboard address is invalid");
  }
  return parsed.origin;
}

function dashboardDestination(baseUrl, dashboardPath = "/dashboard") {
  const origin = normalizeDashboardBaseUrl(baseUrl);
  if (
    typeof dashboardPath !== "string"
    || dashboardPath.length > 2_048
    || !dashboardPath.startsWith("/")
    || dashboardPath.startsWith("//")
  ) {
    throw new Error("Finite dashboard path is invalid");
  }
  const destination = new URL(dashboardPath, `${origin}/`);
  if (destination.origin !== origin || destination.username || destination.password) {
    throw new Error("Finite dashboard destination is invalid");
  }
  return destination.toString();
}

function isDashboardOriginUrl(value, baseUrl) {
  try {
    const parsed = new URL(value);
    return parsed.origin === normalizeDashboardBaseUrl(baseUrl)
      && !parsed.username
      && !parsed.password;
  } catch {
    return false;
  }
}

function isDashboardDocumentUrl(value, baseUrl) {
  if (!isDashboardOriginUrl(value, baseUrl)) {
    return false;
  }
  const { pathname } = new URL(value);
  return pathname === "/dashboard" || pathname.startsWith("/dashboard/");
}

function isGoogleWorkspaceStartUrl(value, baseUrl) {
  if (!isDashboardOriginUrl(value, baseUrl)) {
    return false;
  }
  const parsed = new URL(value);
  const machineIds = parsed.searchParams.getAll("machineId");
  const keys = [...parsed.searchParams.keys()];
  const machineId = machineIds[0];
  return (
    parsed.pathname === "/google-workspace/start"
    && !parsed.hash
    && keys.length === 1
    && keys[0] === "machineId"
    && machineIds.length === 1
    && typeof machineId === "string"
    && machineId.length > 0
    && machineId.trim() === machineId
    && Buffer.byteLength(machineId) <= 1_024
    && !/[\p{Cc}\p{Cf}]/u.test(machineId)
  );
}

function isAllowedUnprivilegedNavigation(value, baseUrl) {
  try {
    const parsed = new URL(value);
    if (isDashboardOriginUrl(value, baseUrl)) {
      return true;
    }
    return parsed.protocol === "https:" && !parsed.username && !parsed.password;
  } catch {
    return false;
  }
}

function trustedDashboardIpcFrame({ frameUrl, isMainFrame }, baseUrl) {
  return isMainFrame === true && isDashboardDocumentUrl(frameUrl, baseUrl);
}

function assertDesktopChatAction(action) {
  if (!action || typeof action !== "object" || Array.isArray(action)) {
    throw new Error("Finite Chat action is invalid");
  }
  const keys = Object.keys(action);
  if (keys.length !== 1 || !desktopChatActions.has(keys[0])) {
    throw new Error("Finite Chat action is not available to the dashboard");
  }
  const encoded = JSON.stringify(action);
  if (Buffer.byteLength(encoded) > MAX_DESKTOP_ACTION_BYTES) {
    throw new Error("Finite Chat action is too large");
  }
  return { action, operation: keys[0], encoded };
}

function parseDeviceLinkPublicRequest(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("Finite Chat device link is invalid");
  }
  return {
    link_session_id: deviceLinkToken(value.link_session_id),
    target_device_id: deviceLinkToken(value.target_device_id),
  };
}

function parseDeviceLinkPublicResponse(value, expected) {
  const request = parseDeviceLinkPublicRequest(expected);
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("Finite Chat device-link service returned an invalid response");
  }
  const statuses = new Set([
    "awaiting_claim",
    "awaiting_key_package",
    "joining_rooms",
    "ready",
    "expired",
  ]);
  if (
    value.link_session_id !== request.link_session_id
    || value.target_device_id !== request.target_device_id
    || !statuses.has(value.status)
    || !nonnegativeSafeInteger(value.expires_at_unix_seconds)
    || !nonnegativeSafeInteger(value.room_count)
    || !nonnegativeSafeInteger(value.active_room_count)
    || value.active_room_count > value.room_count
  ) {
    throw new Error("Finite Chat device-link service returned an invalid response");
  }
  return {
    ...request,
    status: value.status,
    expires_at_unix_seconds: value.expires_at_unix_seconds,
    room_count: value.room_count,
    active_room_count: value.active_room_count,
  };
}

function parseAccountBinding(value) {
  if (
    !value
    || typeof value !== "object"
    || Array.isArray(value)
    || Object.keys(value).length !== 1
    || !NOSTR_ACCOUNT_ID.test(value.account_id)
  ) {
    throw new Error("Finite dashboard returned an invalid account binding");
  }
  return { account_id: value.account_id };
}

function parseLocalDaemonIdentity(value, expectedAccountId) {
  const identity = value?.identity;
  if (
    !value
    || typeof value !== "object"
    || Array.isArray(value)
    || !identity
    || typeof identity !== "object"
    || Array.isArray(identity)
    || !NOSTR_ACCOUNT_ID.test(identity.account_id)
    || typeof identity.device_id !== "string"
    || !identity.device_id
    || Buffer.byteLength(identity.device_id) > MAX_DEVICE_LINK_TOKEN_BYTES
    || identity.device_id.trim() !== identity.device_id
    || /[\p{Cc}\p{Cf}]/u.test(identity.device_id)
  ) {
    throw new Error("Finite Chat local service returned an invalid identity");
  }
  if (identity.account_id !== expectedAccountId) {
    throw new Error(
      "This desktop is linked to a different Finite account. Sign back in with that account to use local chat."
    );
  }
  return {
    account_id: identity.account_id,
    device_id: identity.device_id,
  };
}

function deviceLinkToken(value) {
  if (
    typeof value !== "string"
    || !value
    || Buffer.byteLength(value) > MAX_DEVICE_LINK_TOKEN_BYTES
    || value.trim() !== value
    || /[\p{Cc}\p{Cf}]/u.test(value)
  ) {
    throw new Error("Finite Chat device link is invalid");
  }
  return value;
}

function nonnegativeSafeInteger(value) {
  return Number.isSafeInteger(value) && value >= 0;
}

module.exports = {
  DESKTOP_BRIDGE_CONTRACT_VERSION,
  MAX_DESKTOP_ACTION_BYTES,
  assertDesktopChatAction,
  dashboardDestination,
  isAllowedUnprivilegedNavigation,
  isDashboardDocumentUrl,
  isDashboardOriginUrl,
  isGoogleWorkspaceStartUrl,
  normalizeDashboardBaseUrl,
  parseAccountBinding,
  parseDeviceLinkPublicRequest,
  parseDeviceLinkPublicResponse,
  parseLocalDaemonIdentity,
  trustedDashboardIpcFrame,
};
