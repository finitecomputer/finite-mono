const assert = require("node:assert/strict");
const path = require("node:path");
const test = require("node:test");
const { pathToFileURL } = require("node:url");

const manifestModuleUrl = pathToFileURL(
  path.resolve(__dirname, "../scripts/generate-macos-update-manifest.mjs")
).href;

async function buildManifest(input) {
  const { buildMacosUpdateManifest } = await import(manifestModuleUrl);
  return buildMacosUpdateManifest(input);
}

test("macOS update manifest points at the immutable component release", async () => {
  const manifest = await buildManifest({
    version: "0.1.9",
    assetUrl:
      "https://github.com/finitecomputer/finite-mono/releases/download/finitechat/v0.1.9/finitechat-electron-macos-aarch64.zip",
    publishedAt: "2026-07-25T18:00:00Z",
  });

  assert.deepEqual(manifest, {
    currentRelease: "0.1.9",
    releases: [
      {
        version: "0.1.9",
        updateTo: {
          version: "0.1.9",
          url: "https://github.com/finitecomputer/finite-mono/releases/download/finitechat/v0.1.9/finitechat-electron-macos-aarch64.zip",
          name: "Finite Chat 0.1.9",
          notes:
            "https://github.com/finitecomputer/finite-mono/releases/tag/finitechat/v0.1.9",
          pub_date: "2026-07-25T18:00:00.000Z",
        },
      },
    ],
  });
});

test("macOS update manifest rejects mutable or cross-repository assets", async () => {
  const base = {
    version: "0.1.9",
    publishedAt: "2026-07-25T18:00:00Z",
  };
  for (const assetUrl of [
    "http://github.com/finitecomputer/finite-mono/releases/download/finitechat/v0.1.9/finitechat-electron-macos-aarch64.zip",
    "https://evil.example/finitechat-electron-macos-aarch64.zip",
    "https://github.com/finitecomputer/finite-mono/releases/download/finitechat-latest/finitechat-electron-macos-aarch64.zip",
    "https://github.com/finitecomputer/finite-mono/releases/download/finitechat/v0.1.8/finitechat-electron-macos-aarch64.zip",
    "https://github.com/finitecomputer/finite-mono/releases/download/finitechat/v0.1.9/other.zip",
  ]) {
    await assert.rejects(buildManifest({ ...base, assetUrl }));
  }
});

test("macOS update manifest requires stable numeric versions and valid dates", async () => {
  const assetUrl =
    "https://github.com/finitecomputer/finite-mono/releases/download/finitechat/v0.1.9/finitechat-electron-macos-aarch64.zip";
  await assert.rejects(
    buildManifest({ version: "0.1.9-beta.1", assetUrl, publishedAt: "2026-07-25T18:00:00Z" })
  );
  await assert.rejects(
    buildManifest({ version: "0.1.9", assetUrl, publishedAt: "not-a-date" })
  );
});
