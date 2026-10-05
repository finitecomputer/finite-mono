import assert from "node:assert/strict";
import { once } from "node:events";
import { createServer } from "node:http";
import test from "node:test";
import { issueTrialCampaignAction, increaseTrialCapacityAction, updateTrialCodeAction } from "@/app/dashboard/admin/trial-actions";

test("trial server actions reject non-admins and disabled trials before any mutation", async (t) => {
  const saved = { ...process.env };
  t.after(() => { process.env = saved; });
  let writes = 0;
  const core = createServer((request, response) => {
    if (request.method !== "GET") writes++;
    response.setHeader("content-type", "application/json");
    response.end(JSON.stringify({ email: "member@example.test", workos_user_id: "member", projects: [], claimable_candidates: [], agent_creation_requests: [] }));
  });
  core.listen(0, "127.0.0.1"); await once(core, "listening");
  t.after(() => { core.closeAllConnections(); core.close(); });
  const address = core.address(); assert(address && typeof address !== "string");
  Object.assign(process.env, {
    FC_CORE_BASE_URL: `http://127.0.0.1:${address.port}`, FC_CORE_API_TOKEN: "fixture",
    FC_WORKOS_AUTH_ENABLED: "0", FC_WORKOS_OPERATOR_ORG_ID: "operator",
    FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1", FC_DASHBOARD_DEV_EMAIL: "member@example.test",
    FC_DASHBOARD_DEV_WORKOS_USER_ID: "member", FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: "fixture",
    FC_DASHBOARD_DEV_ADMIN_EMAILS: "", FC_DASHBOARD_TRIALS_ENABLED: "true",
  });
  const form = new FormData();
  form.set("name", "Forged"); form.set("seatLimit", "15"); form.set("trialDays", "7");
  form.set("campaignId", "c"); form.set("expectedSeatLimit", "10"); form.set("isAdmin", "true");
  for (const action of [issueTrialCampaignAction, increaseTrialCapacityAction, updateTrialCodeAction]) {
    assert.deepEqual(await action({}, form), { error: "Admin access required." });
  }
  process.env.FC_DASHBOARD_DEV_ADMIN_EMAILS = "member@example.test";
  process.env.FC_DASHBOARD_TRIALS_ENABLED = "false";
  for (const action of [issueTrialCampaignAction, increaseTrialCapacityAction, updateTrialCodeAction]) {
    assert.deepEqual(await action({}, form), { error: "Trial checkout is disabled in this dashboard." });
  }
  assert.equal(writes, 0);
});
