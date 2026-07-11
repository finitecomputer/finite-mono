import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const dashboardHostUrl = new URL("../components/hosted-web-chat.tsx", import.meta.url);
const electronHostUrl = new URL(
  "../../../../../finitechat/apps/electron-chat/src/App.tsx",
  import.meta.url
);
const sharedProductUrl = new URL(
  "../../../../../finitechat/packages/finitechat-chat-ui/src/react/chat-product.tsx",
  import.meta.url
);

const sharedChatProductImport =
  /import\s*\{\s*ChatProduct\s*\}\s*from\s*["']@finite\/chat-ui\/react["']/u;
const localProductImplementation =
  /\b(?:function|const)\s+(?:MessageRow|ToolRollup|Composer|ChatComposer|MessageComposer)\b|<textarea\b|finite-chat__composer/u;

test("web and Electron are thin hosts for the same shared ChatProduct", async () => {
  const hosts = [
    ["dashboard", await readFile(dashboardHostUrl, "utf8")],
    ["Electron", await readFile(electronHostUrl, "utf8")],
  ] as const;

  for (const [name, source] of hosts) {
    assert.match(
      source,
      sharedChatProductImport,
      `${name} must import the product surface from @finite/chat-ui/react`
    );
    assert.equal(
      source.match(/<ChatProduct(?:\s|>)/gu)?.length,
      1,
      `${name} must render one shared ChatProduct surface`
    );
    assert.doesNotMatch(
      source,
      localProductImplementation,
      `${name} must not grow a second transcript, tool rollup, or composer implementation`
    );
  }
});

test("the shared product, rather than either host, owns transcript and composer UI", async () => {
  const source = await readFile(sharedProductUrl, "utf8");

  assert.match(source, /export function ChatProduct\b/u);
  assert.match(source, /function MessageRow\b/u);
  assert.match(source, /function ToolRollup\b/u);
  assert.match(source, /<textarea\b/u);
  assert.match(source, /finite-chat__composer/u);
});

test("switching dashboard machines remounts product-local drafts and attachments", async () => {
  const source = await readFile(dashboardHostUrl, "utf8");
  assert.match(source, /<ChatProduct\s+key=\{machineId\}/u);
});
