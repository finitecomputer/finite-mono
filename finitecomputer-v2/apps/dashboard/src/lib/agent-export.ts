import {
  HostedHermesStatusError, readHostedHermesJson, sendHostedHermesChange,
  type HostedHermesChange, type HostedHermesChangeResult,
} from "./hosted-hermes-status";

/** The whole-Agent export, built by the selected Agent's finite-export
 * plugin and downloaded through the dashboard. The owner starts a survey,
 * picks brains and sites from it, then starts the export. Starts are sent
 * once and never retried; the status read is the truth after any error. */

export type ExportJobState = "running" | "ready" | "failed" | "interrupted";
export type ExportBrain = { id: string; name: string; kind: "personal" | "organization"; bytes: number | null };
export type ExportSite = { slug: string; url: string | null; owned: boolean };
export type ExportSummary = {
  conversations: number; memories: number; skills: number; scheduledJobs: number;
  files: number; fileBytes: number; brains: ExportBrain[]; sites: ExportSite[]; problems: number;
};
export type SurveyJob = { state: ExportJobState; startedAt: string; finishedAt: string | null; summary: ExportSummary | null };
export type ExportJob = {
  state: ExportJobState; startedAt: string; finishedAt: string | null;
  expiresAt: string | null; bytes: number | null; fileName: string | null; problems: number | null;
};
export type ExportStatus = { survey: SurveyJob | null; export: ExportJob | null };
export type ExportChoices = { brains: string[]; sites: string[] };
export type ExportErrorCode =
  | "busy" | "surveyFirst" | "notOffered" | "notEnoughSpace" | "unavailable" | "access" | "unknown";

const STATES = new Set<ExportJobState>(["running", "ready", "failed", "interrupted"]);
const ERROR_CODES = new Set<ExportErrorCode>(["busy", "surveyFirst", "notOffered", "notEnoughSpace"]);
// Matches the plugin: a choice is an exporter argument and never looks like a flag.
const NAME = /^[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}$/;
// The exporter's own name for the zip: its agent's name as a slug, and when.
export const EXPORT_FILE_NAME = /^finite-export-[a-z0-9-]+-\d{8}-\d{4}\.zip$/;
const base = "api/plugins/finite-export";

export class AgentExportError extends Error {
  constructor(readonly code: ExportErrorCode, message: string = code) {
    super(message);
    this.name = "AgentExportError";
  }
}

function invalid(): never {
  throw new HostedHermesStatusError("This agent needs an update before it can export. Try again after it updates.", "unsupported");
}
function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return invalid();
  return value as Record<string, unknown>;
}
function list(value: unknown): unknown[] {
  return Array.isArray(value) && value.length <= 1000 ? value : invalid();
}
function count(value: unknown): number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : invalid();
}
function maybeCount(value: unknown): number | null {
  return value === null || value === undefined ? null : count(value);
}
function time(value: unknown): string {
  return typeof value === "string" && value.length <= 64 && !Number.isNaN(Date.parse(value)) ? value : invalid();
}
function maybeTime(value: unknown): string | null {
  return value === null || value === undefined ? null : time(value);
}
function state(value: unknown): ExportJobState {
  return STATES.has(value as ExportJobState) ? value as ExportJobState : invalid();
}
function name(value: unknown): string {
  return typeof value === "string" && NAME.test(value) ? value : invalid();
}

function brain(value: unknown): ExportBrain {
  const item = record(value);
  const kind = item.kind === "personal" ? "personal" : item.kind === "organization" ? "organization" : invalid();
  const label = typeof item.name === "string" && item.name.trim() && item.name.length <= 200 ? item.name : invalid();
  return { id: name(item.id), name: label, kind, bytes: maybeCount(item.bytes) };
}

function site(value: unknown): ExportSite {
  const item = record(value);
  let url: string | null = null;
  if (typeof item.url === "string") {
    try { url = new URL(item.url).protocol === "https:" ? item.url : null; } catch { url = null; }
  }
  if (typeof item.owned !== "boolean") return invalid();
  return { slug: name(item.slug), url, owned: item.owned };
}

function summary(value: unknown): ExportSummary {
  const item = record(value);
  return {
    conversations: count(item.conversations), memories: count(item.memories), skills: count(item.skills),
    scheduledJobs: count(item.scheduledJobs), files: count(item.files), fileBytes: count(item.fileBytes),
    brains: list(item.brains).map(brain), sites: list(item.sites).map(site), problems: count(item.problems),
  };
}

function surveyJob(value: unknown): SurveyJob | null {
  if (value === null) return null;
  const item = record(value);
  const jobState = state(item.state);
  return {
    state: jobState, startedAt: time(item.startedAt), finishedAt: maybeTime(item.finishedAt),
    summary: jobState === "ready" ? summary(item.summary) : null,
  };
}

function exportJob(value: unknown): ExportJob | null {
  if (value === null) return null;
  const item = record(value);
  const jobState = state(item.state);
  const ready = jobState === "ready";
  const fileName = ready ? item.fileName : null;
  if (ready && (typeof fileName !== "string" || fileName.length > 255 || !EXPORT_FILE_NAME.test(fileName))) return invalid();
  return {
    state: jobState, startedAt: time(item.startedAt), finishedAt: maybeTime(item.finishedAt),
    expiresAt: ready ? time(item.expiresAt) : null, bytes: ready ? count(item.bytes) : null,
    fileName: fileName as string | null, problems: ready ? count(item.problems) : null,
  };
}

export function parseExportStatus(value: unknown): ExportStatus {
  const data = record(value);
  if (data.version !== 1) return invalid();
  return { survey: surveyJob(data.survey ?? null), export: exportJob(data.export ?? null) };
}

type Dependencies = {
  read: (runtimeId: string, path: string, signal: AbortSignal) => Promise<unknown>;
  send: (runtimeId: string, path: string, change: HostedHermesChange, signal: AbortSignal) => Promise<HostedHermesChangeResult>;
};
const browser: Dependencies = { read: readHostedHermesJson, send: sendHostedHermesChange };

export async function readExportStatus(runtimeId: string, signal: AbortSignal, dependencies = browser) {
  return parseExportStatus(await dependencies.read(runtimeId, `${base}/status`, signal));
}

/** Send once. A grant or access error means the agent was not reached;
 * anything else after sending leaves the outcome to the next status read. */
async function change(dependencies: Dependencies, runtimeId: string, path: string, body: HostedHermesChange, signal: AbortSignal) {
  let result: HostedHermesChangeResult;
  try {
    result = await dependencies.send(runtimeId, path, body, signal);
  } catch (error) {
    if (signal.aborted) throw error;
    if (error instanceof HostedHermesStatusError && error.kind === "access") throw new AgentExportError("access", error.message);
    throw new AgentExportError("unknown");
  }
  if (result.status >= 200 && result.status < 300) return result.body;
  const code = (result.body as { error?: unknown } | null)?.error;
  if (typeof code === "string" && ERROR_CODES.has(code as ExportErrorCode)) throw new AgentExportError(code as ExportErrorCode);
  // Native auth refused it before the plugin ran; nothing started.
  if (result.status === 401 || result.status === 403) throw new AgentExportError("access", "Access to this agent is no longer available.");
  // An Agent without the plugin answers 404 or 405.
  if (result.status === 404 || result.status === 405) throw new AgentExportError("unavailable");
  throw new AgentExportError("unknown");
}

export async function startSurvey(runtimeId: string, signal: AbortSignal, dependencies = browser) {
  await change(dependencies, runtimeId, `${base}/survey`, { method: "POST" }, signal);
}

export async function startExport(runtimeId: string, choices: ExportChoices, signal: AbortSignal, dependencies = browser) {
  if (![...choices.brains, ...choices.sites].every(item => NAME.test(item))) throw new AgentExportError("notOffered");
  await change(dependencies, runtimeId, `${base}/export`,
    { method: "POST", body: { brains: choices.brains, sites: choices.sites } }, signal);
}

/** Cancels a running export, or deletes a finished one. */
export async function deleteExport(runtimeId: string, signal: AbortSignal, dependencies = browser) {
  await change(dependencies, runtimeId, `${base}/export`, { method: "DELETE" }, signal);
}

/** `/export` typed in web chat opens the Export page instead of reaching
 * the agent. Hermes' own `/export` is CLI-only, so chat loses nothing. */
export function isExportCommand(text: string) {
  return text.trim() === "/export";
}

export function exportPagePath(machineId: string) {
  return `/dashboard/machines/${encodeURIComponent(machineId)}/export`;
}

/** Same-origin download: the dashboard fetches the file from the Agent as
 * the signed-in owner and streams it through. */
export function exportDownloadPath(runtimeId: string) {
  return `/api/agents/${encodeURIComponent(runtimeId)}/export/download`;
}

/** What the person picks first: their own brains and the sites the agent
 * owns. Organization brains and sites someone else owns wait for a choice. */
export function defaultChoices(summary: ExportSummary): ExportChoices {
  return {
    brains: summary.brains.filter(item => item.kind === "personal").map(item => item.id),
    sites: summary.sites.filter(item => item.owned).map(item => item.slug),
  };
}

/** About how much the export copies: the files plus the brains chosen. */
export function estimatedBytes(summary: ExportSummary, choices: ExportChoices) {
  return summary.fileBytes + summary.brains
    .filter(item => choices.brains.includes(item.id))
    .reduce((total, item) => total + (item.bytes ?? 0), 0);
}

export function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit++; }
  return `${value >= 10 ? Math.round(value) : value.toFixed(1)} ${units[unit]}`;
}

export function exportErrorMessage(code: ExportErrorCode, agentName: string) {
  switch (code) {
    case "busy": return `${agentName} is already working on an export. Wait for it to finish.`;
    case "surveyFirst":
    case "notOffered": return `What ${agentName} has changed. Check again, then choose what to include.`;
    case "notEnoughSpace": return `There isn’t enough free space on ${agentName}’s computer for this export right now. Leave out a brain, or try again later.`;
    case "unavailable": return `${agentName} needs an update before it can export. Try again after it updates.`;
    case "access": return "Access to this agent is no longer available.";
    case "unknown": return `${agentName} didn’t confirm the request. Refresh to check.`;
  }
}
