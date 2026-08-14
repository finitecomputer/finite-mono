#!/usr/bin/env node
const envAliases = [
  ["SKILL_AB_DEVFINITY_PROMPT_FILE", "DEVFINITY_AGENT_RUN_PROMPT_FILE"],
  ["SKILL_AB_DEVFINITY_RESULT_FILE", "DEVFINITY_AGENT_RUN_OUTPUT_FILE"],
  ["SKILL_AB_DEVFINITY_RUNTIME_OUTPUT_PATH", "DEVFINITY_AGENT_RUN_RUNTIME_OUTPUT_PATH"],
  ["SKILL_AB_DEVFINITY_SKILL_BUNDLE_DIR", "DEVFINITY_AGENT_RUN_SKILL_BUNDLE_DIR"],
  ["SKILL_AB_DEVFINITY_VARIANT", "DEVFINITY_AGENT_RUN_LABEL"],
  ["SKILL_AB_DEVFINITY_REPLY_TIMEOUT_MS", "DEVFINITY_AGENT_RUN_REPLY_TIMEOUT_MS"],
];

for (const [legacyName, genericName] of envAliases) {
  if (!process.env[legacyName] && process.env[genericName]) {
    process.env[legacyName] = process.env[genericName];
  }
}

try {
  await import("../../finite-skills/ab-testing/scripts/run-devfinity-agent-turn.mjs");
} catch (error) {
  console.error(error?.stack || error?.message || String(error));
  process.exit(1);
}
