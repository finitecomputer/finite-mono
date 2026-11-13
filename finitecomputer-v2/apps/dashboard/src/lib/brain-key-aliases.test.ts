import assert from "node:assert/strict";
import { test } from "node:test";
import { KeyAliasError, normalizeAlias, parseKeyAliases } from "./brain-key-aliases";

const npub = "npub1ccz8l9zpa47k6vz9gphftsrumpw80rjt3nhnefat4symjhrsnmjs38mnyd";

test("aliases mirror Core's trim, collapse and bounds", () => {
  assert.deepEqual(normalizeAlias("  Maya   (design) "), { alias: "Maya (design)" });
  assert.deepEqual(normalizeAlias("👩🏽\u200d💻".repeat(9)), { alias: "👩🏽\u200d💻".repeat(9) });
  for (const value of ["", "   ", "x".repeat(65), "tab\there", "\u202ereversed", "bell\u0007"]) {
    assert("error" in normalizeAlias(value), JSON.stringify(value));
  }
  assert("alias" in normalizeAlias("x".repeat(64)));
});

test("alias responses keep only npub and alias and refuse anything malformed", () => {
  assert.deepEqual(
    parseKeyAliases({ aliases: [{ npub, alias: "Maya", publicKeyHex: "ab", updatedAt: "t" }] }),
    [{ npub, alias: "Maya" }],
  );
  for (const value of [null, {}, { aliases: [{ npub: "npub1short", alias: "x" }] }, { aliases: [{ npub, alias: "" }] },
    { aliases: [{ npub, alias: "x".repeat(65) }] }]) {
    assert.throws(() => parseKeyAliases(value), KeyAliasError);
  }
});
