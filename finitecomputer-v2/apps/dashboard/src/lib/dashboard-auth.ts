import { unsealData } from "iron-session";
import { cookies, headers } from "next/headers";

import type { RuntimeImageRevision } from "@/lib/control-plane";
import type { ClusterConfig, DashboardState, DeployMetadata, MachineRecord } from "@/lib/fc-dashboard";
import { isCoreAdminEmail } from "@/lib/admin-ops";
import { isAdminEmail, normalizeEmail, ownsMachine, visibleMachinesForViewer } from "@/lib/permissions";
import { workosAuthStatus, workosSessionCookieName } from "@/lib/workos-auth";

const EMAIL_HEADER_NAMES = [
  "x-auth-request-email",
  "x-forwarded-email",
  "x-auth-request-user",
  "x-forwarded-user",
];

export type AccountAuthContext = {
  email: string | null;
  workosUserId: string | null;
  emailVerified: boolean;
  source: "workos" | "header" | "dev" | "none";
};

/**
 * Return the launch code used by the explicit local-development account.
 *
 * The code is never accepted from a browser field.  It is available only when
 * the request resolved to the configured dev account and the same opt-in that
 * permits dev-account Core authentication is enabled.  WorkOS sessions always
 * take the normal billing path, even if a developer accidentally leaves the
 * dev launch-code environment variable set.
 */
export function dashboardDevLaunchCode(
  account: AccountAuthContext,
  env: Record<string, string | undefined> = process.env
) {
  if (
    account.source !== "dev" ||
    !account.email ||
    !account.workosUserId ||
    !account.emailVerified ||
    env.FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH !== "1"
  ) {
    return "";
  }
  return env.FC_DASHBOARD_DEV_LAUNCH_CODE?.trim() ?? "";
}

type WorkosSessionCookie = {
  accessToken?: unknown;
  user?: {
    id?: unknown;
    email?: unknown;
    emailVerified?: unknown;
  };
};

type DashboardModel = {
  cluster: ClusterConfig;
  dashboardState: DashboardState;
  deployMetadata: DeployMetadata | null;
  runtimeImageRevisions: Record<string, RuntimeImageRevision>;
  machines: MachineRecord[];
};

async function loadDashboardModel() {
  const dashboard = await import("@/lib/fc-dashboard");
  return dashboard.loadDashboardModel();
}

export async function getAuthenticatedEmail() {
  const account = await getAccountAuthContext();
  return account.email;
}

export async function getAccountAuthContext(): Promise<AccountAuthContext> {
  const workosStatus = workosAuthStatus();

  if (workosStatus.enabled && !workosStatus.ready) {
    return {
      email: null,
      workosUserId: null,
      emailVerified: false,
      source: "none",
    };
  }

  if (workosStatus.ready) {
    const { withAuth } = await import("@workos-inc/authkit-nextjs");
    const auth = await withAuth().catch(() => ({ user: null }));
    const account = {
      email: normalizeEmail(auth.user?.email),
      workosUserId: auth.user?.id ?? null,
      emailVerified: auth.user?.emailVerified ?? false,
      source: "workos",
    } satisfies AccountAuthContext;

    if (account.workosUserId) {
      return account;
    }

    return (await getWorkosCookieAccountContext()) ?? account;
  }

  const devEmail = normalizeEmail(process.env.FC_DASHBOARD_DEV_EMAIL);
  const devWorkosUserId = process.env.FC_DASHBOARD_DEV_WORKOS_USER_ID?.trim();
  if (devEmail && devWorkosUserId) {
    return {
      email: devEmail,
      workosUserId: devWorkosUserId,
      emailVerified: true,
      source: "dev",
    };
  }

  const headerList = await headers();

  for (const name of EMAIL_HEADER_NAMES) {
    const email = normalizeEmail(headerList.get(name));
    if (email) {
      return {
        email,
        workosUserId: null,
        emailVerified: false,
        source: "header",
      };
    }
  }

  return {
    email: devEmail,
    workosUserId: null,
    emailVerified: false,
    source: devEmail ? "dev" : "none",
  };
}

export function accountFromWorkosSessionCookie(
  session: unknown,
  nowMs = Date.now()
): AccountAuthContext | null {
  if (!session || typeof session !== "object") {
    return null;
  }

  const workosSession = session as WorkosSessionCookie;
  const user = workosSession.user;
  if (!user || typeof user !== "object") {
    return null;
  }

  if (typeof workosSession.accessToken === "string" && accessTokenExpired(workosSession.accessToken, nowMs)) {
    return null;
  }

  return {
    email: normalizeEmail(typeof user.email === "string" ? user.email : null),
    workosUserId: typeof user.id === "string" ? user.id : null,
    emailVerified: user.emailVerified === true,
    source: "workos",
  };
}

async function getWorkosCookieAccountContext() {
  const cookieName = workosSessionCookieName();
  const cookie = (await cookies()).get(cookieName)?.value;
  const password = process.env.WORKOS_COOKIE_PASSWORD?.trim();
  if (!cookie || !password) {
    return null;
  }

  try {
    return accountFromWorkosSessionCookie(
      await unsealData(cookie, { password })
    );
  } catch (error) {
    console.warn("Could not read WorkOS session cookie", {
      error: error instanceof Error ? error.message : String(error),
    });
    return null;
  }
}

function accessTokenExpired(accessToken: string, nowMs: number) {
  const [, payload] = accessToken.split(".");
  if (!payload) {
    return true;
  }

  try {
    const parsed = JSON.parse(Buffer.from(payload, "base64url").toString("utf8")) as {
      exp?: unknown;
    };
    return typeof parsed.exp === "number" && parsed.exp * 1000 <= nowMs;
  } catch {
    return true;
  }
}

export async function loadOptionalViewerContext() {
  const account = await getAccountAuthContext();

  try {
    return viewerContextFromModel(await loadDashboardModel(), account.email);
  } catch (error) {
    if (!localControlPlaneUnavailable(error)) {
      throw error;
    }

    return viewerContextFromModel(emptyDashboardModel(), account.email);
  }
}

function viewerContextFromModel(model: DashboardModel, email: string | null) {
  const dashboardState = {
    ...model.dashboardState,
    admins: Array.from(new Set([...model.dashboardState.admins, ...devAdminEmails()])),
  };
  const isAdmin = isAdminEmail(dashboardState, email) || isCoreAdminEmail(email);
  const invitedMachines = model.machines.filter((machine) => ownsMachine(machine, email));
  const visibleMachines = visibleMachinesForViewer(model.machines, dashboardState, email);

  return {
    ...model,
    dashboardState,
    email,
    isAdmin,
    invitedMachines,
    visibleMachines,
  };
}

function emptyDashboardModel(): DashboardModel {
  const admins = devAdminEmails();
  const cluster = {
    enabled: false,
    cluster_name: process.env.FC_DASHBOARD_CLUSTER_NAME?.trim() || "finite-computer",
    base_domain: process.env.FC_BASE_DOMAIN?.trim() || "finite.computer",
    dashboard: {
      admins,
    },
  } satisfies ClusterConfig;

  return {
    cluster,
    dashboardState: {
      admins,
      invites: [],
    },
    deployMetadata: null,
    runtimeImageRevisions: {},
    machines: [],
  };
}

function devAdminEmails() {
  return (process.env.FC_DASHBOARD_DEV_ADMIN_EMAILS ?? "")
    .split(",")
    .map((email) => email.trim())
    .filter(Boolean);
}

function localControlPlaneUnavailable(error: unknown) {
  if (!(error instanceof Error)) {
    return false;
  }

  return error.message === "finited binary not found" || error.message.includes("agent-cluster/cluster.json");
}
