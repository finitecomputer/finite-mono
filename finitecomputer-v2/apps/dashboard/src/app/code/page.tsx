import { LaunchCodeForm } from "@/components/launch-code-form";
import { getAccountAuthContext } from "@/lib/dashboard-auth";

export default async function LaunchCodePage() {
  const account = await getAccountAuthContext();
  return <LaunchCodeForm signedIn={Boolean(account.workosUserId && account.emailVerified)} />;
}
