// Self-contained QR rendering: qrcode-generator (zero dependencies) computes
// the module matrix and we emit a single SVG path in module units, so server
// components can render the code inline with no external image service.

import qrcode from "qrcode-generator";

export type QrSvgModel = {
  // Modules per side; the SVG viewBox spans [0, moduleCount] in both axes.
  moduleCount: number;
  // One 1x1 square per dark module.
  path: string;
};

export function qrSvgModel(text: string): QrSvgModel {
  const trimmed = text.trim();
  if (!trimmed) {
    throw new Error("QR code text is required.");
  }

  const qr = qrcode(0, "M");
  qr.addData(trimmed);
  qr.make();

  const moduleCount = qr.getModuleCount();
  const squares: string[] = [];
  for (let row = 0; row < moduleCount; row += 1) {
    for (let col = 0; col < moduleCount; col += 1) {
      if (qr.isDark(row, col)) {
        squares.push(`M${col} ${row}h1v1h-1z`);
      }
    }
  }

  return { moduleCount, path: squares.join("") };
}
