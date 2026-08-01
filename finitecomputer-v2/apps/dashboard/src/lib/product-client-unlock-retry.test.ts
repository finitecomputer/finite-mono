import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { retryProductClientUnlock } from "../../scripts/product-client-unlock-retry";

describe("Product Client unlock retry", () => {
  it("reloads when a rendered Product Client stalls before unlock", async () => {
    const attemptTimeouts: number[] = [];
    let reloads = 0;

    await retryProductClientUnlock({
      timeoutMs: 90_000,
      waitForUnlock: async (timeoutMs) => {
        attemptTimeouts.push(timeoutMs);
        if (attemptTimeouts.length === 1) {
          throw new Error("Brain did not unlock; current status: Brain locked");
        }
      },
      reload: async () => {
        reloads += 1;
      },
    });

    assert.deepEqual(attemptTimeouts, [30_000, 30_000]);
    assert.equal(reloads, 1);
  });

  it("preserves the final unlock diagnostic after bounded retries", async () => {
    let attempts = 0;
    let reloads = 0;

    await assert.rejects(
      retryProductClientUnlock({
        timeoutMs: 90_000,
        waitForUnlock: async () => {
          attempts += 1;
          throw new Error(`unlock attempt ${attempts} remained locked`);
        },
        reload: async () => {
          reloads += 1;
        },
      }),
      /unlock attempt 3 remained locked/u,
    );

    assert.equal(attempts, 3);
    assert.equal(reloads, 2);
  });
});
