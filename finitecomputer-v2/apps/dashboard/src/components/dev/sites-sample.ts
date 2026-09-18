import type { SiteListItem } from "@/components/sites-browser";

// Development-only presentation fixtures; never authorize access from these.
export const sampleSites: SiteListItem[] = [
  { id: "azimuth", title: "Azimuth — The World From Nashville", url: "https://azimuth.example.test", updatedLabel: "3w", access: "private", accessLabel: "Only you", thumbnailUrl: "/sites-preview/azimuth.svg" },
  { id: "mara", title: "Mara Vale Portfolio", url: "https://mara-vale.example.test", updatedLabel: "1mo", access: "private", accessLabel: "Only you", thumbnailUrl: "/sites-preview/portfolio.svg" },
  { id: "studio", title: "Studio Field Notes", url: "https://studio.example.test", updatedLabel: "2d", access: "shared", accessLabel: "Design team", thumbnailUrl: "/sites-preview/portfolio.svg" },
];
