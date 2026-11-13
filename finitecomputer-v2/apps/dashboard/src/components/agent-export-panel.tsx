"use client";
import { useEffect, useRef, useState } from "react";
import { DownloadIcon, Loader2Icon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useAgentInventory } from "@/hooks/use-agent-inventory";
import {
  AgentExportError, defaultChoices, deleteExport, estimatedBytes, exportDownloadPath, exportErrorMessage,
  formatBytes, readExportStatus, startExport, startSurvey,
  type ExportChoices, type ExportJob, type ExportSummary,
} from "@/lib/agent-export";
import { formatUtcDateTime } from "@/lib/date-time";
import headingStyles from "@/styles/agent-page-heading.module.css";

// The agent does the work; the page only checks in while something runs.
const POLL_MS = 3000;

export function AgentExportPanel({ runtimeId, agentName }: { runtimeId: string; agentName: string }) {
  const status = useAgentInventory(runtimeId, readExportStatus);
  const [acting, setActing] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const pending = useRef<AbortController | null>(null);
  const survey = status.data?.survey ?? null;
  const current = status.data?.export ?? null;
  const running = survey?.state === "running" || current?.state === "running";
  const { busy, loadedAt, refresh } = status;

  // One read at a time: the next starts only after the last one finished.
  useEffect(() => {
    if (!running || busy) return;
    const timer = window.setTimeout(() => void refresh(), POLL_MS);
    return () => window.clearTimeout(timer);
  }, [running, busy, loadedAt, refresh]);
  useEffect(() => () => pending.current?.abort(), []);

  async function act(action: (signal: AbortSignal) => Promise<void>) {
    pending.current?.abort();
    const controller = new AbortController();
    pending.current = controller;
    setActing(true);
    setActionError(null);
    try {
      await action(controller.signal);
    } catch (caught) {
      if (controller.signal.aborted) return;
      setActionError(exportErrorMessage(caught instanceof AgentExportError ? caught.code : "unknown", agentName));
    } finally {
      if (!controller.signal.aborted) setActing(false);
    }
    // The agent's own record is the truth, whatever the answer was.
    await refresh();
  }

  return <section className={`grid max-w-3xl gap-8 ${headingStyles.page}`} aria-label="Export">
    <header className={headingStyles.stack}>
      <h1 className={headingStyles.title}>Export</h1>
      <p className={headingStyles.subtitle}>Download everything {agentName} has, to keep or to move somewhere else.</p>
    </header>

    {!status.data && status.busy && <p className="flex items-center gap-2 text-sm text-muted-foreground">
      <Loader2Icon aria-hidden className="size-4 animate-spin" />Checking {agentName}…</p>}
    {status.error && <div role="alert" className="space-y-2 text-sm">
      <p>{status.error}</p>
      {!status.data && <Button type="button" variant="outline" onClick={() => void refresh()}>Try again</Button>}
    </div>}
    {actionError && <p role="alert" className="text-sm text-amber-700 dark:text-amber-400">{actionError}</p>}

    {current && <ExportCard job={current} runtimeId={runtimeId} agentName={agentName} acting={acting}
      onDelete={() => void act(signal => deleteExport(runtimeId, signal))} />}

    {status.data && current?.state !== "running" && <section aria-label="What to include" className="space-y-4 rounded-xl border p-5">
      <h2 className="text-base font-semibold">{current?.state === "ready" ? "Make a new export" : "What to include"}</h2>
      {survey?.state === "running" ? <p className="flex items-center gap-2 text-sm">
        <Loader2Icon aria-hidden className="size-4 animate-spin" />Checking what {agentName} has…</p>
        : survey?.state === "ready" && survey.summary ? <Choices key={survey.finishedAt} summary={survey.summary}
          agentName={agentName} acting={acting} replaces={current?.state === "ready"}
          onCheckAgain={() => void act(signal => startSurvey(runtimeId, signal))}
          onCreate={choices => void act(signal => startExport(runtimeId, choices, signal))} />
          : <div className="space-y-3 text-sm">
            {survey && <p role="alert">{survey.state === "interrupted"
              ? `${agentName} stopped before it finished checking. Try again.`
              : `${agentName} couldn’t check what it has. Try again.`}</p>}
            <p className="text-muted-foreground">First, {agentName} checks what it has: chats, memories, skills, scheduled jobs, sites, brains and files.</p>
            <Button type="button" disabled={acting} onClick={() => void act(signal => startSurvey(runtimeId, signal))}>
              {acting && <Loader2Icon aria-hidden className="animate-spin" />}Check what to export</Button>
          </div>}
    </section>}
  </section>;
}

function ExportCard({ job, runtimeId, agentName, acting, onDelete }: {
  job: ExportJob; runtimeId: string; agentName: string; acting: boolean; onDelete: () => void;
}) {
  if (job.state === "running") {
    return <section aria-label="Your export" className="space-y-3 rounded-xl border p-5">
      <h2 className="flex items-center gap-2 text-base font-semibold">
        <Loader2Icon aria-hidden className="size-4 animate-spin" />Making your export…</h2>
      <p className="text-sm text-muted-foreground">This can take several minutes for a large agent. It keeps going if you leave this page.</p>
      <Button type="button" variant="outline" disabled={acting} onClick={onDelete}>Cancel</Button>
    </section>;
  }
  if (job.state === "ready") {
    return <section aria-label="Your export" className="space-y-3 rounded-xl border p-5">
      <h2 className="text-base font-semibold">Your export is ready</h2>
      <p className="text-sm"><span className="break-all font-mono text-xs">{job.fileName}</span> · {formatBytes(job.bytes ?? 0)}</p>
      {job.expiresAt && <p className="text-sm text-muted-foreground">Available until {formatUtcDateTime(job.expiresAt)}.</p>}
      {(job.problems ?? 0) > 0 && <p className="text-sm text-muted-foreground">
        {job.problems === 1 ? "1 item" : `${job.problems} items`} couldn’t be included. The download’s manifest.json lists what and why.</p>}
      <div className="flex flex-wrap gap-2">
        <Button asChild><a href={exportDownloadPath(runtimeId)} download={job.fileName ?? undefined}>
          <DownloadIcon aria-hidden />Download</a></Button>
        <Button type="button" variant="outline" disabled={acting} onClick={onDelete}>Delete export</Button>
      </div>
    </section>;
  }
  return <p role="alert" className="text-sm">{job.state === "interrupted"
    ? `The last export stopped before it finished, for example because ${agentName} restarted. Make a new one below.`
    : "The last export didn’t finish. Make a new one below."}</p>;
}

function Choices({ summary, agentName, acting, replaces, onCheckAgain, onCreate }: {
  summary: ExportSummary; agentName: string; acting: boolean; replaces: boolean;
  onCheckAgain: () => void; onCreate: (choices: ExportChoices) => void;
}) {
  const [choices, setChoices] = useState(() => defaultChoices(summary));
  const toggle = (kind: keyof ExportChoices, value: string) => setChoices(previous => ({
    ...previous,
    [kind]: previous[kind].includes(value) ? previous[kind].filter(item => item !== value) : [...previous[kind], value],
  }));
  const facts: Array<[string, string]> = [
    ["Chats", String(summary.conversations)],
    ["Memories", String(summary.memories)],
    ["Skills you or the agent made", String(summary.skills)],
    ["Scheduled jobs", String(summary.scheduledJobs)],
    ["Files", `${summary.files} (${formatBytes(summary.fileBytes)})`],
  ];
  return <div className="space-y-5 text-sm">
    <dl className="grid grid-cols-[max-content_1fr] gap-x-6 gap-y-1">
      {facts.map(([label, value]) => <div key={label} className="contents">
        <dt className="text-muted-foreground">{label}</dt><dd>{value}</dd>
      </div>)}
    </dl>
    <fieldset className="space-y-2">
      <legend className="mb-2 font-medium">Brains</legend>
      {summary.brains.length === 0 && <p className="text-muted-foreground">{agentName} has no brains to export.</p>}
      {summary.brains.map(brain => <label key={brain.id} className="flex items-start gap-3">
        <input type="checkbox" className="mt-0.5 size-4 accent-primary" checked={choices.brains.includes(brain.id)}
          onChange={() => toggle("brains", brain.id)} />
        <span><span className="font-medium">{brain.name}</span>
          <span className="block text-muted-foreground">{brain.kind === "personal"
            ? "Personal brain"
            : "Organization brain: includes what your teammates wrote. If your team is moving it separately, leave it out."}</span></span>
      </label>)}
    </fieldset>
    <fieldset className="space-y-2">
      <legend className="mb-2 font-medium">Sites</legend>
      {summary.sites.length === 0 && <p className="text-muted-foreground">{agentName} has no sites to export.</p>}
      {summary.sites.map(site => <label key={site.slug} className="flex items-start gap-3">
        <input type="checkbox" className="mt-0.5 size-4 accent-primary" checked={choices.sites.includes(site.slug)}
          onChange={() => toggle("sites", site.slug)} />
        <span><span className="font-medium">{site.url ? new URL(site.url).host : site.slug}</span>
          {!site.owned && <span className="block text-muted-foreground">Someone else owns this site. Include it only if it’s yours to move.</span>}</span>
      </label>)}
    </fieldset>
    <p className="text-muted-foreground">Credential files such as .env and keys are never included. Credentials are removed from memories and scheduled jobs, and other files that may hold one, archives included, are left out. Chats and media go as they are, so anything pasted into a chat comes with it. App dependencies and build output are left out too; they can be rebuilt.</p>
    {summary.problems > 0 && <p className="text-muted-foreground">{agentName} couldn’t read everything while checking. The export lists anything it misses.</p>}
    <p>About {formatBytes(estimatedBytes(summary, choices))} of files and brains.{replaces ? " This replaces your current export." : ""}</p>
    <div className="flex flex-wrap gap-2">
      <Button type="button" disabled={acting} onClick={() => onCreate(choices)}>
        {acting && <Loader2Icon aria-hidden className="animate-spin" />}{replaces ? "Create a new export" : "Create export"}</Button>
      <Button type="button" variant="outline" disabled={acting} onClick={onCheckAgain}>Check again</Button>
    </div>
  </div>;
}
