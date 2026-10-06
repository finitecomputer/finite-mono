import { HostedHermesStatusError, readHostedHermesJson } from "./hosted-hermes-status";
import { parseOwnerNpub, parsePersonalAgentConsent } from "./personal-brain-setup";

/// Browser side of Personal Brain setup: the person's key from the dashboard,
/// the selected Agent's consent from its runtime, then the owner-signed
/// creation through the dashboard. Each step uses fresh authorization.

export const UPDATE_AGENT_MESSAGE = "Update this agent to set up a Personal Brain.";
export const CONSENT_REFUSED_MESSAGE = "This agent could not consent to a Personal Brain right now.";
const SETUP_FAILED = "Your Personal Brain could not be set up right now.";
const SETUP_ROUTE = "/api/brain/personal";

export class PersonalBrainSetupError extends Error {
  /** The Brain refused creation as a conflict; it may already exist. */
  constructor(message: string, readonly mayExist = false) {
    super(message);
    this.name = "PersonalBrainSetupError";
  }
}

type Dependencies = {
  fetch: (input: string, init: RequestInit) => Promise<Response>;
  readAgentJson: (runtimeId: string, path: string, signal: AbortSignal) => Promise<unknown>;
};

const browserDependencies: Dependencies = {
  fetch: (input, init) => fetch(input, init),
  readAgentJson: readHostedHermesJson,
};

export async function setUpPersonalBrain(
  runtimeId: string,
  signal: AbortSignal,
  dependencies: Dependencies = browserDependencies,
): Promise<string> {
  const ownerNpub = parseOwnerNpub(await setupRoute(dependencies, { signal }));
  if (!ownerNpub) throw new PersonalBrainSetupError(SETUP_FAILED);
  let response: unknown;
  try {
    response = await dependencies.readAgentJson(
      runtimeId,
      `api/plugins/finite-brain/personal-agent-consent/${ownerNpub}`,
      signal,
    );
  } catch (error) {
    // A runtime without the consent route (404) or with an older response.
    if (error instanceof HostedHermesStatusError && error.kind === "unsupported") {
      throw new PersonalBrainSetupError(UPDATE_AGENT_MESSAGE);
    }
    // The page loaded through the same grant, so a refusal here comes from
    // the Agent's consent command rather than lost access.
    if (error instanceof HostedHermesStatusError && error.kind === "access") {
      throw new PersonalBrainSetupError(CONSENT_REFUSED_MESSAGE);
    }
    throw error;
  }
  const setup = parsePersonalAgentConsent(response, ownerNpub);
  if (!setup) throw new PersonalBrainSetupError(UPDATE_AGENT_MESSAGE);
  const created = await setupRoute(dependencies, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(setup),
    signal,
  });
  const brainId = (created as { brainId?: unknown } | null)?.brainId;
  return typeof brainId === "string" ? brainId : setup.brainId;
}

async function setupRoute(dependencies: Dependencies, init: RequestInit): Promise<unknown> {
  const response = await dependencies.fetch(SETUP_ROUTE, {
    ...init,
    cache: "no-store",
    credentials: "same-origin",
    redirect: "error",
  });
  const body: unknown = await response.json().catch(() => null);
  if (!response.ok) {
    const message = (body as { error?: unknown } | null)?.error;
    throw new PersonalBrainSetupError(
      typeof message === "string" && message.trim() ? message : SETUP_FAILED,
      init.method === "POST" && response.status === 409,
    );
  }
  return body;
}
