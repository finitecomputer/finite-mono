"use client";

import {
  ArrowUpIcon,
  BotIcon,
  CommandIcon,
  LayoutDashboardIcon,
  PaperclipIcon,
  PlugIcon,
  SquareIcon,
} from "lucide-react";

import {
  FiniteLoader,
  type FiniteLoaderVariant,
} from "@/components/finite-loader";
import { Button } from "@/components/ui/button";
import { CONTROL_PRIMITIVES, OCEAN_TOKENS, SHADCN_TOKENS } from "@/lib/design-tokens";
import { statusBadgeToneClass, type StatusBadgeState } from "@/lib/status-badge-tone";
import { cn } from "@/lib/utils";

import { StickerBlock, StickerRow } from "./sticker-sheet-section";

const STATUS_SAMPLES: StatusBadgeState[] = ["pending", "in_progress", "complete", "blocked"];
const LOADER_VARIANTS: { label: string; value: FiniteLoaderVariant }[] = [
  { label: "Lines top → bottom", value: "line-cycle" },
  { label: "Wipe left → right", value: "wipe-right" },
  { label: "Grow from middle", value: "grow-middle" },
  { label: "Slide up", value: "rise" },
  { label: "Slide left", value: "left" },
  { label: "Slide right", value: "right" },
  { label: "Alternating slide", value: "split" },
  { label: "Lights up", value: "scan-up" },
  { label: "Lights down", value: "scan-down" },
  { label: "Center out", value: "center-out" },
  { label: "Edges in", value: "edges-in" },
  { label: "Odd / even", value: "alternate" },
  { label: "Signal", value: "signal" },
];

function TokenSwatches({ tokens }: { tokens: typeof OCEAN_TOKENS }) {
  return (
    <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4">
      {tokens.map((token) => (
        <div key={token.cssVar} className="overflow-hidden rounded-lg border border-border">
          <div
            className="h-12 border-b border-border"
            style={{ background: `var(${token.cssVar})` }}
            title={token.cssVar}
          />
          <div className="bg-card px-2 py-1.5">
            <p className="type-mono-sm text-muted-foreground">{token.name}</p>
            <p className="type-caption text-muted-foreground">{token.description}</p>
          </div>
        </div>
      ))}
    </div>
  );
}

export function StickerSheetDesignSystem() {
  return (
    <>
      <div className="mb-8 border-t border-border pt-10">
        <h2 className="type-title-2">Design system</h2>
        <p className="mt-1 type-body-sm text-muted-foreground">
          Semantic tokens and control primitives extracted from the dashboard and chat surfaces.
        </p>
      </div>

      <StickerBlock title="Ocean semantic colors">
        <TokenSwatches tokens={OCEAN_TOKENS.filter((entry) => entry.group === "color")} />
      </StickerBlock>

      <StickerBlock title="Shadcn semantic colors">
        <TokenSwatches tokens={SHADCN_TOKENS} />
      </StickerBlock>

      <StickerBlock title="Status badges">
        <StickerRow>
          {STATUS_SAMPLES.map((status) => (
            <span key={status} className={statusBadgeToneClass(status)}>
              {status.replace("_", " ")}
            </span>
          ))}
        </StickerRow>
      </StickerBlock>

      <StickerBlock title="Finite loading sequences">
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-5">
          {LOADER_VARIANTS.map((variant) => (
            <div
              className="flex min-h-40 flex-col items-center justify-center gap-4 rounded-xl border border-border bg-card p-5"
              key={variant.value}
            >
              <FiniteLoader
                label={`${variant.label} loading animation`}
                size={76}
                variant={variant.value}
              />
              <span className="type-caption text-muted-foreground">{variant.label}</span>
            </div>
          ))}
        </div>
      </StickerBlock>

      <StickerBlock title="Application button sizes">
        <StickerRow>
          <Button size="default">Default</Button>
          <Button size="lg">Large</Button>
          <Button size="xl">Extra large</Button>
        </StickerRow>
      </StickerBlock>

      <StickerBlock title="Icon button">
        <StickerRow>
          <button type="button" className="ocean-icon-button" aria-label="Menu">
            <LayoutDashboardIcon className="size-4" />
          </button>
          <button type="button" className="ocean-icon-button" aria-label="Connections">
            <PlugIcon className="size-4" />
          </button>
        </StickerRow>
      </StickerBlock>

      <StickerBlock title="Pill button">
        <StickerRow>
          <button type="button" className="ocean-pill-button">
            Secondary action
          </button>
          <button type="button" className="ocean-pill-button" disabled>
            Disabled
          </button>
        </StickerRow>
      </StickerBlock>

      <StickerBlock title="Composer controls">
        <StickerRow className="items-end">
          <button type="button" className="ocean-control-tool" aria-label="Attach">
            <PaperclipIcon className="size-4" />
          </button>
          <button type="button" className="ocean-control-command" aria-label="Commands">
            <CommandIcon className="size-4" />
            <span>Commands</span>
          </button>
          <button type="button" className="ocean-control-tool ocean-control-stop" aria-label="Stop">
            <SquareIcon className="size-4" />
          </button>
          <button type="button" className="ocean-control-send" aria-label="Send">
            <ArrowUpIcon className="size-4" />
          </button>
        </StickerRow>
      </StickerBlock>

      <StickerBlock title="Segmented control">
        <nav className="ocean-section-tabs max-w-md" aria-label="Demo sections">
          <span className="ocean-section-tab">
            <BotIcon className="size-4" />
            <span>Agents</span>
          </span>
          <span className="ocean-section-tab is-active">
            <LayoutDashboardIcon className="size-4" />
            <span>Overview</span>
          </span>
          <span className="ocean-section-tab">
            <PlugIcon className="size-4" />
            <span>Connections</span>
          </span>
          <span className={cn("ocean-section-tab", "is-disabled")} aria-disabled="true">
            <span>Skills</span>
          </span>
        </nav>
      </StickerBlock>

      <StickerBlock title="Control catalog">
        <div className="overflow-x-auto rounded-xl border border-border">
          <table className="w-full min-w-[640px] border-collapse text-left">
            <thead>
              <tr className="border-b border-border type-label text-muted-foreground">
                <th className="px-3 py-2">Name</th>
                <th className="px-3 py-2">Class</th>
                <th className="px-3 py-2">Alias</th>
                <th className="px-3 py-2">Use</th>
              </tr>
            </thead>
            <tbody>
              {CONTROL_PRIMITIVES.map((control) => (
                <tr key={control.className} className="border-b border-border/60 type-body-sm">
                  <td className="px-3 py-2 font-medium">{control.name}</td>
                  <td className="px-3 py-2">
                    <code className="type-mono-sm">{control.className}</code>
                  </td>
                  <td className="px-3 py-2 text-muted-foreground">
                    <code className="type-mono-sm">{control.alias}</code>
                  </td>
                  <td className="px-3 py-2 text-muted-foreground">{control.use}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </StickerBlock>
    </>
  );
}
