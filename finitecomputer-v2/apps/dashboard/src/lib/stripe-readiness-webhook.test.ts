import assert from "node:assert/strict";
import test from "node:test";

import {
  FINITE_STRIPE_EVENTS,
  FINITE_STRIPE_WEBHOOK_URL,
} from "./stripe-readiness";
import {
  selectStripeReadinessWebhook,
  stripeReadinessErrorMessage,
  type StripeReadinessEventDestination,
} from "./stripe-readiness-webhook";

function destination(id = "ed_live_finite"): StripeReadinessEventDestination {
  return {
    id,
    livemode: true,
    status: "enabled",
    type: "webhook_endpoint",
    event_payload: "snapshot",
    events_from: ["@self"],
    enabled_events: [...FINITE_STRIPE_EVENTS],
    snapshot_api_version: "2024-06-20",
    webhook_endpoint: { url: FINITE_STRIPE_WEBHOOK_URL },
  };
}

test("a unique URL retains the existing selection contract and reports its ID", async () => {
  const unrelated = destination("ed_live_unrelated");
  unrelated.webhook_endpoint!.url = "https://example.com/webhook";
  const selected = await selectStripeReadinessWebhook([unrelated, destination()]);
  assert.equal(selected?.id, "ed_live_finite");
  assert.deepEqual(selected?.enabledEvents, [...FINITE_STRIPE_EVENTS]);
  assert.equal(await selectStripeReadinessWebhook([unrelated]), null);
});

test("duplicate URLs fail closed regardless of ordering or individual event coverage", async () => {
  const first = destination("ed_live_first");
  const second = destination("ed_live_second");
  second.enabled_events = ["checkout.session.completed"];
  for (const endpoints of [[first, second], [second, first]]) {
    await assert.rejects(selectStripeReadinessWebhook(endpoints), /selection is ambiguous/);
  }
});

test("selection consumes later pages before accepting a unique URL", async () => {
  async function* pages() {
    yield destination("ed_live_page_one");
    await Promise.resolve();
    yield destination("ed_live_page_two");
  }
  await assert.rejects(selectStripeReadinessWebhook(pages()), /selection is ambiguous/);
  const selected = await selectStripeReadinessWebhook(pages(), "ed_live_page_two");
  assert.equal(selected?.id, "ed_live_page_two");
});

test("an explicit ID selects that destination without merging a sibling's event set", async () => {
  const first = destination("ed_live_first");
  const second = destination("ed_live_second");
  first.enabled_events = first.enabled_events.filter((event) => event !== "checkout.session.expired");
  second.enabled_events = second.enabled_events.filter((event) => event !== "checkout.session.completed");
  const selected = await selectStripeReadinessWebhook([first, second], second.id);
  assert.equal(selected?.id, second.id);
  assert.deepEqual(selected?.enabledEvents, second.enabled_events);
  assert.equal(selected?.enabledEvents.includes("checkout.session.completed"), false);
});

test("an unknown explicit ID never falls back to a complete matching URL", async () => {
  await assert.rejects(
    selectStripeReadinessWebhook([destination()], "ed_live_missing"),
    /expected webhook destination was not found/,
  );
});

test("an explicit destination at the wrong URL is rejected despite a valid sibling", async () => {
  const wrong = destination("ed_live_wrong_url");
  wrong.webhook_endpoint!.url = "https://example.com/webhook";
  await assert.rejects(
    selectStripeReadinessWebhook([destination(), wrong], wrong.id),
    /does not use the Finite production webhook URL/,
  );
});

test("selection excludes signing secrets and unrelated provider fields", async () => {
  const secret = "whsec_synthetic_must_not_appear";
  const raw = {
    ...destination(),
    metadata: { private: secret },
    webhook_endpoint: { url: FINITE_STRIPE_WEBHOOK_URL, signing_secret: secret },
  };
  const selected = await selectStripeReadinessWebhook([raw]);
  assert.equal(JSON.stringify(selected).includes(secret), false);
  assert.equal(JSON.stringify(selected).includes("signing_secret"), false);
});

test("late-page failures fail closed and provider errors cannot expose credentials", async () => {
  const secret = "rk_live_synthetic_must_not_appear";
  async function* failedPages() {
    yield destination();
    throw new Error(`Invalid API Key provided: ${secret}`);
  }
  await assert.rejects(selectStripeReadinessWebhook(failedPages()), (error: unknown) => {
    const message = stripeReadinessErrorMessage(error);
    assert.equal(message.includes(secret), false);
    assert.match(message, /provider error details are withheld/);
    return true;
  });
});

test("selection errors explain recovery without echoing supplied IDs or URLs", async () => {
  const secret = "whsec_synthetic_must_not_appear";
  await assert.rejects(selectStripeReadinessWebhook([destination()], secret), (error: unknown) => {
    const message = stripeReadinessErrorMessage(error);
    assert.match(message, /expected webhook destination was not found/);
    assert.equal(message.includes(secret), false);
    return true;
  });
  const wrong = destination();
  wrong.webhook_endpoint!.url = `https://example.com/${secret}`;
  await assert.rejects(selectStripeReadinessWebhook([wrong], wrong.id), (error: unknown) => {
    assert.equal(stripeReadinessErrorMessage(error).includes(secret), false);
    return true;
  });
});
