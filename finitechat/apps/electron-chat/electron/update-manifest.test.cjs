const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { pathToFileURL } = require("node:url");

const manifestModuleUrl = pathToFileURL(
  path.resolve(__dirname, "../scripts/generate-macos-update-manifest.mjs")
).href;

const versionedAssetUrl = (version) =>
  `https://github.com/finitecomputer/finite-mono/releases/download/finitechat/v${version}/finitechat-electron-macos-aarch64.zip`;

async function buildManifest(input) {
  const { buildMacosUpdateManifest } = await import(manifestModuleUrl);
  return buildMacosUpdateManifest(input);
}

function writeAssetFixture() {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "finitechat-feed-"));
  const assetFile = path.join(directory, "finitechat-electron-macos-aarch64.zip");
  const bytes = Buffer.from("pretend ditto zip payload");
  fs.writeFileSync(assetFile, bytes);
  return {
    assetFile,
    bytes,
    cleanup() {
      fs.rmSync(directory, { recursive: true, force: true });
    },
  };
}

test("macOS update feed is an electron-updater channel file pinned to the versioned release", async () => {
  const fixture = writeAssetFixture();
  try {
    const manifest = await buildManifest({
      version: "0.1.9",
      assetUrl: versionedAssetUrl("0.1.9"),
      assetFile: fixture.assetFile,
      publishedAt: "2026-07-25T18:00:00Z",
    });

    assert.deepEqual(manifest, {
      version: "0.1.9",
      releaseDate: "2026-07-25T18:00:00.000Z",
      files: [
        {
          url: versionedAssetUrl("0.1.9"),
          sha512: crypto.createHash("sha512").update(fixture.bytes).digest("base64"),
          size: fixture.bytes.length,
        },
      ],
    });
  } finally {
    fixture.cleanup();
  }
});

test("macOS update feed rejects mutable or cross-repository assets", async () => {
  const fixture = writeAssetFixture();
  try {
    const base = {
      version: "0.1.9",
      assetFile: fixture.assetFile,
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
  } finally {
    fixture.cleanup();
  }
});

test("macOS update feed requires stable numeric versions and valid dates", async () => {
  const fixture = writeAssetFixture();
  try {
    await assert.rejects(
      buildManifest({
        version: "0.1.9-beta.1",
        assetUrl: versionedAssetUrl("0.1.9"),
        assetFile: fixture.assetFile,
        publishedAt: "2026-07-25T18:00:00Z",
      })
    );
    await assert.rejects(
      buildManifest({
        version: "0.1.9",
        assetUrl: versionedAssetUrl("0.1.9"),
        assetFile: fixture.assetFile,
        publishedAt: "not-a-date",
      })
    );
  } finally {
    fixture.cleanup();
  }
});
