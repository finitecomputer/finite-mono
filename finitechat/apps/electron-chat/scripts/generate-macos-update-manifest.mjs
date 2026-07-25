import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const electronAssetName = "finitechat-electron-macos-aarch64.zip";

export function buildMacosUpdateManifest({ version, assetUrl, publishedAt }) {
  if (!/^\d+\.\d+\.\d+$/u.test(version)) {
    throw new Error("Finite Chat update version must be numeric MAJOR.MINOR.PATCH");
  }

  const parsedAssetUrl = new URL(assetUrl);
  if (parsedAssetUrl.protocol !== "https:" || parsedAssetUrl.hostname !== "github.com") {
    throw new Error("Finite Chat update asset must use GitHub HTTPS");
  }
  const expectedPath =
    `/finitecomputer/finite-mono/releases/download/finitechat/v${version}/${electronAssetName}`;
  if (parsedAssetUrl.pathname !== expectedPath || parsedAssetUrl.search || parsedAssetUrl.hash) {
    throw new Error("Finite Chat update asset URL does not match the versioned release");
  }

  const publishedDate = new Date(publishedAt);
  if (!Number.isFinite(publishedDate.getTime())) {
    throw new Error("Finite Chat update publication date is invalid");
  }

  return {
    currentRelease: version,
    releases: [
      {
        version,
        updateTo: {
          version,
          url: parsedAssetUrl.toString(),
          name: `Finite Chat ${version}`,
          notes: `https://github.com/finitecomputer/finite-mono/releases/tag/finitechat/v${version}`,
          pub_date: publishedDate.toISOString(),
        },
      },
    ],
  };
}

function parseArguments(argv) {
  const values = {};
  for (let index = 0; index < argv.length; index += 2) {
    const name = argv[index];
    const value = argv[index + 1];
    if (!name?.startsWith("--") || value === undefined) {
      throw new Error("Expected --version, --asset-url, --published-at, and --output");
    }
    values[name.slice(2)] = value;
  }
  for (const name of ["version", "asset-url", "published-at", "output"]) {
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
    publishedAt: values["published-at"],
  });
  const outputPath = path.resolve(values.output);
  fs.mkdirSync(path.dirname(outputPath), { recursive: true });
  fs.writeFileSync(outputPath, `${JSON.stringify(manifest, null, 2)}\n`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2));
}
