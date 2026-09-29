"use server";

import { revalidatePath } from "next/cache";
import { createCoreTrialCampaign } from "@/lib/core-client";
import { canAccessAdminOps } from "@/lib/admin-ops";
import { loadOptionalViewerContext } from "@/lib/dashboard-auth";

export async function issueTrialCampaignAction(
  _state: { code?: string; error?: string }, form: FormData,
): Promise<{ code?: string; error?: string }> {
  if (!canAccessAdminOps(await loadOptionalViewerContext())) return { error: "Admin access required." };
  try {
    const issued = await createCoreTrialCampaign({
      name: String(form.get("name") ?? ""),
      seatLimit: Number(form.get("seatLimit")),
      trialDays: Number(form.get("trialDays")),
    });
    revalidatePath("/dashboard/admin");
    return { code: issued.code };
  } catch (error) {
    return { error: error instanceof Error ? error.message : "Could not issue trial code." };
  }
}
