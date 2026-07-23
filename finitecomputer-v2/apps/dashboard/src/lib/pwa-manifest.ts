export const PWA_THEME_COLOR = "#212121";
export const PWA_THEME_COLOR_LIGHT = "#f7f6f3";

export function buildPwaManifest(machineId: string | null) {
  const safeMachineId = sanitizePwaMachineId(machineId);
  const startUrl = safeMachineId
    ? `/dashboard/machines/${encodeURIComponent(safeMachineId)}`
    : "/dashboard";

  return {
    id: startUrl,
    name: safeMachineId ? `Finite ${safeMachineId}` : "Finite.Computer",
    short_name: "Finite",
    description: "A personal agent computer that lives in the cloud.",
    start_url: startUrl,
    scope: "/dashboard",
    display: "standalone",
    background_color: PWA_THEME_COLOR,
    theme_color: PWA_THEME_COLOR,
    icons: [
      {
        src: "/favicon.svg",
        sizes: "any",
        type: "image/svg+xml",
        purpose: "any",
      },
      {
        src: "/icons/icon-192.png",
        sizes: "192x192",
        type: "image/png",
        purpose: "any maskable",
      },
      {
        src: "/icons/icon-512.png",
        sizes: "512x512",
        type: "image/png",
        purpose: "any maskable",
      },
    ],
  };
}

export function sanitizePwaMachineId(value: string | null) {
  const trimmed = value?.trim();
  if (!trimmed) {
    return null;
  }
  return /^[a-zA-Z0-9_-]+$/u.test(trimmed) ? trimmed : null;
}
