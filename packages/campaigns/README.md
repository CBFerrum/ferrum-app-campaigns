# campaigns — email marketing

Package `campaigns` (depends on `core` and the Mail app's `comms`): marketing contacts with consent,
segments, audience imports, email campaigns with A/B tests, journeys (automation), open and click
tracking, bounces and complaints, a suppression list and double opt-in signup forms.

Everything is operated through the generic records API (`/api/v1/<Entity>`, document actions at
`POST /api/v1/<Entity>/{id}/actions/<action>`) and MCP; the only custom endpoints are the public
ones a person's mail client or browser reaches (tracking, unsubscribe, signup, confirmation) and the
provider's bounce webhook. Times are RFC 3339 UTC with milliseconds. Errors use the usual API shape
(`{"error", "message", "fields"?}`): `not_found` (404), `forbidden` (403), `invalid` /
`validation_failed` (422), `conflict` (409), `unauthorized` (401).

**Sending.** Every email is an outgoing `Communication` of the Mail app, created in the same
transaction that claims its recipient and delivered by the Mail app after commit (its accounts,
retries with backoff and failure handling apply). Emails appear on the contact's timeline
(`reference_entity: MarketingContact`). Campaigns, journeys and the Mail app's newsletters share the
workspace's hourly limit (`CommsSettings.newsletter_hourly_limit`).

## Roles

| Role | Can |
|---|---|
| Marketing User | contacts (read, create, edit, export, import), segments (read, create, edit), their own campaigns (draft, edit, preview, send tests, delete drafts — not schedule), their own imports, adding suppressions, reading recipients, journeys, enrollments and forms; Mail email groups (read) and members (read, add, edit), templates and accounts (read) |
| Marketing Manager | everything above for every record, plus scheduling and cancelling campaigns, journeys, signup forms, deleting contacts and suppressions, Campaign Settings and the webhook secret, importing from customers. With the CRM app the role also carries the CRM's Marketing Manager permissions (leads, contacts, campaigns) — roles merge by name |

Administrators can do everything. Recipients and enrollments are written by the app only: nobody
(administrators included) creates, edits or deletes a `CampaignRecipient`, and enrollments change
only through the journey or the Take Out Of Journey action.

## Setup

1. **Communication Settings** (Mail app): `public_url` (e.g. `https://erp.example.com`) — campaigns
   cannot be scheduled (nor journeys with emails activated) until it is set, because tracking,
   unsubscribe and confirmation links must reach the server; `newsletter_hourly_limit` — the
   workspace's emails per hour for campaigns, journeys and newsletters together.
2. **Email Account** (Mail app): an outgoing account; the default outgoing account (company empty)
   is used when a campaign, journey or form names none.
3. **Campaign Settings** (`CampaignSettings`, one per workspace):

| Field | Default | Meaning |
|---|---|---|
| `postal_address` | — | printed in every footer the app adds (CAN-SPAM and similar laws require it) |
| `soft_bounce_limit` | 3 | soft bounces in a row after which the address is suppressed (a human open or click resets the count) |
| `link_expiry_days` | 365 | days an email's open, click and unsubscribe links keep working |
| `confirmation_hours` | 72 | lifetime of a double opt-in confirmation link |
| `webhook_secret` | — | **secret field** (write-only; reads return `true` when set) signing the bounce webhook |
| `signing_key` | created on first use | **secret field**, hidden: the HMAC key of every link |

Action `rotate_webhook_secret` (write) → `{secret, path, url, note}`: a new 256-bit webhook secret,
**shown once**; the previous one stops working.

## Entities

### MarketingContact

| Field | Notes |
|---|---|
| `email` | required, unique per workspace; stored lower-case (a database check enforces it), so `A@X.test` and `a@x.test` are one contact |
| `first_name`, `last_name`, `full_name` | `full_name` is derived |
| `consent_status` | `Subscribed` (default) · `Pending` (waiting for a confirmation) · `Unsubscribed` · `Bounced` · `Complained`. Only `Subscribed` contacts are mailed |
| `consent_at`, `consent_source`, `consent_ip_hash` | read-only evidence of the last consent change: when, what (`user: <email>`, `form: …`, `double opt-in: …`, `unsubscribe link: <campaign>`, `import: …`, `hard bounce: …`, `soft bounces: N in a row …`, `complaint: …`) and a keyed SHA-256 of the network address it came from (never the address) |
| `source` | `Manual` · `Import` · `Email Group` · `Customer` · `Contact` · `Lead` · `Subscription Form` · `API` |
| `attributes` | JSON object of custom values: keys `^[a-z][a-z0-9_]{0,39}$`, values text (≤ 1000 characters), numbers or true/false; dates as `YYYY-MM-DD` text; at most 100 keys, 16 KB |
| `record_entity`, `record_id` | optional link to the record the contact came from (both or neither; a person must be able to read it) |
| `soft_bounce_count` | read-only |

Consent rules for people (the engine is not limited): new contacts start as `Subscribed`, `Pending`
or `Unsubscribed`; anyone with write access may unsubscribe a contact; `Bounced` may go back to
`Subscribed` (a fixed mailbox); nobody can turn `Unsubscribed`, `Complained` or `Pending` into
`Subscribed` — only the contact's own confirmed signup does. Every change is logged (audience
growth, journey triggers) and:

- `Unsubscribed` and `Complained` add the address to the suppression list, mark it unsubscribed in
  every Mail email group (so newsletters honour it) and set the linked record's own `unsubscribed`
  flag when it has one (CRM contacts and leads);
- `Bounced` adds it to the suppression list (`Hard Bounce` or `Soft Bounces`).

### EmailSuppression (Suppression List)

`email` (unique, lower-case), `reason` (`Manual` · `Unsubscribed` · `Hard Bounce` · `Soft Bounces` ·
`Complaint`), `source` (read-only: who or what added it), `note`. Campaigns and journeys never mail a
suppressed address, whatever its consent; it is re-checked when each email is handed over. A
contact's own confirmed opt-in lifts the automatic entries of their address; `Manual` ones stay
until a manager deletes them.

### MarketingSegment + SegmentRule

`segment_name` (unique), `match_type` (`All` · `Any`), `rules`, `description`; read-only
`contact_count`, `mailable_count`, `counted_at` (from the last preview).

| `rule_type` | Fields | Operators |
|---|---|---|
| `Contact Field` | `field`: `Email`, `First Name`, `Last Name`, `Full Name`, `Source`, `Consent Status`, `Linked Entity` | `Equals`, `Not Equals` (case-insensitive), `Contains`, `Not Contains`, `Starts With`, `Ends With` (`%` and `_` match themselves), `Is Empty`, `Is Not Empty` — `value` |
| `Contact Field` | `field`: `Created On` | `Before`, `After` (`value` YYYY-MM-DD, the workspace's local date), `Within Last Days` (`value` 1–36500) |
| `Attribute` | `attribute` (key) | the text operators; `Greater Than`, `Less Than` (`value` a number; non-numeric values never match); `Before`, `After`, `Within Last Days` (values starting with YYYY-MM-DD) |
| `Email Group` | `email_group` | `Is Member`, `Is Not Member` (unsubscribed members are not members) |
| `Engagement` | `activity` (`Opened`, `Clicked`, `Was Sent`), `within_days` (1–3650), `campaign` (optional) | `Did`, `Did Not` — a human open (automatic ones do not count), a click, or a delivered email of any campaign or journey (or of that campaign) in the last N days |

Rules compile to one SQL condition from fixed tables of columns and operators; keys, values,
groups and days are bound parameters, so no input ever becomes SQL. A segment is evaluated whenever
it is used. Action `preview` (read) → `{contact_count, mailable_count, counted_at, sample: [{id,
email, full_name, consent_status, mailable}]}` (first 20 by address).

### AudienceImport

`import_name`, `source` (`Email Group` · `Customer` · `Contact` · `Lead` — the last two need the
CRM app), `email_group` (for Email Group), `filters` (for records: the records API filter format,
e.g. `{"customer_type": "Individual"}` or `[["customer_group", "=", "<id>"]]`, checked against what
the author may read — an unknown or unreadable field is `forbidden`); read-only `status` (`Draft` →
`Completed`), `last_run_at`, `created_count`, `existing_count`, `unsubscribed_count`,
`invalid_count`.

Action `run_import` (write) → the counts above. One set-based statement per run, as the user (their
read access and row scopes apply): one contact per address (case-insensitive; an opt-out wins over
another record with the same address), names from the record (customers: `customer_name`; members:
`member_name` split into first and last), `record_entity`/`record_id` linked. Addresses already known
are not changed except: they get linked when unlinked, and an opt-out on the record (an unsubscribed
CRM contact or lead) unsubscribes them. Opt-outs arrive as `Unsubscribed` and suppressed. Disabled
customers are skipped. Run it again to add new records.

### SubscriptionForm

`form_name` (unique), `enabled`, `endpoint` (read-only: the path to post to), `email_group` (confirmed
subscribers join this Mail group), `email_account`, `allowed_attributes` (comma-separated keys the
form may set; not `website`), `confirmation_subject` (default "Please confirm your subscription"),
`confirmation_content` (HTML with `{{ confirm_url }}`, `{{ contact.first_name }}`, `{{ form_name }}`;
a built-in message when empty), `thank_you_url`, `confirmed_url`; read-only `submission_count`,
`confirmed_count`.

### EmailCampaign (document, numbered `EC-YYYY-00001`)

| Field | Notes |
|---|---|
| `campaign_name`, `subject` (≤ 998), `preheader` (≤ 250) | subject and preheader are templates too |
| `email_account` | blank = the default outgoing account |
| `email_template` | a Mail `EmailTemplate`: fills a blank subject and content |
| `content` | HTML template (see Templates), ≤ 1 MB |
| `text_content` | plain-text template; blank = derived from the HTML (links kept as `text (url)`) |
| `audience` | rows of `segment` or `email_group` (one per row), `exclude` |
| `schedule_at` | blank = when submitted |
| `hourly_limit` | the campaign's own emails per hour (0 = only the workspace limit) |
| `track_opens`, `track_clicks` | default on |
| `ab_test`, `variants` (2–5 rows: `variant` A–E assigned, `subject`, optional `content`), `test_percent` (1–90, default 20), `winner_metric` (`Opens` · `Clicks`), `winner_after_hours` (1–720, default 4) | the metric needs its tracking on |
| read-only | `status` (`Draft` → `Scheduled` → `Sending` → `Sent`, or `Cancelled`), `winner_variant`, `test_sent_at`, `winner_chosen_at`, `total_recipients`, `sent_count`, `failed_count`, `skipped_count`, `unique_opens`, `total_opens`, `unique_clicks`, `total_clicks`, `unsubscribe_count`, `bounce_count`, `complaint_count`, `started_at`, `finished_at`, `last_error` |

- **Submit** (`POST /api/v1/EmailCampaign/{id}/submit`, Marketing Manager) schedules it: refused
  until the Public URL is set, when the account cannot send, when a segment's rules are invalid or
  when a variant does not render. Content, audience and variants are then fixed; its static links
  are numbered for tracking.
- When due, maintenance turns the audience into `CampaignRecipient`s in one statement: the union of
  the included segments and groups (members of included groups without a contact get one,
  `Subscribed`), minus the excluded ones, mailable contacts only, each once. With an A/B test the
  recipients are ranked by SHA-256(campaign id ‖ contact id): the first `test_percent`% (rounded up,
  at least one per variant) get the variants in turn (A, B, A, B, …), the rest are `Held`.
- Batches of up to 200 then claim `Pending` recipients (`FOR UPDATE SKIP LOCKED`, under a
  per-workspace lock that serializes the hourly budget), re-check consent and suppression (`Skipped`
  otherwise), and queue one email each — so an email exists exactly when its claim commits.
- A/B: once no test email is waiting, `test_sent_at` is set; `winner_after_hours` later the variant
  with the highest unique opens (or clicks) per delivered email wins (ties: more opens or clicks,
  then the earlier variant), and the held recipients get it.
- **Sent** once nobody is waiting (and an A/B test has its winner); counters follow for 30 days.
- **Cancel** (Marketing Manager) stops a Scheduled or Sending campaign: waiting recipients become
  `Cancelled` (emails already handed over still go). A Sent campaign cannot be cancelled or deleted.
  **Amend** a cancelled one to start over from a copy.
- Action `preview` (read) `{contact?: id, variant?: "B"}` → `{subject, html, text, variant}`
  (a stand-in "Jane Doe" without a contact; no tracking).
- Action `send_test` (write; draft or scheduled) `{emails: [..] | "a@x, b@y", contact?, variant?}` →
  `{sent, communications}`: up to 10 addresses, subject prefixed `[Test] `, untracked, sent as the
  caller.

### CampaignRecipient (read-only)

One marketing email to one contact, from a campaign (`campaign`, `variant`) or a journey (`journey`,
`journey_step`, `enrollment`): `contact`, `email`, `status` (`Pending` · `Held` (waiting for the A/B
winner) · `Queued` (handed to the Mail app) · `Sent` (accepted by the mail server) · `Failed` ·
`Skipped` (with `skip_reason`) · `Cancelled`), `communication`, `queued_at`, `sent_at`, `opened_at`
(first human open), `last_opened_at`, `open_count` (human opens), `machine_open_count`, `clicked_at`,
`last_clicked_at`, `click_count`, `unsubscribed_at`, `bounced_at`, `bounce_type` (`Hard` · `Soft`),
`complained_at`, `error`. Unique per (campaign, contact).

### MarketingJourney + JourneyStep

| Field | Notes |
|---|---|
| `journey_name` (unique), `email_account`, `track_opens`, `track_clicks` | |
| `trigger_type` | `Joined Email Group` (`trigger_group`) · `Entered Segment` (`trigger_segment`) · `Attribute Changed` (`trigger_attribute`, optional `trigger_value`: only when it changes to that value, case-insensitive) · `Date Anniversary` (`trigger_attribute` holding YYYY-MM-DD, `anniversary_offset_days` −60…60, the workspace's local date; 29 February counts on the 28th in other years; once a year) |
| `include_existing` | group and segment triggers: also enroll who is already in it at activation (otherwise only who joins later) |
| `allow_reentry` | who left the group or segment may enter again on re-joining; attribute changes may start it again after a finished run |
| `goal_segment` | contacts who enter it leave with `Goal Reached` |
| `steps` | 1–50 rows, numbered from 1 in order |
| read-only | `status` (`Draft` · `Active` · `Paused` · `Stopped`), `activated_at`, `enrolled_count`, `active_count`, `completed_count`, `goal_count`, `exited_count` |

Steps (`step_type`): `Send Email` (`subject`, `preheader`, `content`, or `email_template`), `Wait`
(`wait_amount` + `wait_unit` `Hours` or `Days`; days keep the time of day in the workspace's zone
across daylight-saving changes), `Branch` (`branch_on`: `Opened Last Email`, `Clicked Last Email`
(the last email this journey sent the contact), `Attribute Equals` (`attribute`, `value`,
case-insensitive), `In Segment` (`segment`); true → `next_step` (blank = the following step), false →
`else_step` (blank = the journey ends)), `Update Attribute` (`attribute`, `value`; blank removes it),
`Add To Group` / `Remove From Group` (`email_group`; someone who unsubscribed from the group is not
added back), `Exit`. Any step may jump with `next_step`.

Actions (write): `activate` (Draft or Paused → Active; refused without the Public URL or a sending
account when it sends email; the first activation numbers its links and records who is already in
the group or segment, or the current attribute change), `pause`, `stop` (contacts still in it leave
with `Exited`; final). Trigger and steps are fixed after the first activation.

### JourneyEnrollment (read-only)

`journey`, `contact`, `email`, `status` (`Active` · `Completed` · `Goal Reached` · `Exited`),
`current_step`, `waiting`, `next_run_at`, `entered_at`, `finished_at`, `exit_reason`,
`last_recipient`, `steps_done`. Unique per (journey, contact, trigger occurrence); at most one active
per (journey, contact). Action `exit_journey` (write, Active) takes the contact out.

## Templates

MiniJinja, the Mail app's dialect: no loader (templates cannot read files or each other), a fuel
limit, missing values render empty, `{{ … }}` HTML-escaped in HTML. Values: `contact` (also as
`doc`): `id`, `email`, `first_name`, `last_name`, `full_name`, `attributes.<key>`;
`unsubscribe_url`; `campaign.name`, `campaign.number` (campaigns) or `journey.name` (journeys);
`confirm_url`, `form_name` (confirmations). When the HTML has no `{{ unsubscribe_url }}`, a footer
with the unsubscribe link and the postal address is added; the plain-text part always carries the
unsubscribe link.

## Tracking and links

Links are `{public_url}/api/site/{workspace}/campaigns/…`, signed with HMAC-SHA256 (128-bit tags,
compared in constant time) under the workspace's `signing_key`, valid for `link_expiry_days` after
the email was queued:

- open pixel `/o/{recipient}.{tag}.gif` — appended to the HTML when `track_opens`;
- clicks `/c/{recipient}.{link}.{tag}` — every `<a href>` with a static `http(s)` URL (no template
  code in the href) is numbered per campaign or journey, at most 100; a click redirects (`302`) to
  that number's URL only, so nothing in the request can choose the destination;
- unsubscribe `/u/{recipient}.{tag}`.

An open counts as automatic (in `machine_open_count`, not in opens or engagement) when it has no
user agent or a tool's, the bare `Mozilla/5.0` of Apple Mail Privacy Protection, or arrives within
two seconds of sending. A click with no open recorded also records the open (images blocked).
Human opens and clicks reset the contact's soft-bounce count.

## Public endpoints — `/api/site/{workspace}/campaigns/...`

No sign-in; each request is authenticated by its signature or token. Unknown, tampered and expired
links all get the same `404` page.

| Method & path | Request | Response |
|---|---|---|
| `GET /o/{token}.gif` | — | always `200 image/gif` (1×1, `no-store`); counted only when valid |
| `GET /c/{token}` | — | `302 Location: <the campaign's URL>` (`no-store`, `Referrer-Policy: no-referrer`); `404` page otherwise |
| `GET /u/{token}` | — | `200` page asking to confirm (a GET never unsubscribes: link scanners only GET) |
| `POST /u/{token}` | any body (the button, or `List-Unsubscribe=One-Click`) | `200` page; the contact becomes `Unsubscribed` (evidence: the campaign, a keyed hash of the caller's address) |
| `POST /subscribe/{form}` | JSON or `application/x-www-form-urlencoded`: `email`, `first_name`, `last_name`, allowed attribute keys (or JSON `attributes: {}`), honeypot `website` (must be empty) | JSON: `202 {"status": "pending", "message"}`; browser forms: `303` to `thank_you_url` or a `200` page. The same answer whether the address is new, known, already subscribed, blocked or the honeypot was filled. `422` for an invalid address, `404` for an unknown or disabled form, `503` until the Public URL is set |
| `GET /confirm/{token}` | — | `200` page asking to confirm |
| `POST /confirm/{token}` | — | `303` to `confirmed_url` or a `200` page; the contact becomes `Subscribed` (`double opt-in: <form>`, keyed IP hash), automatic suppressions of the address are lifted, the contact joins the form's group |
| `POST /webhooks/bounces` | see below | `200 {"processed", "duplicates", "ignored"}`; `401` for a missing secret or a bad or stale signature; `422` for a malformed body or more than 1000 events |

Signup rules: a new address becomes a `Pending` contact (data from the form; an existing contact is
not changed by the unauthenticated request) and gets a confirmation email with
`/confirm/{contact}.{form}.{expiry}.{tag}`; no email goes to addresses suppressed for a complaint,
a hard bounce or by hand, to contacts who complained, to subscribed members of the form's group, or
to an address mailed a confirmation in the last 10 minutes. The server limits public requests per
network address.

### Bounce and complaint webhook

Sign each request with `X-Campaigns-Signature: t=<unix seconds>,v1=<hex HMAC-SHA256 of "<t>.<raw
body>" with the webhook secret>`; signatures older or newer than 5 minutes are refused. The body is
one event, a list, or `{"events": [...]}`; each event:

```json
{"id": "provider-event-id", "type": "hard | soft | complaint | bounce", "bounce_type": "Permanent | Transient",
 "email": "a@x.test", "message_id": "<…>", "recipient": "<CampaignRecipient id>", "campaign": "<id>", "reason": "…"}
```

The email is found by `recipient`, else by the Mail app's `message_id`, else the latest one to
`email` (in `campaign`). Events are applied once (by `id`, or a hash of the event). A hard bounce
makes a subscribed contact `Bounced` and suppresses the address; a complaint makes the contact
`Complained` and suppresses it; a soft bounce counts up and suppresses at `soft_bounce_limit`.
Addresses without a contact are suppressed for hard bounces and complaints all the same. Events of
other types are ignored. A rejection by the receiving server for an unknown mailbox (5.1.x) when
the Mail app hands an email over counts as a hard bounce too.

## Reports

| Name | Entity | Parameters | Rows |
|---|---|---|---|
| `campaign_performance` | EmailCampaign | `from_date`, `to_date` (started; default the last 90 days) or `campaign` | per campaign: recipients, sent, delivered (sent − bounces), failed, skipped, unique and total opens, open rate, automatic opens, unique and total clicks, click rate (CTR), click-to-open, unsubscribes and rate, bounces (hard, soft) and rate, complaints and rate; `totals` |
| `ab_test_results` | EmailCampaign | `campaign` | per variant: subject, recipients, delivered, unique opens and rate, unique clicks and rate, total clicks, winner; `meta`: metric, winner, test share and times |
| `link_clicks` | EmailCampaign | `campaign` or `journey` | per link: URL, unique and total clicks |
| `journey_funnel` | MarketingJourney | `journey` | per step: type, label, reached (and % of enrolled), emails sent, unique opens and clicks; `totals`: enrolled, active, completed, goal reached, exited, conversion |
| `audience_growth` | MarketingContact | `from_date`, `to_date` (default the last 30 days, at most two years) | per local day: new contacts, new subscribers (into Subscribed), unsubscribed, bounced, complained (out of Subscribed), net change; `meta`: subscribed and contacts now |

Rates are percentages with two decimals. Reports see only the campaigns the caller may read.

## Background work

Every minute (`maintenance`, one replica at a time), per workspace with the package, each step
independent of the others:

1. **Delivery** — queued recipients take the Mail app's verdict (`Sent` with its time, or `Failed`
   with the error; unknown-mailbox rejections are hard bounces); emails deleted before sending fail
   after a day.
2. **Journeys** (≤ 15 s) — enroll from triggers (group and segment members not seen before, new
   attribute changes, today's anniversaries), apply goals and exits (no longer mailable), then
   advance due enrollments (at most 500 a round, 25 steps each, emails within the hourly budget —
   otherwise retried in 5 minutes).
3. **Campaigns** (≤ 30 s) — start due campaigns, decide A/B tests, send batches within the hourly
   limits, close finished campaigns, refresh counters.
4. **Purge** — webhook deduplication keys after 30 days, contact change history after 25 months.
