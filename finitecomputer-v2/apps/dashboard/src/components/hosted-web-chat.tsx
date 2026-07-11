"use client";

import { useEffect, useMemo } from "react";

import { ChatProduct } from "@finite/chat-ui/react";

import {
  createHostedSitePreview,
  createHostedWebChatTransport,
} from "@/lib/hosted-web-chat-transport";

/**
 * Hosted Web is an authenticated transport for the shared Finite Chat
 * product. Product state, behavior, DOM, and styling live in @finite/chat-ui
 * and are exactly the same surface rendered by Electron.
 */
export function HostedWebChat({
  connectionsHref,
  initialDraft,
  machineId,
  machineLabel,
  showSkills,
  viewerEmail,
}: {
  connectionsHref?: string | null;
  initialDraft?: string;
  machineId: string;
  machineLabel: string;
  showSkills: boolean;
  viewerEmail?: string | null;
}) {
  const transport = useMemo(
    () => createHostedWebChatTransport(machineId),
    [machineId]
  );
  const preview = useMemo(() => createHostedSitePreview(machineId), [machineId]);

  useEffect(() => () => transport.close(), [transport]);

  return (
    <ChatProduct
      key={machineId}
      transport={transport}
      machineLabel={machineLabel}
      viewerLabel={viewerEmail}
      initialDraft={initialDraft}
      preview={preview}
      navigation={{
        agent: {
          href: `/dashboard/machines/${encodeURIComponent(machineId)}`,
          label: "Agent",
        },
        connections: connectionsHref
          ? { href: connectionsHref, label: "Connections" }
          : null,
        skills: showSkills
          ? {
              href: `/dashboard/skills?machine=${encodeURIComponent(machineId)}`,
              label: "Skills",
            }
          : null,
        signOut: { href: "/logout", label: "Sign out" },
      }}
    />
  );
}
