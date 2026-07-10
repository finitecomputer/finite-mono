import { unsealData } from "iron-session";
import { cookies, headers } from "next/headers";

import type { RuntimeImageRevision } from "@/lib/control-plane";
import type { ClusterConfig, DashboardState, DeployMetadata, MachineRecord } from "@/lib/fc-dashboard";
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
  /** WorkOS organization from the signed AuthKit access token, if any. */
  organizationId?: string | null;
  /** Present only on the server; never pass this context to a client component. */
  accessToken?: string;
  source: "workos" | "header" | "dev" | "none";
};

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
      accessToken:
        "accessToken" in auth && typeof auth.accessToken === "string"
          ? auth.accessToken
          : undefined,
      organizationId:
        "accessToken" in auth && typeof auth.accessToken === "string"
          ? workosOrganizationId(auth.accessToken)
          : null,
      source: "workos",
    } satisfies AccountAuthContext;

    if (account.workosUserId) {
      return account;
    }

    return (await getWorkosCookieAccountContext()) ?? account;
  }

  const devAccount = devAccountAuthContext(process.env);
  if (devAccount) {
    return devAccount;
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
    email: null,
    workosUserId: null,
    emailVerified: false,
    source: "none",
  };
}

export function devAccountAuthContext(
  env: Record<string, string | undefined>
): AccountAuthContext | null {
  const email = normalizeEmail(env.FC_DASHBOARD_DEV_EMAIL);
  const workosUserId = env.FC_DASHBOARD_DEV_WORKOS_USER_ID?.trim();
  const accessToken = env.FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN?.trim();
  if (
    env.FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH !== "1" ||
    !email ||
    !workosUserId ||
    !accessToken
  ) {
    return null;
  }
  return {
    email,
    workosUserId,
    emailVerified: true,
    accessToken,
    organizationId: workosOrganizationId(accessToken),
    source: "dev",
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
    accessToken: typeof workosSession.accessToken === "string" ? workosSession.accessToken : undefined,
    organizationId:
      typeof workosSession.accessToken === "string"
        ? workosOrganizationId(workosSession.accessToken)
        : null,
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

function workosOrganizationId(accessToken: string) {
  const [, payload] = accessToken.split(".");
  if (!payload) return null;
  try {
    const parsed = JSON.parse(Buffer.from(payload, "base64url").toString("utf8")) as {
      org_id?: unknown;
    };
    return typeof parsed.org_id === "string" && parsed.org_id.trim()
      ? parsed.org_id.trim()
      : null;
  } catch {
    return null;
  }
}

export async function loadOptionalViewerContext() {
  const account = await getAccountAuthContext();

  try {
    return viewerContextFromModel(await loadDashboardModel(), account);
  } catch (error) {
    if (!localControlPlaneUnavailable(error)) {
      throw error;
    }

    return viewerContextFromModel(emptyDashboardModel(), account);
  }
}

function viewerContextFromModel(model: DashboardModel, account: AccountAuthContext) {
  const email = account.email;
  const isSaasAccount = account.source === "workos" || account.source === "dev";
  const operatorOrganizationId = process.env.FC_WORKOS_OPERATOR_ORG_ID?.trim();
  const dashboardState = {
    ...model.dashboardState,
    admins: Array.from(new Set([...model.dashboardState.admins, ...devAdminEmails()])),
  };
  const isAdmin = isSaasAccount
    ? Boolean(operatorOrganizationId && account.organizationId === operatorOrganizationId)
    : isAdminEmail(dashboardState, email);
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
