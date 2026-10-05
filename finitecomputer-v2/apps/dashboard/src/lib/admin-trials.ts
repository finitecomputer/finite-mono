import type { TrialCampaign } from "@/lib/trial-types";

export function trialCampaignInput(form: FormData) {
  const name = String(form.get("name") ?? "").trim();
  if (!name || [...name].length > 120 || /\p{Cc}/u.test(name)) {
    throw new Error("Enter a campaign name of 1–120 characters without control characters.");
  }
  return {
    name,
    seatLimit: wholeNumber(form.get("seatLimit"), 1, 10000, "Total signup limit"),
    trialDays: wholeNumber(form.get("trialDays"), 1, 30, "Trial days"),
  };
}

export function trialCapacityInput(form: FormData) {
  const id = String(form.get("campaignId") ?? "").trim();
  if (!id || id.length > 200) throw new Error("Campaign is required.");
  const seatLimit = wholeNumber(form.get("seatLimit"), 1, 10000, "New total signup limit");
  const expectedSeatLimit = wholeNumber(form.get("expectedSeatLimit"), 1, 10000, "Current signup limit");
  if (seatLimit <= expectedSeatLimit) throw new Error("New total signup limit must exceed the current limit.");
  return { id, seatLimit, expectedSeatLimit };
}

function wholeNumber(value: FormDataEntryValue | null, min: number, max: number, label: string) {
  const number = Number(value);
  if (!Number.isInteger(number) || number < min || number > max) {
    throw new Error(`${label} must be a whole number from ${min} to ${max}.`);
  }
  return number;
}

export function summarizeTrialCampaigns(campaigns: TrialCampaign[]) {
  const seats = campaigns.flatMap(campaign => campaign.redemptions);
  const redeemed = seats.filter(seat => seat.state === "redeemed");
  const unknownAccess = redeemed.filter(seat => !seat.trialAccess).length;
  return {
    limit: campaigns.reduce((sum, campaign) => sum + campaign.seatLimit, 0),
    reserved: campaigns.reduce((sum, campaign) => sum + campaign.reservedSeats, 0),
    redeemed: campaigns.reduce((sum, campaign) => sum + campaign.redeemedSeats, 0),
    remaining: campaigns.filter(campaign => campaign.active).reduce((sum, campaign) => sum + campaign.seatsRemaining, 0),
    attributedAccounts: new Set(seats.map(seat => seat.customerOrgId)).size,
    expiredCheckouts: seats.filter(seat => seat.state === "expired").length,
    // Older Core responses omit trialAccess. Never turn missing data into zero.
    activeTrials: unknownAccess ? null : redeemed.filter(seat => seat.trialAccess?.subscriptionStatus === "trialing" && !seat.trialAccess.blocked).length,
    unknownAccess,
  };
}

export function trialCampaignStatus(campaign: TrialCampaign) {
  if (!campaign.active) return "Inactive";
  return campaign.seatsRemaining > 0 ? "Open" : "Full";
}

export function trialSeatStatus(seat: TrialCampaign["redemptions"][number]) {
  if (seat.state === "reserved") return "In checkout";
  if (seat.state === "expired") return "Checkout expired";
  if (seat.state !== "redeemed") return `Unknown seat state: ${seat.state}`;
  const access = seat.trialAccess;
  if (!access) return "Redeemed · access unknown";
  const status = access.subscriptionStatus;
  if (status === "trialing") return access.blocked ? "Trial ended · access blocked" : "Active trial · access allowed";
  return `${status ?? "Billing unknown"} · access ${access.blocked ? "blocked" : "allowed"}`;
}

export function trialDate(value: string | null) {
  if (!value) return "Unknown";
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? new Intl.DateTimeFormat("en", {
    dateStyle: "medium", timeStyle: "short", timeZone: "UTC",
  }).format(parsed) + " UTC" : "Unknown";
}

export function trialCodeInput(form: FormData) {
  const id = String(form.get("campaignId") ?? "").trim();
  if (!id || id.length > 200) throw new Error("Campaign is required.");
  const code = String(form.get("code") ?? "");
  const compact = code.replace(/[-\t\n\f\r ]/g, "");
  if (code.length > 128 || !/^[A-Za-z0-9]{8,64}$/.test(compact)) {
    throw new Error("Use 8–64 letters or numbers, with optional spaces or hyphens.");
  }
  const revision = form.get("expectedCodeRevision");
  if (revision === null || revision === "") throw new Error("Refresh before editing this code.");
  return { id, code, expectedCodeRevision: wholeNumber(revision, 0, 2147483646, "Code revision") };
}
