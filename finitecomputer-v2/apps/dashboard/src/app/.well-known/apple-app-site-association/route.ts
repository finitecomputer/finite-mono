import { NextResponse } from "next/server";

const ASSOCIATION = {
  applinks: {
    details: [
      {
        appIDs: ["JBLHZ83X6T.computer.finite.finitechat"],
        components: [
          {
            "/": "/auth/ios/callback",
            comment: "Finite Chat AuthKit callback",
          },
        ],
      },
    ],
  },
  webcredentials: {
    apps: ["JBLHZ83X6T.computer.finite.finitechat"],
  },
};

export function GET() {
  return NextResponse.json(ASSOCIATION, {
    headers: {
      "cache-control": "public, max-age=3600",
      "content-type": "application/json",
    },
  });
}
