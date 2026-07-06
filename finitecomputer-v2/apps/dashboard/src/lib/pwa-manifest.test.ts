import assert from "node:assert/strict";
import test from "node:test";

import { buildPwaManifest } from "@/lib/pwa-manifest";

test("PWA manifest starts on the requested machine overview", () => {
  const manifest = buildPwaManifest("paul-finite-2");

  assert.equal(manifest.id, "/dashboard/machines/paul-finite-2");
  assert.equal(manifest.start_url, "/dashboard/machines/paul-finite-2");
  assert.equal(manifest.scope, "/dashboard");
  assert.deepEqual(manifest.icons[0], {
    src: "/favicon.svg",
    sizes: "any",
    type: "image/svg+xml",
    purpose: "any",
  });
});

test("PWA manifest ignores invalid machine ids", () => {
  const manifest = buildPwaManifest("../box1");

  assert.equal(manifest.id, "/dashboard");
  assert.equal(manifest.start_url, "/dashboard");
});
