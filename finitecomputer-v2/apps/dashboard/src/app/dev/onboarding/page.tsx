import { notFound } from "next/navigation";

import { OnboardingFlowPreview } from "@/components/dev/onboarding-flow-preview";

export const dynamic = "force-dynamic";

export default function OnboardingPreviewPage() {
  if (process.env.NODE_ENV !== "development") {
    notFound();
  }

  return <OnboardingFlowPreview />;
}
