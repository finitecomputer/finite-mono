import assert from "node:assert/strict";
import test from "node:test";
import { summarizeTrialCampaigns, trialCampaignInput, trialCapacityInput, trialCampaignStatus, trialSeatStatus } from "./admin-trials";
import type { TrialCampaign } from "./trial-types";

const campaign: TrialCampaign = {
  id: "c", name: "Workshop", active: true, seatLimit: 10, trialDays: 7,
  redeemedSeats: 2, reservedSeats: 1, seatsRemaining: 7,
  redemptions: [
    { customerOrgId: "a", ownerWorkosUserId: null, state: "expired", redeemedAt: null },
    { customerOrgId: "a", ownerWorkosUserId: null, state: "reserved", redeemedAt: null },
    { customerOrgId: "b", ownerWorkosUserId: null, state: "redeemed", redeemedAt: "2026-10-01", trialAccess: { blocked: false, eventName: "Workshop", subscriptionStatus: "trialing", periodEnd: "2026-10-08" } },
    { customerOrgId: "c", ownerWorkosUserId: null, state: "redeemed", redeemedAt: "2026-10-01", trialAccess: { blocked: false, eventName: "Workshop", subscriptionStatus: "active", periodEnd: "2026-11-01" } },
  ],
};

test("capacity separates occupied seats, attribution, expired attempts, and active trials", () => {
  assert.deepEqual(summarizeTrialCampaigns([campaign]), {
    limit: 10, reserved: 1, redeemed: 2, remaining: 7,
    attributedAccounts: 3, expiredCheckouts: 1, activeTrials: 1, unknownAccess: 0,
  });
  assert.equal(summarizeTrialCampaigns([{ ...campaign, active: false }]).remaining, 0);
  assert.equal(trialCampaignStatus({ ...campaign, active: false }), "Inactive");
  assert.equal(trialCampaignStatus({ ...campaign, seatsRemaining: 0 }), "Full");
  assert.equal(summarizeTrialCampaigns([{ ...campaign, redemptions: campaign.redemptions.map(seat => ({ ...seat, trialAccess: undefined })) }]).activeTrials, null);
  assert.equal(summarizeTrialCampaigns([]).activeTrials, 0);
});

test("billing and access are independent from redemption and checkout expiry", () => {
  assert.equal(trialSeatStatus(campaign.redemptions[0]), "Checkout expired");
  assert.equal(trialSeatStatus(campaign.redemptions[1]), "In checkout");
  assert.equal(trialSeatStatus(campaign.redemptions[2]), "Active trial · access allowed");
  assert.equal(trialSeatStatus(campaign.redemptions[3]), "active · access allowed");
  const redeemed = campaign.redemptions[2];
  assert.equal(trialSeatStatus({ ...redeemed, trialAccess: undefined }), "Redeemed · access unknown");
  for (const status of ["past_due", "unpaid", "canceled", "incomplete", "incomplete_expired", "paused", null]) {
    assert.match(trialSeatStatus({ ...redeemed, trialAccess: { ...redeemed.trialAccess!, subscriptionStatus: status, blocked: true } }), /access blocked/);
  }
  assert.equal(trialSeatStatus({ ...redeemed, trialAccess: { ...redeemed.trialAccess!, blocked: true } }), "Trial ended · access blocked");
});

test("campaign form preserves existing bounds and capacity updates use absolute totals", () => {
  const form = new FormData();
  form.set("name", "  Workshop  "); form.set("seatLimit", "10"); form.set("trialDays", "7");
  assert.deepEqual(trialCampaignInput(form), { name: "Workshop", seatLimit: 10, trialDays: 7 });
  for (const invalid of ["", "0", "1.5", "10001", "Infinity"]) {
    form.set("seatLimit", invalid); assert.throws(() => trialCampaignInput(form));
  }
  form.set("seatLimit", "15"); form.set("campaignId", "c"); form.set("expectedSeatLimit", "10");
  assert.deepEqual(trialCapacityInput(form), { id: "c", seatLimit: 15, expectedSeatLimit: 10 });
  form.set("seatLimit", "5"); assert.throws(() => trialCapacityInput(form), /must exceed/);
  form.set("seatLimit", "10"); form.set("name", "bad\u0085name"); assert.throws(() => trialCampaignInput(form), /campaign name/);
  form.set("name", "ok"); form.set("trialDays", "31"); assert.throws(() => trialCampaignInput(form), /Trial days/);
});
