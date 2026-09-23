"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";

type Question = { qid?: string; question: string; choices?: string[]; multi_select?: boolean };
export type HermesClarification = Question & {
  request_id: string;
  questions?: Question[];
  answers?: Record<string, string>;
};

// Hermes accepts JSON arrays or comma-separated multi-select answers. Emit JSON
// so a choice containing a comma remains one choice, including after reconnect.
function selections(answer: string): string[] {
  try {
    const parsed: unknown = JSON.parse(answer);
    if (Array.isArray(parsed)) return parsed.filter((value): value is string => typeof value === "string" && !!value.trim());
  } catch { /* Native also accepts plain comma-separated answers. */ }
  return answer.split(",").map(value => value.trim()).filter(Boolean);
}

export function HermesClarificationCard({ request, connected, respond }: {
  request: HermesClarification;
  connected: boolean;
  respond: (answer: string, questionId?: string) => Promise<void>;
}) {
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return <section aria-label="Agent question" className="mx-4 mb-3 rounded-lg border p-3 text-sm">
    {error && <p role="alert" className="mb-2 text-destructive">{error}</p>}
    {(request.questions ?? [request]).map(question => {
      const key = question.qid ?? "single";
      const answer = drafts[key] ?? request.answers?.[key] ?? "";
      const multi = !!question.multi_select && !!question.choices?.length;
      const selected = multi ? selections(answer) : [];
      const custom = multi ? selected.filter(value => !question.choices?.includes(value)).join(", ") : answer;
      const valid = multi ? selected.length > 0 : !!answer.trim();
      const change = (value: string) => setDrafts(current => ({ ...current, [key]: value }));
      return <form key={key} className="mb-3 last:mb-0" onSubmit={async event => {
        event.preventDefault();
        if (!valid || busy || !connected) return;
        setBusy(true);
        setError(null);
        try { await respond(multi ? JSON.stringify(selected) : answer, question.qid); }
        catch (error) { setError(error instanceof Error ? error.message : "Could not send answer."); }
        finally { setBusy(false); }
      }}>
        <label className="block font-medium">{question.question}
          <textarea aria-label={question.question} className="mt-2 block w-full rounded border p-2 font-normal"
            value={custom} onChange={event => change(multi
              ? JSON.stringify([...selected.filter(value => question.choices?.includes(value)), ...(event.target.value.trim() ? [event.target.value] : [])])
              : event.target.value)} />
        </label>
        {!!question.choices?.length && <fieldset className="my-2" disabled={!connected || busy}>
          <legend>{multi ? "Choose one or more, or write your own answer" : "Choose an answer or write your own"}</legend>
          {question.choices.map(choice => <label key={choice} className="mr-4 inline-flex items-center gap-2">
            <input type={multi ? "checkbox" : "radio"} name={key} checked={multi ? selected.includes(choice) : answer === choice}
              onChange={() => change(multi ? JSON.stringify(selected.includes(choice) ? selected.filter(value => value !== choice) : [...selected, choice]) : choice)} />
            {choice}
          </label>)}
        </fieldset>}
        <Button type="submit" disabled={!connected || busy || !valid}>
          {request.answers?.[key] !== undefined ? "Update answer" : "Send answer"}
        </Button>
      </form>;
    })}
  </section>;
}
