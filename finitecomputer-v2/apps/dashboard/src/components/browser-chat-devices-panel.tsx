"use client";

import { useHostedChat } from "@/components/hosted-chat-provider";

export function BrowserChatDevicesPanel() {
  const { state, transportError, streamConnected } = useHostedChat();
  const devices = [...(state?.devices ?? [])].sort((a, b) =>
    Number(b.current_device) - Number(a.current_device) || a.device_id.localeCompare(b.device_id));
  const agent = state?.hosted_agent_binding?.agent_account_id;

  return (
    <section aria-labelledby="chat-devices-heading" className="rounded-xl border border-border bg-white/[0.03] p-4">
      <div className="mb-4">
        <h2 id="chat-devices-heading" className="font-medium">
          Chat devices{state ? ` · ${devices.length}` : ""}
        </h2>
        <p className="text-sm text-muted-foreground">
          Members of your encrypted chat Room. Each browser profile has its own Device; tabs share a Device.
        </p>
        <p className="mt-1 text-sm text-muted-foreground">
          Membership updates automatically. Listed devices can be offline.
        </p>
      </div>
      {transportError ? (
        <p role="alert" className="mb-3 text-sm text-destructive">
          {transportError}{state ? " Showing the last known membership." : ""}
        </p>
      ) : null}
      {!state ? <p role="status" className="text-sm text-muted-foreground">Loading chat devices…</p> : (
        <>
          {!streamConnected && !transportError ? (
            <p role="status" className="mb-3 text-sm text-muted-foreground">Reconnecting… Showing the last known membership.</p>
          ) : null}
          <div className="overflow-x-auto">
            <table className="w-full text-left text-sm">
              <thead className="text-xs text-muted-foreground">
                <tr className="border-b border-border/70">
                  <th className="px-2 py-2 font-medium">Device</th>
                  <th className="px-2 py-2 font-medium">Device ID</th>
                </tr>
              </thead>
              <tbody>
                {devices.map(device => (
                  <tr key={`${device.account_id}:${device.device_id}`} className="border-b border-border/40 last:border-0">
                    <td className="whitespace-nowrap px-2 py-3">
                      {device.current_device ? "This browser" : device.account_id === agent ? "Hermes" : "Another browser"}
                    </td>
                    <td className="break-all px-2 py-3 font-mono text-xs">{device.device_id}</td>
                  </tr>
                ))}
                {devices.length === 0 ? (
                  <tr><td colSpan={2} className="px-2 py-4 text-muted-foreground">Reload this page to load the updated Device list.</td></tr>
                ) : null}
              </tbody>
            </table>
          </div>
        </>
      )}
    </section>
  );
}
