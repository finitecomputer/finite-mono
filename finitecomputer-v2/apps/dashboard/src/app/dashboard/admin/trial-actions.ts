"use server";

import { revalidatePath } from "next/cache";
import { createCoreTrialCampaign, increaseCoreTrialCapacity, updateCoreTrialCode } from "@/lib/core-client";
import { canAccessAdminOps } from "@/lib/admin-ops";
import { trialCampaignInput, trialCapacityInput, trialCodeInput } from "@/lib/admin-trials";
import { loadOptionalViewerContext } from "@/lib/dashboard-auth";

type TrialActionState = { campaignId?: string; code?: string; error?: string; message?: string };

async function trialAdminError() {
  if (!canAccessAdminOps(await loadOptionalViewerContext())) return "Admin access required.";
  if (process.env.FC_DASHBOARD_TRIALS_ENABLED !== "true") return "Trial checkout is disabled in this dashboard.";
  return null;
}

export async function issueTrialCampaignAction(
  _state: TrialActionState, form: FormData,
): Promise<TrialActionState> {
  const error = await trialAdminError();
  if (error) return { error };
  try {
    const input = trialCampaignInput(form);
    const issued = await createCoreTrialCampaign(input);
    revalidatePath("/dashboard/admin");
    return { campaignId: issued.id, code: issued.code, message: `${input.name} · ${input.seatLimit} total seats · ${input.trialDays}-day trial` };
  } catch (error) {
    return { error: error instanceof Error ? error.message : "Could not issue trial code." };
  }
}

export async function increaseTrialCapacityAction(
  _state: TrialActionState, form: FormData,
): Promise<TrialActionState> {
  const error = await trialAdminError();
  if (error) return { error };
  try {
    const { id, ...input } = trialCapacityInput(form);
    await increaseCoreTrialCapacity(id, input);
    revalidatePath("/dashboard/admin");
    return { message: `Signup limit increased to ${input.seatLimit} total seats (+${input.seatLimit - input.expectedSeatLimit}).` };
  } catch (error) {
    return { error: error instanceof Error ? error.message : "Could not increase capacity." };
  }
}

export async function updateTrialCodeAction(
  _state: TrialActionState, form: FormData,
): Promise<TrialActionState> {
  const error = await trialAdminError();
  if (error) return { error };
  try {
    const { id, ...input } = trialCodeInput(form);
    await updateCoreTrialCode(id, input);
    revalidatePath("/dashboard/admin");
    return { message: "Code saved. Existing enrollments and capacity are unchanged." };
  } catch (error) {
    return { error: error instanceof Error ? error.message : "Could not update trial code." };
  }
}
