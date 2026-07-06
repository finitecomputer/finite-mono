import { notFound } from "next/navigation";

import { ComponentStickerSheet } from "@/components/dev/component-sticker-sheet";

export const dynamic = "force-dynamic";

export default function StickerSheetPage() {
  if (process.env.NODE_ENV !== "development") {
    notFound();
  }

  return <ComponentStickerSheet />;
}
