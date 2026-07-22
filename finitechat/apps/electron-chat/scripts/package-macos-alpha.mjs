import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const appRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = path.resolve(appRoot, "../../..");
const electronApp = path.join(
  appRoot,
  "node_modules",
  "electron",
  "dist",
  "Electron.app"
);
const daemonBinary = path.resolve(
  process.env.FINITECHAT_DAEMON_BINARY || path.join(repoRoot, "target", "release", "finitechatd")
);
const outputRoot = path.join(appRoot, "release");
const outputApp = path.join(outputRoot, "Finite Chat.app");
const contents = path.join(outputApp, "Contents");
const resources = path.join(contents, "Resources");
const packagedApp = path.join(resources, "app");
const packageJson = JSON.parse(fs.readFileSync(path.join(appRoot, "package.json"), "utf8"));

requireDirectory(electronApp, "Electron runtime");
requireDirectory(path.join(appRoot, "dist"), "built renderer");
requireExecutable(daemonBinary, "finitechatd release binary");

fs.mkdirSync(outputRoot, { recursive: true });
fs.rmSync(outputApp, { recursive: true, force: true });
fs.cpSync(electronApp, outputApp, {
  recursive: true,
  dereference: false,
  // Electron frameworks use relative symlinks. Node otherwise rewrites them
  // to absolute paths into node_modules, producing a non-portable bundle that
  // macOS correctly refuses to seal.
  verbatimSymlinks: true,
});
fs.rmSync(path.join(contents, "_CodeSignature"), { recursive: true, force: true });

const oldExecutable = path.join(contents, "MacOS", "Electron");
const executable = path.join(contents, "MacOS", "Finite Chat");
fs.renameSync(oldExecutable, executable);

fs.mkdirSync(packagedApp, { recursive: true });
fs.cpSync(path.join(appRoot, "dist"), path.join(packagedApp, "dist"), {
  recursive: true,
});
fs.cpSync(path.join(appRoot, "electron"), path.join(packagedApp, "electron"), {
  recursive: true,
  filter(source) {
    return !source.endsWith(".test.cjs");
  },
});
fs.writeFileSync(
  path.join(packagedApp, "package.json"),
  `${JSON.stringify(
    {
      name: packageJson.name,
      version: packageJson.version,
      private: true,
      main: "electron/main.cjs",
    },
    null,
    2
  )}\n`
);

const packagedDaemon = path.join(resources, "finitechatd");
fs.copyFileSync(daemonBinary, packagedDaemon);
fs.chmodSync(packagedDaemon, 0o755);

const iconPath = buildIcon(outputRoot, resources);
const infoPath = path.join(contents, "Info.plist");
let info = fs.readFileSync(infoPath, "utf8");
info = replacePlistString(info, "CFBundleDisplayName", "Finite Chat");
info = replacePlistString(info, "CFBundleExecutable", "Finite Chat");
info = replacePlistString(info, "CFBundleIconFile", path.basename(iconPath));
info = replacePlistString(info, "CFBundleIdentifier", "computer.finite.chat");
info = replacePlistString(info, "CFBundleName", "Finite Chat");
info = replacePlistString(info, "CFBundleShortVersionString", packageJson.version);
info = replacePlistString(info, "CFBundleVersion", packageJson.version);
info = replacePlistString(
  info,
  "LSApplicationCategoryType",
  "public.app-category.social-networking"
);
fs.writeFileSync(infoPath, info);

signAlphaBundle(outputApp, packagedDaemon);

console.log(outputApp);

function requireDirectory(directory, label) {
  if (!fs.statSync(directory, { throwIfNoEntry: false })?.isDirectory()) {
    throw new Error(`${label} is missing: ${directory}`);
  }
}

function requireExecutable(filePath, label) {
  const metadata = fs.statSync(filePath, { throwIfNoEntry: false });
  if (!metadata?.isFile()) {
    throw new Error(`${label} is missing: ${filePath}`);
  }
  if ((metadata.mode & 0o111) === 0) {
    throw new Error(`${label} is not executable: ${filePath}`);
  }
}

function replacePlistString(info, key, value) {
  const pattern = new RegExp(`(<key>${key}</key>\\s*<string>)[^<]*(</string>)`, "u");
  if (!pattern.test(info)) {
    throw new Error(`Electron Info.plist is missing ${key}`);
  }
  return info.replace(pattern, `$1${escapeXml(value)}$2`);
}

function escapeXml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&apos;");
}

function signAlphaBundle(appPath, daemonPath) {
  const frameworks = path.join(appPath, "Contents", "Frameworks");
  const electronFramework = path.join(
    frameworks,
    "Electron Framework.framework"
  );
  const electronVersion = path.join(electronFramework, "Versions", "A");
  const nestedCode = [
    path.join(electronVersion, "Helpers", "chrome_crashpad_handler"),
    ...fs
      .readdirSync(path.join(electronVersion, "Libraries"))
      .filter((name) => name.endsWith(".dylib"))
      .sort()
      .map((name) => path.join(electronVersion, "Libraries", name)),
    electronFramework,
    path.join(frameworks, "Mantle.framework"),
    path.join(frameworks, "ReactiveObjC.framework"),
    path.join(frameworks, "Squirrel.framework"),
    ...fs
      .readdirSync(frameworks)
      .filter((name) => name.startsWith("Electron Helper") && name.endsWith(".app"))
      .sort()
      .map((name) => path.join(frameworks, name)),
    daemonPath,
    appPath,
  ];

  // Apple's manual-signing contract is inside-out: every nested code item must
  // already have a valid signature when its containing bundle is sealed. The
  // alpha uses an ad-hoc identity; release distribution still requires the
  // normal Developer ID and notarization workflow.
  for (const codePath of nestedCode) {
    execFileSync("/usr/bin/codesign", [
      "--force",
      "--sign",
      "-",
      "--timestamp=none",
      codePath,
    ]);
  }
  execFileSync("/usr/bin/codesign", [
    "--verify",
    "--strict",
    "--verbose=2",
    daemonPath,
  ]);
  execFileSync("/usr/bin/codesign", [
    "--verify",
    "--deep",
    "--strict",
    "--verbose=2",
    appPath,
  ]);
}

function buildIcon(outputDirectory, resourceDirectory) {
  if (process.platform !== "darwin") {
    throw new Error("The internal alpha packager currently targets macOS");
  }
  const source = path.join(
    repoRoot,
    "finitechat",
    "ios",
    "Sources",
    "Assets.xcassets",
    "AppIcon.appiconset",
    "AppIcon-1024.png"
  );
  const iconset = path.join(outputDirectory, ".finitechat.iconset");
  const output = path.join(resourceDirectory, "finitechat.icns");
  fs.rmSync(iconset, { recursive: true, force: true });
  fs.mkdirSync(iconset, { recursive: true });
  const representations = [
    ["icp4", "icon_16x16.png", 16],
    ["icp5", "icon_32x32.png", 32],
    ["icp6", "icon_64x64.png", 64],
    ["ic07", "icon_128x128.png", 128],
    ["ic08", "icon_256x256.png", 256],
    ["ic09", "icon_512x512.png", 512],
    ["ic10", "icon_1024x1024.png", 1024],
  ];
  for (const [, name, pixels] of representations) {
    execFileSync("/usr/bin/sips", ["-z", String(pixels), String(pixels), source, "--out", path.join(iconset, name)], {
      stdio: "ignore",
    });
  }
  const elements = representations.map(([type, name]) => {
    const png = fs.readFileSync(path.join(iconset, name));
    const header = Buffer.alloc(8);
    header.write(type, 0, 4, "ascii");
    header.writeUInt32BE(header.length + png.length, 4);
    return Buffer.concat([header, png]);
  });
  const header = Buffer.alloc(8);
  header.write("icns", 0, 4, "ascii");
  header.writeUInt32BE(header.length + elements.reduce((size, element) => size + element.length, 0), 4);
  fs.writeFileSync(output, Buffer.concat([header, ...elements]));
  fs.rmSync(iconset, { recursive: true, force: true });
  return output;
}
