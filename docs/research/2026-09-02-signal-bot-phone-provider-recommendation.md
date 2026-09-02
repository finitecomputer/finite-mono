# US number provider for Finite Signal bots

Date: 2026-09-02

Status: READ-ONLY RESEARCH

## Experiment scope

Small-scale internal / open-source test: buy a Telnyx number, register it on
Signal, and message among ourselves. Signal policy, support, and any later
product-integration or automation questions are deferred. They are not a gate
for this experiment.

## Recommendation

**Use Telnyx.** It can search, buy, and later automate US number provisioning
via API. Start with one SMS-capable US local DID (voice fallback if available),
prove Signal registration on a company-controlled phone, then add more numbers
the same way.

No US number provider supplies a supported automated Signal endpoint. Signal
still registers through its Android/iOS app, and chats stay on Signal's
service. Telnyx only supplies the number and the SMS/voice path for the
registration code. That is enough for this experiment. Twilio is a fallback
only if the first Telnyx DID fails Signal registration.

## What a number provider actually supplies

Signal's supported registration flow requires Signal on an Android phone or
iOS device and a number that can receive an ordinary SMS or phone call. Signal
Desktop is a linked client, not an independent account: the primary phone must
come online every 30 days, linked devices are unlinked after 45 days of
inactivity, and each account has at most five linked devices.
[
Register a phone number
](https://support.signal.org/hc/en-us/articles/360007318691-Register-a-phone-number),
[
Linked Devices
](https://support.signal.org/hc/en-us/articles/360007320551-Linked-Devices)

A provider therefore solves number ownership and OTP delivery. Finite still
needs a primary device per number. Signal's public developer documentation
describes protocol specifications and libraries, but the reviewed official
material does not document a supported bot/account-messaging API. [
Signal developer documentation
](https://signal.org/docs/)

## Provider choice for the bounded pilot

| Provider | Verified useful capability | What it does not establish |
| --- | --- | --- |
| **Telnyx — choose for the pilot** | Numbers API can search and order US numbers; search can filter for `sms`, and the response exposes cost information. US local numbers start at $1/month, plus $0.10/month for SMS/MMS capability. [Number search](https://developers.telnyx.com/docs/numbers/phone-numbers/number-search), [buy a number](https://developers.telnyx.com/docs/numbers/phone-numbers/buy-phone-number), [pricing](https://telnyx.com/pricing/numbers) | No Signal integration or acceptance guarantee. Inbound SMS is delivered through a configured messaging-profile webhook; Telnyx says there is no portal inbox, so reading Signal's OTP needs a temporary receiver. [Inbound webhooks](https://support.telnyx.com/en/articles/4348981-receiving-sms-on-your-telnyx-number) |
| Twilio — fallback test only | API/Console can search and buy virtual local, national, mobile, and toll-free numbers; the API exposes Voice/SMS/MMS capabilities. US local numbers start at $1.15/month. [Phone Numbers](https://www.twilio.com/docs/phone-numbers), [capabilities API](https://www.twilio.com/docs/phone-numbers/api/availablephonenumber-resource), [US pricing](https://www.twilio.com/en-us/voice/pricing/us) | No Signal integration or acceptance guarantee; its programmable messaging APIs still do not provide Signal account registration or Signal message transport. |

Telnyx is the least disruptive choice because Finite already investigated its
account, it has the desired search/order primitives, and it is cheaper at the
published starting rate. That is a provider-operability recommendation for a
manual experiment—not evidence that Telnyx DIDs are reliably accepted by
Signal.

## Actionable next step

Buy **one** SMS-capable Telnyx US DID (voice fallback if available), receive
the OTP through a temporary controlled webhook, register it manually on a
company-controlled Android/iOS device, and test a few internal conversations.
If that number registers cleanly, buy a few more the same way. If it fails at
registration, test one Twilio number as a compatibility comparison.
