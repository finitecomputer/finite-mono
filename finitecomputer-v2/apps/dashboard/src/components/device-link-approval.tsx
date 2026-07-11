"use client";

import Link from "next/link";
import { CheckCircle2Icon, LaptopIcon, Loader2Icon, ShieldCheckIcon } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import type {
  HostedDeviceLinkRequest,
  HostedDeviceLinkResponse,
} from "@/lib/hosted-web-device";

type DeviceLinkApprovalProps = {
  input: HostedDeviceLinkRequest;
  viewerEmail: string;
};

export function DeviceLinkApproval({ input, viewerEmail }: DeviceLinkApprovalProps) {
  const [result, setResult] = useState<HostedDeviceLinkResponse | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const approved = result !== null;
  const terminal = result?.status === "ready" || result?.status === "expired";

  const request = useCallback(
    async (path: "approve" | "status", signal?: AbortSignal) => {
      const response = await fetch(`/api/device-links/${path}`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(input),
        cache: "no-store",
        signal,
      });
      const value = (await response.json()) as unknown;
      if (!response.ok) {
        const message =
          value &&
          typeof value === "object" &&
          typeof (value as { error?: unknown }).error === "string"
            ? (value as { error: string }).error
            : "Device linking is unavailable right now.";
        throw new Error(message);
      }
      return publicDeviceLinkResponse(value, input);
    },
    [input]
  );

  const approve = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      setResult(await request("approve"));
    } catch (requestError) {
      setError(
        requestError instanceof Error
          ? requestError.message
          : "Device linking is unavailable right now."
      );
    } finally {
      setBusy(false);
    }
  }, [request]);

  useEffect(() => {
    if (!approved || terminal) return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      try {
        const next = await request("status", controller.signal);
        setResult(next);
        setError(null);
        if (next.status !== "ready" && next.status !== "expired") {
          timer = setTimeout(poll, 1_000);
        }
      } catch (requestError) {
        if (controller.signal.aborted) return;
        setError(
          requestError instanceof Error
            ? requestError.message
            : "Could not check this Device yet."
        );
        timer = setTimeout(poll, 2_000);
      }
    };
    timer = setTimeout(poll, 500);
    return () => {
      controller.abort();
      if (timer) clearTimeout(timer);
    };
  }, [approved, request, terminal]);

  return (
    <section className="ocean-utility-card max-w-2xl">
      <div className="ocean-utility-card__header">
        <span className="ocean-utility-card__icon" aria-hidden>
          {result?.status === "ready" ? (
            <CheckCircle2Icon className="size-5" />
          ) : (
            <LaptopIcon className="size-5" />
          )}
        </span>
        <div>
          <h2 className="ocean-utility-card__title">
            {result?.status === "ready" ? "Electron is linked" : "Link this Electron"}
          </h2>
          <p className="text-sm text-muted-foreground">
            {statusCopy(result)}
          </p>
        </div>
      </div>

      <dl className="mt-6 grid gap-4 rounded-2xl border border-border/70 bg-background/55 p-5 text-sm sm:grid-cols-2">
        <div>
          <dt className="text-muted-foreground">Signed-in account</dt>
          <dd className="mt-1 font-medium">{viewerEmail}</dd>
        </div>
        <div>
          <dt className="text-muted-foreground">New Device</dt>
          <dd className="mt-1 break-all font-mono text-xs font-medium">
            {input.target_device_id}
          </dd>
        </div>
      </dl>

      {!approved ? (
        <div className="mt-6 space-y-4">
          <p className="flex gap-2 text-sm text-muted-foreground">
            <ShieldCheckIcon className="mt-0.5 size-4 shrink-0" aria-hidden />
            Approval adds this one local Device to your existing encrypted chats for messages from
            this point forward. It does not give the browser or WorkOS your chat key.
          </p>
          <Button type="button" onClick={() => void approve()} disabled={busy}>
            {busy ? <Loader2Icon className="size-4 animate-spin" /> : null}
            Approve Electron Device
          </Button>
        </div>
      ) : result.status === "ready" ? (
        <div className="mt-6 flex flex-wrap items-center gap-3">
          <Button asChild>
            <Link href="/dashboard">Return to dashboard</Link>
          </Button>
          <span className="text-sm text-muted-foreground">
            You can return to Electron and continue the same conversation.
          </span>
        </div>
      ) : result.status === "expired" ? (
        <p className="mt-6 text-sm text-destructive">
          This approval expired. Return to Electron and start a fresh link request.
        </p>
      ) : (
        <div className="mt-6 flex items-center gap-3 text-sm text-muted-foreground" role="status">
          <Loader2Icon className="size-4 animate-spin" aria-hidden />
          Keep this page open while Electron finishes linking.
        </div>
      )}

      {error ? (
        <div className="mt-5 rounded-xl border border-destructive/30 bg-destructive/5 p-4 text-sm">
          <p className="text-destructive">{error}</p>
          {!terminal ? (
            <Button className="mt-3" type="button" variant="outline" onClick={() => void approve()}>
              Retry
            </Button>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}

function statusCopy(result: HostedDeviceLinkResponse | null) {
  switch (result?.status) {
    case "awaiting_claim":
      return "Approved. Waiting for Electron to receive the encrypted account handoff.";
    case "awaiting_key_package":
      return "Electron received the handoff and is preparing its own Device keys.";
    case "joining_rooms":
      return `Adding Electron to your encrypted rooms (${result.active_room_count}/${result.room_count}).`;
    case "ready":
      return "This local Device now participates in the same rooms as your web Device.";
    case "expired":
      return "The short-lived approval window has ended.";
    default:
      return "Review the exact local Device before adding it to your Finite Chat account.";
  }
}

function publicDeviceLinkResponse(
  value: unknown,
  expected: HostedDeviceLinkRequest
): HostedDeviceLinkResponse {
  if (!value || typeof value !== "object") {
    throw new Error("Device linking returned an invalid response.");
  }
  const record = value as Record<string, unknown>;
  const statuses = [
    "awaiting_claim",
    "awaiting_key_package",
    "joining_rooms",
    "ready",
    "expired",
  ] as const;
  const expiresAt = record.expires_at_unix_seconds;
  const roomCount = record.room_count;
  const activeRoomCount = record.active_room_count;
  if (
    record.link_session_id !== expected.link_session_id ||
    record.target_device_id !== expected.target_device_id ||
    !statuses.includes(record.status as (typeof statuses)[number]) ||
    !Number.isSafeInteger(expiresAt) ||
    (expiresAt as number) < 0 ||
    !Number.isSafeInteger(roomCount) ||
    (roomCount as number) < 0 ||
    !Number.isSafeInteger(activeRoomCount) ||
    (activeRoomCount as number) < 0 ||
    (activeRoomCount as number) > (roomCount as number)
  ) {
    throw new Error("Device linking returned an invalid response.");
  }
  return {
    link_session_id: expected.link_session_id,
    target_device_id: expected.target_device_id,
    status: record.status as HostedDeviceLinkResponse["status"],
    expires_at_unix_seconds: expiresAt as number,
    room_count: roomCount as number,
    active_room_count: activeRoomCount as number,
  };
}
