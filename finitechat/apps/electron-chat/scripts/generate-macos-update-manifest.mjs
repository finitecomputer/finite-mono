import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Emits the electron-updater macOS channel file (latest-mac.yml) for the
// generic provider. The feed URL baked into the packaged app points at the
// rolling `finitechat-latest` alias release, while every artifact the file
// references is pinned to the immutable versioned `finitechat/vX.Y.Z` release
// so an attacker who could rewrite the alias still cannot redirect downloads
// to a mutable artifact URL.
const electronAssetName = "finitechat-electron-macos-aarch64.zip";

export function buildMacosUpdateManifest({ version, assetUrl, assetFile, publishedAt }) {
  if (!/^\d+\.\d+\.\d+$/u.test(version)) {
    throw new Error("Finite Chat update version must be numeric MAJOR.MINOR.PATCH");
  }

  const parsedAssetUrl = new URL(assetUrl);
  if (parsedAssetUrl.protocol !== "https:" || parsedAssetUrl.hostname !== "github.com") {
    throw new Error("Finite Chat update asset must use GitHub HTTPS");
  }
  const expectedPath =
    `/finitecomputer/finite-releases/releases/download/finitechat/v${version}/${electronAssetName}`;
  if (parsedAssetUrl.pathname !== expectedPath || parsedAssetUrl.search || parsedAssetUrl.hash) {
    throw new Error("Finite Chat update asset URL does not match the versioned release");
  }

  const publishedDate = new Date(publishedAt);
  if (!Number.isFinite(publishedDate.getTime())) {
    throw new Error("Finite Chat update publication date is invalid");
  }

  const assetBytes = fs.readFileSync(path.resolve(assetFile));
  const sha512 = crypto.createHash("sha512").update(assetBytes).digest("base64");
  return {
    version,
    releaseDate: publishedDate.toISOString(),
    files: [
      {
        url: parsedAssetUrl.toString(),
        sha512,
        size: assetBytes.length,
      },
    ],
  };
}

function renderYaml(manifest) {
  const file = manifest.files[0];
  const lines = [
    `version: ${manifest.version}`,
    `releaseDate: ${JSON.stringify(manifest.releaseDate)}`,
    "files:",
    `  - url: ${file.url}`,
    `    sha512: ${file.sha512}`,
    `    size: ${file.size}`,
  ];
  return `${lines.join("\n")}\n`;
}

function parseArguments(argv) {
  const values = {};
  for (let index = 0; index < argv.length; index += 2) {
    const name = argv[index];
    const value = argv[index + 1];
    if (!name?.startsWith("--") || value === undefined) {
      throw new Error("Expected --version, --asset-url, --asset-file, --published-at, and --output");
    }
    values[name.slice(2)] = value;
  }
  for (const name of ["version", "asset-url", "asset-file", "published-at", "output"]) {
    if (!values[name]) {
      throw new Error(`Missing --${name}`);
    }
  }
  return values;
}

function main(argv) {
  const values = parseArguments(argv);
  const manifest = buildMacosUpdateManifest({
    version: values.version,
    assetUrl: values["asset-url"],
    assetFile: values["asset-file"],
    publishedAt: values["published-at"],
  });
  const outputPath = path.resolve(values.output);
  fs.mkdirSync(path.dirname(outputPath), { recursive: true });
  fs.writeFileSync(outputPath, renderYaml(manifest));
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2));
}
