import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const appRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = path.resolve(appRoot, "../../..");
const require = createRequire(import.meta.url);
const electronExecutable = require("electron");
const electronVersion = JSON.parse(
  fs.readFileSync(require.resolve("electron/package.json"), "utf8")
).version;
const fixtureRoot = path.join(repoRoot, ".local-state", "electron-web-design");
const profilePath = path.join(fixtureRoot, "profile");
const runtimePath = prepareFixtureRuntime();

fs.mkdirSync(profilePath, { recursive: true, mode: 0o700 });

const child = spawn(runtimePath, [appRoot], {
  cwd: appRoot,
  env: {
    ...process.env,
    FINITECHAT_DASHBOARD_PATH: "/dashboard/machines/runtime_web_design/chat",
    FINITECHAT_DASHBOARD_URL: "http://127.0.0.1:13002",
    FINITECHAT_DISABLE_LOCAL_CHAT_BRIDGE: "1",
    FINITECHAT_DISABLE_SINGLE_INSTANCE_LOCK: "1",
    FINITECHAT_USER_DATA_DIR: profilePath,
  },
  stdio: "inherit",
});

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => child.kill(signal));
}
child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exitCode = code ?? 1;
});

function prepareFixtureRuntime() {
  if (process.platform !== "darwin") {
    return electronExecutable;
  }

  const sourceApp = path.resolve(path.dirname(electronExecutable), "../..");
  const runtimeDirectory = path.join(fixtureRoot, "runtime");
  const outputApp = path.join(
    runtimeDirectory,
    `Finite Chat Web Design Fixture-${electronVersion}-v1.app`
  );
  const outputExecutable = path.join(outputApp, "Contents", "MacOS", "Electron");
  if (fs.statSync(outputExecutable, { throwIfNoEntry: false })?.isFile()) {
    return outputExecutable;
  }

  fs.mkdirSync(runtimeDirectory, { recursive: true });
  const temporaryApp = `${outputApp}.preparing`;
  fs.rmSync(temporaryApp, { recursive: true, force: true });
  fs.cpSync(sourceApp, temporaryApp, {
    recursive: true,
    dereference: false,
    verbatimSymlinks: true,
  });

  const contents = path.join(temporaryApp, "Contents");
  fs.rmSync(path.join(contents, "_CodeSignature"), { recursive: true, force: true });
  const infoPath = path.join(contents, "Info.plist");
  let info = fs.readFileSync(infoPath, "utf8");
  info = replacePlistString(info, "CFBundleDisplayName", "Finite Chat Web Design Fixture");
  info = replacePlistString(info, "CFBundleIdentifier", "computer.finite.chat.web-design-fixture");
  info = replacePlistString(info, "CFBundleName", "Finite Chat Web Design Fixture");
  info = replacePlistString(
    info,
    "NSMicrophoneUsageDescription",
    "The local Finite Chat fixture uses the microphone only when you choose to record audio."
  );
  fs.writeFileSync(infoPath, info);

  signFixtureRuntime(temporaryApp);
  execFileSync("/usr/bin/codesign", [
    "--verify",
    "--deep",
    "--strict",
    temporaryApp,
  ]);
  fs.renameSync(temporaryApp, outputApp);
  return outputExecutable;
}

function signFixtureRuntime(appPath) {
  const frameworks = path.join(appPath, "Contents", "Frameworks");
  const electronFramework = path.join(frameworks, "Electron Framework.framework");
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
    appPath,
  ];
  for (const codePath of nestedCode) {
    const codesignArguments = ["--force", "--sign", "-", "--timestamp=none"];
    if (codePath === appPath) {
      codesignArguments.push(
        "--entitlements",
        path.join(appRoot, "build", "entitlements.mac.plist")
      );
    }
    codesignArguments.push(codePath);
    execFileSync("/usr/bin/codesign", codesignArguments);
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
