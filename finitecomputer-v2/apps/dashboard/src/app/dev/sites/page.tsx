import { notFound, redirect } from "next/navigation";

export const dynamic = "force-dynamic";

export default async function SitesPreviewPage({ searchParams }: { searchParams: Promise<{ state?: string }> }) {
  if (process.env.NODE_ENV !== "development") notFound();
  const { state } = await searchParams;
  const query = new URLSearchParams({ preview: "1", state: state ?? "populated" });
  redirect(`/dashboard/machines/runtime_web_design/sites?${query}`);
}
