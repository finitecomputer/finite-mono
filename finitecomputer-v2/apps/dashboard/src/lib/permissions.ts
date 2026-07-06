import type { DashboardState, MachineRecord, SiteAuthMode } from "./fc-dashboard";

export type PublishedAuthConfig = {
  mode?: SiteAuthMode;
  owner_email?: string;
  emails?: string[];
  org_domain?: string;
};

export function normalizeEmail(value?: string | null) {
  const email = value?.trim().toLowerCase();
  return email ? email : null;
}

export function isAdminEmail(state: Pick<DashboardState, "admins">, email: string | null) {
  if (!email) {
    return false;
  }

  return state.admins.some((admin) => normalizeEmail(admin) === email);
}

export function ownsMachine(machine: MachineRecord, email: string | null) {
  if (!email) {
    return false;
  }

  return (
    normalizeEmail(machine.ownerEmail) === email ||
    normalizeEmail(machine.invite?.email) === email ||
    normalizeEmail(machine.workload.owner_email) === email
  );
}

export function canOperateMachineRecord(
  machine: MachineRecord,
  state: Pick<DashboardState, "admins">,
  email: string | null,
) {
  return isAdminEmail(state, email) || ownsMachine(machine, email);
}

export function visibleMachinesForViewer(
  machines: MachineRecord[],
  state: Pick<DashboardState, "admins">,
  email: string | null,
) {
  if (isAdminEmail(state, email)) {
    return machines;
  }

  return machines.filter((machine) => ownsMachine(machine, email));
}

function emailDomain(email: string | null) {
  if (!email) {
    return null;
  }

  const [, domain] = email.split("@");
  return domain?.trim().toLowerCase() || null;
}

export function canAccessPublishedAuth(
  auth: PublishedAuthConfig | undefined,
  fallbackOwnerEmail: string | null | undefined,
  viewerEmail: string | null,
) {
  const mode = auth?.mode ?? "self";

  if (mode === "public") {
    return true;
  }

  if (!viewerEmail) {
    return false;
  }

  if (mode === "self") {
    return normalizeEmail(auth?.owner_email) === viewerEmail || normalizeEmail(fallbackOwnerEmail) === viewerEmail;
  }

  if (mode === "emails") {
    return (auth?.emails ?? []).some((email) => normalizeEmail(email) === viewerEmail);
  }

  if (mode === "org") {
    const viewerDomain = emailDomain(viewerEmail);
    const allowedDomain = auth?.org_domain?.trim().toLowerCase() || null;
    return Boolean(viewerDomain && allowedDomain && viewerDomain === allowedDomain);
  }

  return false;
}
