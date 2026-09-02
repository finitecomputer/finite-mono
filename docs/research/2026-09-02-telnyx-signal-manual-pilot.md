# Telnyx numbers for a manual Signal bot pilot

Date: 2026-09-02

Status: READ-ONLY RESEARCH

This note is the planning investigation for a small-scale internal /
open-source experiment. Signal policy and support are deferred; they are not a
gate for buying a number and testing registration. Prices and account rules
are live service information and should be rechecked in the portal immediately
before paying. The live signup/order record is
[the experiment log](./2026-09-02-telnyx-signal-experiment-log.md).

## The short answer

Telnyx can supply a US SMS-capable phone number and carry the SMS or voice call
used to receive Signal's registration code. It does not carry Signal messages.
Signal's official registration flow requires the number to receive an ordinary
SMS or phone call, then runs chats through Signal on a primary Android or iOS
device. Signal's documentation says Signal messages are sent over the internet,
not traditional SMS/MMS. ([Signal registration](https://support.signal.org/hc/en-us/articles/360007318691-Register-a-phone-number),
[Signal permissions and network behavior](https://support.signal.org/hc/en-us/articles/360007062172-Signal-Permissions-OS-Notification-Settings))

That makes the first experiment a device-backed manual setup: one Signal
primary device per number, with a desktop or other linked device for
operators. It is not a Telnyx-only configuration. Official Signal material
reviewed here does not describe a supported bot/API account path. That is an
operational fact for later product work, not a reason to delay this test.

## What signing up and paying would involve

### Verified Telnyx account path

Telnyx's signup guide asks for contact information, company details, and a
secure password, followed by email verification. The Help Center says a
business email is mandatory, KYC may be requested, and phone verification may
be required after email verification. A use-case questionnaire is shown after
activation. ([Create a Telnyx account](https://developers.telnyx.com/docs/account-setup/create-account),
[Telnyx signup Help Center](https://support.telnyx.com/en/articles/5295540-how-to-sign-up-for-a-telnyx-account))

The account-level model is in transition. Under the legacy Level 1/Level 2
model, most users become Level 1 after email confirmation; Level 1 permits a
local number in the account's home country and assigning a messaging profile,
but the Help Center lists a 50-outbound-message-per-day limit. Level 2 removes
many restrictions and requires a payment method, company name, and contact
number; review can take up to 48 hours. Newer signups may instead show the
Trial/Paid/Verified/Enterprise (TPVE) Account Levels page. Check which page
Austin's account actually shows before relying on any limit. ([Account
Verification](https://support.telnyx.com/en/articles/1130595-account-verification),
[Phone-number ordering restrictions](https://support.telnyx.com/en/articles/10715715-phone-number-ordering-restrictions))

Telnyx operates a balance/top-up model in Mission Control. The Billing area
manages balance, payment method, scheduled payments/auto-recharge, and
invoices. The Help Center lists credit card, PayPal, Bitcoin, and ACH Direct
Debit. New users are documented as limited to $100 of payments on day one,
with the limit increasing by $50 per day; payment-method review can take up to
48 hours. The same guide says the minimum payment is $10 and that the billing
address normally must match the country of the phone number used to verify the
account (Level 2 removes that restriction). ([Billing setup](https://support.telnyx.com/en/articles/4280500-billing-setup-billing-groups))

Do not budget on trial credit without checking the live account. Telnyx's
developer setup page says new accounts come with free testing credits, while
the signup Help Center currently says there is no promotion providing free
trial credit. ([Developer account setup](https://developers.telnyx.com/docs/account-setup/create-account),
[Signup Help Center](https://support.telnyx.com/en/articles/5295540-how-to-sign-up-for-a-telnyx-account))

### Buying the first numbers

Mission Control's Search & Buy Numbers flow searches by country, feature,
type, area code, or city/region/state. The portal shows each number's one-time
upfront cost and recurring monthly cost. Numbers can be assigned to a
Messaging Profile during the order. The order total is deducted from the
balance; orders are final and mistaken purchases are not refundable. Telnyx
recommends no more than 50 numbers per cart; larger bulk orders should go
through sales. ([Search and Buy Numbers](https://support.telnyx.com/en/articles/4380325-search-and-buy-numbers))

The public pay-as-you-go price currently starts at **$1/month for a local
number**, with an additional **$0.10/month** to add SMS/MMS capability. Thus a
plain SMS-capable local DID has a public starting-point estimate of **$1.10 per
month before message/carrier fees, taxes, and any number-specific upfront
charge**. The actual Austin-area number may be priced differently; use the
portal's displayed monthly amount as authoritative. ([Global numbers pricing](https://telnyx.com/pricing/numbers))

Illustrative recurring number-only floor (not a quote):

| SMS-capable local DIDs | Starting monthly estimate |
| ---: | ---: |
| 1 | $1.10 |
| 3 | $3.30 |
| 5 | $5.50 |
| 10 | $11.00 |

Telnyx bills monthly recurring charges at the start of each month from the
account balance. Deleting a number stops its monthly recurring charge; a
deleted number can generally be repurchased for 15 days, with a prorated MRC
on repurchase. A prolonged negative balance can lead to account abolition and
number release, so a small auto-recharge or balance alert matters even for a
low-volume pilot. ([Monthly charges report](https://support.telnyx.com/en/articles/4425088-reporting-monthly-charges),
[Number deletion and recovery](https://support.telnyx.com/en/articles/8648864-what-happens-with-my-numbers-after-my-account-gets-abolished-for-negative-balance),
[Search and Buy Numbers](https://support.telnyx.com/en/articles/4380325-search-and-buy-numbers))

## Telnyx messaging setup and the manual-pilot catch

Telnyx's Mission Control messaging guide lists four prerequisites: Level 1
verification, a funded balance, a Messaging Profile, and a DID. Its messaging
features are described as programmatic: a webhook URL is needed to receive
inbound messages and track outbound status. ([Messaging in Mission Control](https://support.telnyx.com/en/articles/8219294-messaging-in-mission-control))

The current pay-as-you-go Messaging pricing page lists SMS at $0.004 per
message part plus carrier fee for both outbound and inbound traffic. It lists
MMS at $0.015 outbound and $0.005 inbound, each plus carrier fee. Pricing is
per message part, so long or non-GSM messages can cost more than one segment.
These are Telnyx SMS/MMS costs; Signal conversations themselves do not traverse
Telnyx and therefore are not priced as Telnyx SMS. ([Telnyx Messaging pricing](https://telnyx.com/pricing/messaging))

This matters for Signal registration. The OTP will arrive as an inbound SMS
to the Telnyx DID; the reviewed Telnyx documentation does not promise that an
operator can read that SMS in a normal consumer inbox. A tiny temporary
operator-owned webhook/receiver (or an approved existing internal receiver)
may be needed just to inspect the registration code. That is a pilot
operations tool, not product integration, but it means the process is not
strictly zero-configuration.

For ordinary US outbound SMS from a local +1 long code, Telnyx says 10DLC is
the compliance framework for business/A2P traffic and that unregistered 10DLC
traffic has been blocked since February 3, 2025. Registration requires a
Brand, Campaign, number assignment, and manual review; Telnyx lists a $4.50
Brand fee, $15 Campaign Review fee, and campaign MRCs (for example, $1.50 for
low-volume mixed or $10 for several standard use cases), with campaign fees
initially billed for three months. ([10DLC FAQ](https://support.telnyx.com/en/articles/3679260-frequently-asked-questions-about-10dlc),
[10DLC fees](https://support.telnyx.com/en/articles/5634625-10dlc-fees-and-charges))

Signal's chat traffic is internet traffic, not SMS. Therefore, a pilot that
only receives Signal's registration SMS should not be assumed to need an
outbound 10DLC campaign; confirm the exact use case with Telnyx. If the same
number later sends SMS to US users, budget and register it as 10DLC rather
than trying to classify business traffic as P2P.

## Signal-specific feasibility and custody

Signal's supported registration flow is:

1. Install Signal on Android or iOS.
2. Enter the Telnyx number.
3. Receive the registration code by SMS, or request a verification call after
   the timer expires.
4. Enter the code in the Signal app.

Signal says not to share the verification code. The number must remain under
Finite's control; Signal's safety guidance recommends maintaining ownership of
the registered number. ([Register a phone number](https://support.signal.org/hc/en-us/articles/360007318691-Register-a-phone-number),
[Registration troubleshooting](https://support.signal.org/hc/en-us/articles/360007318751-Registration-troubleshooting),
[How to protect yourself on Signal](https://support.signal.org/hc/en-us/articles/9932632052378-How-to-protect-yourself-on-Signal))

The official docs do not say whether every Telnyx VoIP/DID is accepted by
Signal. They specify only SMS/call reachability. Treat acceptance as an
unverified compatibility question and run one test number before buying a
batch. Keep both SMS and voice capability if the selected DID supports both,
because Signal offers the call fallback.

Signal Desktop is a linked-device client, not an independent phone-number
registration. Signal allows up to five linked devices per account; the
primary phone must come online at least every 30 days, and linked devices can
be unlinked after 45 days of inactivity. ([Linked Devices](https://support.signal.org/hc/en-us/articles/360007320551-Linked-Devices))

Signal states that message history is stored on users' devices and Signal does
not have a copy of conversation history. This is important for a bot pilot:
the primary phone and linked desktop are operational state and need controlled
custody, backups/transfer procedures, and an owner. A Telnyx invoice or DID
inventory is not a Signal conversation backup. ([Registration troubleshooting](https://support.signal.org/hc/en-us/articles/360007318751-Registration-troubleshooting))

## A bounded first experiment

The lowest-commitment sequence is:

1. In Austin's existing Telnyx account, record the account framework/status,
   current balance, payment method custody, billing address country, and any
   existing DIDs. Do not create a second account until this inventory is
   complete.
2. Confirm that the account owner and payment method are company-controlled;
   enable account notifications and a conservative recharge/balance alert.
3. Fund only enough for one number plus a small usage buffer, remembering that
   Telnyx's documented minimum payment is $10 and that the first-day cap may
   apply.
4. Search one US local DID with SMS (and voice fallback if available), record
   its portal-displayed upfront/MRC, and assign a temporary Messaging Profile
   and inbound webhook.
5. Register that number on a dedicated, company-controlled Android or iOS
   device. Set a Signal PIN and Registration Lock, and store the number/device
   custody record without storing the registration OTP.
6. Link one operator desktop, test a few internal conversations, and measure
   whether registration, inbound delivery, device persistence, and
   re-registration work.
7. Only after the first number works, repeat the purchase in small batches.
   Keep a ledger of DID, Signal primary device, linked devices, owner,
   Messaging Profile, purchase date, MRC, and deletion/renewal decision.

## Facts still requiring portal/provider confirmation

- Whether Austin's existing account is on legacy Level 1/2 or TPVE, and what
  its current limits are.
- The exact Austin-area DID inventory, upfront cost, MRC, SMS/voice features,
  and whether Telnyx's number can receive Signal's OTP/call.
- Whether Telnyx exposes inbound SMS for this account in a portal view or
  requires the temporary webhook described above.
- Signal's current treatment of the selected Telnyx VoIP/DID.
- Taxes, carrier fees, and any account-specific payment review or spending
  limits.
