import type { Metadata, Viewport } from "next";
import localFont from "next/font/local";
import { TooltipProvider } from "@/components/ui/tooltip";
import { ThemeProvider } from "@/components/theme-provider";
import { PWA_THEME_COLOR, PWA_THEME_COLOR_LIGHT } from "@/lib/pwa-manifest";
import "./globals.css";

const funnelSans = localFont({
  src: [
    { path: "./fonts/funnel-sans-400.ttf", weight: "400", style: "normal" },
    { path: "./fonts/funnel-sans-500.ttf", weight: "500", style: "normal" },
    { path: "./fonts/funnel-sans-600.ttf", weight: "600", style: "normal" },
    { path: "./fonts/funnel-sans-700.ttf", weight: "700", style: "normal" },
  ],
  variable: "--font-funnel-sans",
  display: "swap",
});

const funnelDisplay = localFont({
  src: [
    { path: "./fonts/funnel-display-500.ttf", weight: "500", style: "normal" },
    { path: "./fonts/funnel-display-600.ttf", weight: "600", style: "normal" },
    { path: "./fonts/funnel-display-700.ttf", weight: "700", style: "normal" },
  ],
  variable: "--font-funnel-display",
  display: "swap",
});

const jetbrainsMono = localFont({
  src: [
    { path: "./fonts/jetbrains-mono-400.ttf", weight: "400", style: "normal" },
    { path: "./fonts/jetbrains-mono-500.ttf", weight: "500", style: "normal" },
    { path: "./fonts/jetbrains-mono-600.ttf", weight: "600", style: "normal" },
  ],
  variable: "--font-jetbrains-mono",
  display: "swap",
});

function metadataBaseUrl() {
  const candidates = [
    process.env.NEXT_PUBLIC_DASHBOARD_BASE_URL,
    process.env.FC_DASHBOARD_BASE_URL,
    process.env.NEXT_PUBLIC_WORKOS_REDIRECT_URI,
    "https://finite.computer",
  ];

  for (const candidate of candidates) {
    if (!candidate) {
      continue;
    }

    try {
      return new URL(candidate).origin;
    } catch {
      continue;
    }
  }

  return "https://finite.computer";
}

export const metadata: Metadata = {
  metadataBase: new URL(metadataBaseUrl()),
  title: "Finite.Computer",
  description:
    "Finite makes frontier AI accessible to non-developers through in-person training and beautifully simple agent software.",
  openGraph: {
    title: "Finite Computer",
    description:
      "Finite makes frontier AI accessible to non-developers through in-person training and beautifully simple agent software.",
    images: [{ url: "/marketing/finite-meadow.webp" }],
  },
  icons: {
    icon: [
      { url: "/favicon.svg", type: "image/svg+xml" },
      { url: "/icons/icon-192.png", sizes: "192x192", type: "image/png" },
      { url: "/icons/icon-512.png", sizes: "512x512", type: "image/png" },
    ],
    shortcut: [{ url: "/favicon.svg", type: "image/svg+xml" }],
    apple: [{ url: "/icons/icon-180.png", sizes: "180x180", type: "image/png" }],
  },
  appleWebApp: {
    capable: true,
    title: "Finite.Computer",
    statusBarStyle: "black-translucent",
  },
};

export const viewport: Viewport = {
  viewportFit: "cover",
  themeColor: [
    { media: "(prefers-color-scheme: dark)", color: PWA_THEME_COLOR },
    { media: "(prefers-color-scheme: light)", color: PWA_THEME_COLOR_LIGHT },
  ],
  colorScheme: "dark light",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html
      lang="en"
      className={`${funnelSans.variable} ${funnelDisplay.variable} ${jetbrainsMono.variable} h-full antialiased`}
      suppressHydrationWarning
    >
      <body className="min-h-full flex flex-col">
        <ThemeProvider>
          <TooltipProvider>{children}</TooltipProvider>
        </ThemeProvider>
      </body>
    </html>
  );
}
