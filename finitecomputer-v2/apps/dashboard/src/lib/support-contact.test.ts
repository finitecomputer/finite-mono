import assert from "node:assert/strict";
import test from "node:test";
import { supportCommand, supportContact, supportEmailHref } from "./support-contact";

test("support routing is deployment-owned and rejects header/URL injection", () => {
  assert.equal(supportContact("support@finite.vip"), "support@finite.vip");
  assert.equal(supportContact("it+help@example.org"), "it+help@example.org");
  for (const value of [undefined, "", "it@example.org\r\nBcc: evil@example.org", "it@example.org?bcc=evil@example.org", "a@b@c.org", "a@example.org,evil@example.org", "mailto:it@example.org"]) {
    assert.equal(supportContact(value), null);
  }
  assert.equal(supportEmailHref("it+help@example.org"), "mailto:it%2Bhelp%40example.org");
});

test("only an explicit slash command opens support; normal chat stays untouched", () => {
  assert.equal(supportCommand("/support"), "");
  assert.equal(supportCommand(" /support My agent stopped\nPlease help "), "My agent stopped\nPlease help");
  for (const text of ["hello", "Please explain /support", "/supporting", "/support@other", "@support"]) assert.equal(supportCommand(text), null);
});
