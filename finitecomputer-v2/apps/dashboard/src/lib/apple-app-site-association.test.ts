import assert from "node:assert/strict";
import test from "node:test";

import { GET } from "@/app/.well-known/apple-app-site-association/route";

test("Apple association binds only the Finite Chat AuthKit callback", async () => {
  const response = GET();
  assert.equal(response.status, 200);
  assert.match(response.headers.get("content-type") ?? "", /^application\/json/u);
  assert.deepEqual(await response.json(), {
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
  });
});
