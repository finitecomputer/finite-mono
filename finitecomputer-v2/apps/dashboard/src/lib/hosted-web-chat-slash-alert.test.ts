import assert from "node:assert/strict";
import test from "node:test";
import { createElement, type ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { HostedChatContext } from "@/components/hosted-chat-provider";
import { HostedWebChat } from "@/components/hosted-web-chat";

type HostedChatValue = NonNullable<ComponentProps<typeof HostedChatContext.Provider>["value"]>;

const unused = () => {
  throw new Error("not used while rendering");
};

function renderChat(draft: string, alert: Partial<HostedChatValue>) {
  const value: HostedChatValue = {
    apiBase: "",
    state: null,
    transportError: null,
    claimError: null,
    sessionError: null,
    streamConnected: false,
    ownerClaimed: true,
    bindingRecoveryRequired: false,
    selectionPending: false,
    load: unused,
    claimOwner: unused,
    recoverBinding: unused,
    reportSessionAuthFailure: () => false,
    signInAgain: unused,
    noteComposerInput: () => undefined,
    dispatch: unused,
    dispatchQuiet: unused,
    refreshPendingChat: unused,
    uploadAttachments: unused,
    attachmentUrl: () => "",
    ...alert,
  };
  return renderToStaticMarkup(createElement(
    HostedChatContext.Provider,
    { value },
    createElement(HostedWebChat, {
      initialDraft: draft,
      machineId: "machine",
      machineLabel: "Agent",
      runtimeStatus: "online",
    })
  ));
}

// The picker is positioned upward from the element that contains it, so the
// alert stays uncovered only while that element comes before the alert.
test("slash picker is anchored above the chat alert", () => {
  for (const draft of ["/", "/help"]) {
    for (const alert of [
      { transportError: "Chat could not refresh. Reconnecting…" },
      { claimError: "Could not claim this chat." },
      { sessionError: "Your session ended." },
    ]) {
      const html = renderChat(draft, alert);
      const picker = html.indexOf('class="finite-chat__slash"');
      const chatAlert = html.indexOf('role="alert"');
      assert.notEqual(picker, -1, `${draft} opens the picker`);
      assert.notEqual(chatAlert, -1, "the alert is shown");
      assert.ok(picker < chatAlert, `${draft} picker must come before the alert`);
    }
  }
});
