"use client";

import { MonitorIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import "@/styles/ocean-shell.css";

import { StickerSheetDesignSystem } from "./sticker-sheet-design-system";
import { StickerSheetPrimitives } from "./sticker-sheet-primitives";
import { StickerPageHeader } from "./sticker-sheet-section";

export function ComponentStickerSheet() {
  return (
    <div className="ocean-shell min-h-screen">
      <StickerPageHeader
        title="Component sticker sheet"
        description="The narrow visual source of truth for dashboard typography, tokens, buttons, status badges, and Ocean shell controls."
        action={
          <Button type="button" variant="outline" size="sm" disabled>
            <MonitorIcon />
            System theme
          </Button>
        }
      />

      <main className="px-6 py-8">
        <StickerSheetPrimitives />
        <StickerSheetDesignSystem />
      </main>
    </div>
  );
}
