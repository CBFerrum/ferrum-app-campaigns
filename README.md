<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/banner-dark.svg">
  <img alt="Ferrum Campaigns: email marketing from first draft to revenue" src="docs/assets/banner-light.svg" width="100%">
</picture>

<br>

[![Version](https://img.shields.io/badge/version-0.5.2-a63089?style=flat-square)](CHANGELOG.md)
[![Licence: AGPL-3.0](https://img.shields.io/badge/licence-AGPL--3.0-blue?style=flat-square)](LICENSE)
[![Ferrum platform](https://img.shields.io/badge/runs%20on-Ferrum-2f5bea?style=flat-square)](https://github.com/CBFerrum/Ferrum)
![Rust](https://img.shields.io/badge/Rust-server-b7410e?style=flat-square&logo=rust)
![PostgreSQL](https://img.shields.io/badge/PostgreSQL-data-336791?style=flat-square&logo=postgresql&logoColor=white)
![React](https://img.shields.io/badge/React%20%2B%20TypeScript-interface-0ea5e9?style=flat-square&logo=react&logoColor=white)
![Tests](https://img.shields.io/badge/tests-27%20Rust%20%2B%20113%20UI-16a34a?style=flat-square)

### Write, check, send and measure email, text and chat-app campaigns, with consent kept honestly.

Ferrum Campaigns is for marketing teams that want a newsletter and automation tool that lives next to their
customers, orders and support tickets. It replaces a hosted email service that needs its contacts copied in
and its revenue figures copied out.

[💡 Why](#-why-ferrum-campaigns) · [📸 Screenshots](#-screenshots) · [✨ Features](#-features) · [📊 Compare](#-how-it-compares) · [📍 Roadmap](#-roadmap) · [🚀 Install](#-install) · [🔧 Develop](#-develop)

</div>

## 💡 Why Ferrum Campaigns

- 🧬 **Your audience is your data.** Segments can read any record in the workspace: a count of orders, the latest deal date, a saved query. No list uploads, no sync jobs.
- 💶 **Revenue counted from real orders.** A submitted sales order (or invoice) is credited to the last click or open of the same person within a window, once per order.
- ✅ **One record of consent for every app.** An unsubscribe here counts in every other Ferrum app, and an opt-out recorded elsewhere counts here. Every send asks the platform before it mails.
- 🛡️ **Never sent twice, never sent to the suppressed.** Each recipient is claimed before the mail is handed over; a suppression list and the platform's marketing decision are checked on every send.
- 🏠 **Self-hosted and open source.** AGPL-3.0, no per-contact or per-send fee from Ferrum. You pay only your own mail provider.
- 🛠️ **Extendable by configuration.** Add fields, rules and reports in Ferrum Studio; reach every record over REST and MCP.
- 🌍 **English, Arabic (right to left) and Hindi.**

## 🎯 At a glance

| | |
|---|---|
| **Version** | 0.5.2 (package `campaigns` 0.1.0, `campaigns_crm` 0.1.0) |
| **Kind** | Its own interface at `/campaigns/` (campaigns, results, audiences, imports) plus Ferrum's standard lists and forms for every other record |
| **What it adds** | 18 entities (13 records and 5 line tables), 2 roles, 6 reports, 1 menu with 19 entries, 2 create shortcuts, 1 assistant guide |
| **Depends on** | Ferrum platform `>=0.3.2, <0.4`; the Mail app (comms `>=0.13.1, <0.21`) |
| **Works with** | Mail (sending, email groups, templates, DNS check), CRM (campaign link and attribution), ERP (sales orders and invoices for revenue), any app whose records start a journey |
| **Languages** | English, Arabic, Hindi |
| **Tests** | 27 Rust tests against a real PostgreSQL, 113 interface tests |
| **Licence** | AGPL-3.0-only |

## 📸 Screenshots

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/campaigns-audiences-dark.webp">
  <img alt="Audiences" src="docs/assets/screens/campaigns-audiences-light.webp" width="100%">
</picture>

**📣 Campaigns** — See email campaigns from first draft to results.

<table>
<tr>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/campaigns-campaigns-dark.webp"><img alt="Campaigns" src="docs/assets/screens/campaigns-campaigns-light.webp"></picture>
<p><b>🎯 Audiences</b><br><sub>Manage the contacts and segments that campaigns are sent to.</sub></p>
</td>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/campaigns-new-campaign-dark.webp"><img alt="New campaign" src="docs/assets/screens/campaigns-new-campaign-light.webp"></picture>
<p><b>✍️ New campaign</b><br><sub>Start an email campaign by choosing an audience and writing the message.</sub></p>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/campaigns-table-dark.webp"><img alt="Campaigns as a table" src="docs/assets/screens/campaigns-table-light.webp"></picture>
<p><b>🗂️ Campaigns as a table</b><br><sub>See campaigns as a table to sort and filter them.</sub></p>
</td>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/campaigns-marketing-contact-list-dark.webp"><img alt="Marketing contacts" src="docs/assets/screens/campaigns-marketing-contact-list-light.webp"></picture>
<p><b>👥 Marketing contacts</b><br><sub>Browse, search and filter marketing contacts with their consent status.</sub></p>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/campaigns-marketing-segment-list-dark.webp"><img alt="Marketing segments" src="docs/assets/screens/campaigns-marketing-segment-list-light.webp"></picture>
<p><b>🧩 Marketing segments</b><br><sub>Browse segments and the number of people each reaches.</sub></p>
</td>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/campaigns-report-email-campaign-performance-dark.webp"><img alt="Email campaign performance" src="docs/assets/screens/campaigns-report-email-campaign-performance-light.webp"></picture>
<p><b>📊 Email campaign performance</b><br><sub>Run the campaign performance report and read the result.</sub></p>
</td>
</tr>
</table>

On a phone:

<table>
<tr>
<td width="50%" valign="top" align="center">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/campaigns-campaigns-phone-dark.webp"><img alt="Campaigns" src="docs/assets/screens/campaigns-campaigns-phone-light.webp" width="260"></picture>
<p><b>📣 Campaigns</b><br><sub>See email campaigns from first draft to results.</sub></p>
</td>
<td width="50%" valign="top" align="center">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/campaigns-marketing-contact-list-phone-dark.webp"><img alt="Marketing contacts" src="docs/assets/screens/campaigns-marketing-contact-list-phone-light.webp" width="260"></picture>
<p><b>👥 Marketing contacts</b><br><sub>Browse, search and filter marketing contacts with their consent status.</sub></p>
</td>
</tr>
</table>

> [!TIP]
> The screenshots follow your GitHub theme: switch between light and dark to see both.


## ✨ Features

### 👥 Contacts and consent

- **One marketing contact per address.** Addresses are stored in lower case, so `A@X.test` and `a@x.test` are one person. A contact holds name, phone, an attribute bag (city, plan, anything), an optional link to the record it came from, and the soft-bounce count.
- **Consent with evidence.** Status is Subscribed, Pending, Unsubscribed, Bounced or Complained. The time, the source (Manual, Import, Email Group, Customer, Contact, Lead, Subscription Form, API) and a keyed hash of the IP address are kept. Only the person can opt back in, through a signed link.
- **One consent record for the platform.** An unsubscribe or complaint records the withdrawal with the platform; a confirmed signup records consent. A list imported as subscribed records nothing, so it cannot override a withdrawal heard elsewhere.
- **A suppression list.** Unsubscribes, hard bounces, repeated soft bounces, complaints and manual blocks are never mailed, whatever the contact's status. The soft-bounce limit is a setting (default 3 in a row; a human open or click resets the count).
- **Personal-data requests.** A request exports and erases a contact's email, phone, names and consent evidence. The suppression list keeps the address so an erased person is never mailed again.
- **Topics and a preference page.** A topic is a kind of mail (a newsletter, offers, product news), promotional or operational, in by default or not. A contact's own choice for a topic is kept with when, how and where. The signed unsubscribe link opens the person's page: topics to tick and "stop everything", saved without a sign-in. Operational mail reaches people whatever their marketing choice until they untick that topic.
- **Stop everything is the platform's marketing opt-out of the address,** one fact every app reads.
- **Re-subscribing can ask for confirmation.** A setting (off by default) sends a confirmation email when someone who left chooses to subscribe again, because a forwarded link must not re-subscribe someone else. At most one such email in ten minutes.

### 🎯 Audiences and segments

- **Segments by rules.** Match All or Any of: contact fields, attributes, email group membership, engagement (opened, clicked, was sent in the last N days), a field of the record the contact came from, or a count, sum or latest date of records that name the person (in a period, with a condition).
- **Segments from a saved query.** A query whose column names the person works as a segment.
- **Live counts and history.** Counts are worked out when the segment is used and on a schedule; entries and exits are recorded.
- **Audience imports.** Add contacts from a Mail email group, from customers, CRM contacts or CRM leads, skipping addresses that exist. Opt-outs are never made subscribed again.
- **CSV import with column mapping.** Choose a file or paste rows, map the columns, check before importing, and download what was not imported. Each row says why it was left out (already a contact, on the suppression list, invalid address). Rows may be imported as subscribed (with a record of how you know they agreed) or as unsubscribed, on the suppression list.
- **Signup forms with double opt-in.** A public endpoint and HTML to paste. A new subscriber gets a confirmation email and becomes subscribed (and joins the email group and topic) only when they confirm. A honeypot field catches bots. Allowed attributes, confirmation text and thank-you pages are set per form.
- **Sending domains.** A domain's SPF, DKIM and DMARC are checked through Mail's DNS check; a warm-up limits a new domain's daily volume, counting journey emails too; a seed list shows where a send lands. A promotional campaign from a domain that failed its check is refused unless an administrator overrides it with a reason.

### ✍️ Writing a campaign

- **One page for the whole job.** A campaign is written, previewed, tested, checked and sent from one page, with comments and history.
- **Email, text message or chat app.** A campaign's channel is Email, Text Message or Chat App. Text and chat messages go through the platform's message templates and channel connections.
- **HTML and plain text.** Subject, preheader, HTML content and a plain-text part. A Blocks menu inserts email-safe HTML (heading, button, picture and more), and merge tags insert first name, last name, full name, email, an attribute and the unsubscribe link. The template language allows if-blocks written by hand. A missing value renders empty and values are escaped.
- **Start from a Mail template** or a message template.
- **Preview** as it will look on a computer and a phone.
- **Send a test** to yourself, and a seed send to the workspace's seed addresses to see where it lands.
- **Check who will get it, and why others are left out.** Each person is mailed or left out for one reason: unsubscribed, bounced, complained, suppressed, pending consent, or not in the campaign's topic. The counts add up.
- **Tracking parameters.** One tick adds `utm_campaign`, `utm_source` and `utm_medium` to tracked links (a link's own values are kept) and keeps them on each click.
- **Open and click tracking** with signed links. Opens by privacy proxies and mail scanners are counted apart from human opens.
- **A/B tests.** Versions of the subject and content go to a share of the audience; after a set number of hours the winner by Opens or Clicks goes to the rest.
- **Schedule.** Pick a time or send now. The time is on the workspace's clock with its zone named beside the box. Once scheduled, content and audience are fixed; cancel the send to stop it, or duplicate it to change a copy.
- **Hourly limits.** A workspace-wide limit shared with Mail's newsletters, and a limit per campaign.
- **One-click unsubscribe headers.** Every campaign and journey email carries `List-Unsubscribe` with the recipient's own signed link and `List-Unsubscribe-Post` (RFC 8058). Test sends and confirmation requests do not.
- **A postal address in every footer** the app adds, from Campaign Settings.

### 🛤️ Journeys

- **Triggers.** Joined Email Group, Entered Segment, Attribute Changed, Date Anniversary (with an offset in days), and Record Event: something that happens to a record of any entity (a form response, an order, a deal, a ticket, a booking), with a condition in the platform's expressions.
- **Steps.** Send Email, Send Message (text or chat app), Wait (minutes, hours or days; local time is kept across daylight saving), Wait For Event, Branch, Update Attribute, Add To Group, Remove From Group, Exit.
- **Branches** on Opened Last Email, Clicked Last Email, Attribute Equals, In Segment, Event Happened or Record Condition.
- **Goals and re-entry.** A goal segment or event, re-entry after a number of hours, and once per person, journey and entry rule.
- **Consent per channel.** An email-only journey needs email consent; a journey with only text or chat steps does not need email consent; a journey with both enrols whoever can be reached on either and skips the step of a channel they cannot be reached on.
- **People leave at once** when they opt out or are erased.
- **Pause, stop, take someone out.** A live journey's trigger and steps are fixed after activation; duplicate it to change them.
- **Enrolments** show the step each person is on, when it continues and how it ended.

### 📈 Results and revenue

- **Charts over time.** Hour by hour or day by day: sent, opened, clicked, unsubscribed, bounced, complained, with the A/B outcome per version.
- **Per-campaign counts.** Total recipients, sent, failed, skipped, unique and total opens, unique and total clicks, unsubscribes, bounces and complaints.
- **Per-recipient outcome.** Pending, Held, Queued, Sent, Failed, Skipped or Cancelled, with the skip reason, delivery time, open and click counts, machine opens, and bounce type.
- **Revenue.** Submitted sales orders (or invoices, a setting) are credited to the last click (or click and open, a setting) of the same person within a window in days, once per order. Totals appear on campaigns and journeys and in the Revenue From Email report by message and link.
- **Bounces and complaints.** A signed webhook takes bounce and complaint events (with replay protection and duplicate suppression); hard bounces and complaints go to the suppression list at once.
- **The contact's timeline** lists every mail sent to them.

### 📊 Reports

| Report | What it answers |
|---|---|
| Email Campaign Performance | How each campaign did |
| A/B Test Results | Which version won, with each version's rates |
| Link Clicks | Which links were clicked, and by how many |
| Journey Funnel | How many people reached each step |
| Audience Growth | How the audience grew |
| Revenue From Email | Orders and revenue by message and link |

### 🔔 Assistant and languages

- **Assistant.** The platform's assistant knows campaign states, recipient outcomes and journey states, and suggests questions such as "How did last month's campaigns do?".
- **Right to left.** Arabic mirrors the screens. Names, words and sentences are in Arabic and Hindi, and a workspace can supply its own words.
- **Times** show on the workspace's clock.

## 🧭 How it works

### Sending a campaign

```mermaid
flowchart LR
    AUD["Audience: segments, groups, exclusions"] --> CON{"Consent and suppression"}
    CON -->|ok| CL["Recipient claimed (never twice)"]
    CON -->|not ok| SK["Skipped, reason kept"]
    CL --> MAIL["Mail app (hourly limits)"]
    MAIL --> TR["Opens and clicks (signed links)"]
    MAIL --> BN["Bounces and complaints"]
    BN --> SUP["Suppression list"]
    TR --> REV["Revenue from orders"]
    classDef app fill:#a63089,color:#fff,stroke:#a63089
    class AUD,CL app
```

The audience is worked out when the send starts. Each person is checked against the suppression list and the platform's marketing decision, then claimed in the same transaction that queues the mail, so nobody is mailed twice. Mail delivers it.

### Journeys and the rest of Ferrum

```mermaid
flowchart LR
    EV["Record event in any app: order, form, deal, ticket"] --> J["Journey: wait, branch, send"]
    SEG["Segment entry"] --> J
    J --> CH["Email, text or chat app"]
    CH --> CON["Platform consent and opt-outs"]
    J --> GO["Goal reached or exit"]
    ORD["Sales order or invoice"] --> ATT["Revenue attribution"]
    ATT --> J
    classDef app fill:#a63089,color:#fff,stroke:#a63089
    class J app
```

A journey can start from a record event in any app. Every send is checked against the one consent record, and orders that follow a click are credited back to the journey.

## 🧱 Data model

| Entity | What it is | Key fields or states |
|---|---|---|
| MarketingContact | A person you may email, one per address | Consent status, evidence, attributes, source, soft-bounce count |
| EmailSuppression | Addresses never mailed | Reason (Manual, Unsubscribed, Hard Bounce, Soft Bounces, Complaint) |
| MarketingTopic, TopicSubscription | A kind of mail and a contact's choice for it | Promotional or operational, default in; status, method, when |
| MarketingSegment (with SegmentRule) | A dynamic group | Source (Rules, Query), match All or Any, counts, refresh minutes |
| AudienceImport | One-off adding of contacts | Source (Email Group, Customer, Contact, Lead), counts created, existing, unsubscribed, invalid |
| SubscriptionForm | A public double opt-in signup form | Endpoint, embed HTML, group, topic, allowed attributes, submissions, confirmed |
| SendingDomain | A domain mail is sent from | DNS check, alignment, warm-up plan |
| EmailCampaign (with CampaignAudience, CampaignVariant) | One message to an audience | Status (Draft, Scheduled, Sending, Sent, Cancelled), channel, schedule, A/B test, counts, attributed revenue |
| CampaignRecipient | One message to one contact | Status (7), skip reason, opens, clicks, bounce type |
| MarketingJourney (with JourneyStep, JourneyEventRule) | An automation | Status (Draft, Active, Paused, Stopped), trigger, steps, goal, counts |
| JourneyEnrollment | A contact's way through a journey | Step, next run, how it ended |
| CampaignSettings | One settings record | Postal address, bounce limit, link expiry, double opt-in, seed list, attribution |

### 👥 Roles

| Role | What it can do |
|---|---|
| Marketing User | Keeps contacts, segments and imports; drafts, edits, previews and tests their own campaigns; reads results, journeys and forms. A manager sends the campaign |
| Marketing Manager | Everything above for all records, plus scheduling and cancelling campaigns, journeys, signup forms, topics, consent, the suppression list, sending domains, settings and the webhook secret |

Recipients and enrolments are written by the app only: nobody, administrators included, creates or edits a recipient.

### 🔌 Scheduled and public parts

- Public endpoints: open and click tracking, unsubscribe and preference page, signup and confirmation, and the signed bounce and complaint webhook.
- Background work: sending, journey steps, segment refresh and entry and exit recording, revenue attribution.

## 🔌 Integrations

| With | How | Needs |
|---|---|---|
| Mail | Every email is an outgoing communication sent through Mail's accounts, with retries; email groups, templates and the DNS check are Mail's | Mail installed first, with its public URL and an outgoing account set |
| CRM | `campaigns_crm` links an email campaign to a CRM campaign; clicks from a lead's or a contact's address count in CRM's Campaign Attribution; audience imports read CRM contacts and leads | CRM installed |
| ERP | Sales orders or invoices are credited as revenue | ERP installed |
| Any app | A journey starts on a record event of any entity; a segment reads any entity's fields | None |
| Text and chat apps | Through the platform's message templates and channel connections; opt-outs, quiet hours and limits are the platform's | A channel connection to a gateway you configure |
| Bounce and complaint webhook | One generic signed JSON format, header `x-campaigns-signature` | Your mail provider's events mapped to it |
| REST, MCP, webhooks | Every record and action | API keys |

## 📊 How it compares

Compared with Mailchimp, HubSpot Marketing Hub, Klaviyo, Brevo and, as open-source options, Mautic and listmonk.

| Capability | Ferrum Campaigns | Mailchimp | HubSpot | Klaviyo | Brevo | Mautic | listmonk |
|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| Newsletters and campaigns | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Consent evidence and a suppression list | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Double opt-in signup forms | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Topics and a preference page | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | 🟡 |
| Segments from rules | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | 🟡 |
| Segments over other business data (orders, deals, tickets) | ✅ | 🟡 | ✅ | ✅ | 🟡 | 🟡 | 🟡 |
| A/B tests | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ |
| Drag-and-drop email builder | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | 🟡 |
| Automation journeys | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ |
| Journeys started by an event in another system | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ |
| Visual canvas for journeys | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | ➖ |
| SMS campaigns | 🟡 [1] | ✅ | 🟡 | ✅ | ✅ | 🟡 | ❌ |
| WhatsApp or chat-app campaigns | 🟡 [1] | ❌ | 🟡 | 🟡 | ✅ | ❌ | ❌ |
| Revenue attribution from orders | ✅ | ✅ | ✅ | ✅ | 🟡 | 🟡 | ❌ |
| Send-time optimisation | ❌ | ✅ | ✅ | ✅ | ✅ | ❌ | ❌ |
| Landing pages | ❌ | ✅ | ✅ | 🟡 | ✅ | ✅ | ❌ |
| Website visitor tracking | ❌ | 🟡 | ✅ | ✅ | ✅ | ✅ | ❌ |
| UTM tags added to links | ✅ | ✅ | ✅ | ✅ | ✅ | 🟡 | ❌ |
| One-click unsubscribe headers | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Sending-domain DNS check and warm-up | ✅ | 🟡 | 🟡 | 🟡 | ✅ | ❌ | ❌ |
| Bounce and complaint handling | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Provider-specific bounce adapters (SES, Postmark and so on) | ❌ | ➖ | ➖ | ➖ | ➖ | ✅ | ✅ |
| Transactional email | 🟡 [2] | 🟡 | 🟡 | ✅ | ✅ | 🟡 | ✅ |
| Built-in CRM and lead scoring | 🟡 [3] | 🟡 | ✅ | ❌ | ✅ | ✅ | ❌ |
| Frequency capping | ❌ | 🟡 | ✅ | ✅ | 🟡 | ✅ | ❌ |
| One consent record shared with accounting, support and sales | ✅ | ❌ | 🟡 | ❌ | 🟡 | ❌ | ❌ |
| Self-hosted, open source | ✅ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ |
| No per-contact or per-send fee from the vendor | ✅ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ |
| Personal-data export and erasure built in | ✅ | 🟡 | ✅ | 🟡 | 🟡 | ✅ | 🟡 |

Notes. 🟡 for a leader means a higher plan, an add-on, a partner, or only part of the feature.

1. Text and chat-app messages go through the platform's message templates and channel connections; no gateway ships with the app and none has been run against a live provider.
2. Mail sends any message; Campaigns marks only confirmations and tests as transactional and has no transactional dashboard.
3. CRM has lead scoring, and the `campaigns_crm` package credits clicks to a CRM campaign; Campaigns does not yet write opens and clicks to a lead's score.

### 🏆 Where Ferrum Campaigns stands out

- **Segments and journeys over the whole workspace.** Orders, tickets, deals and bookings are first-class, not imports.
- **Revenue from real orders** without a shop connector.
- **A consent model that is shared.** One record, read by every app, with evidence kept.
- **Open source, self-hosted, no per-contact pricing.** You keep the data.
- **Safe sending by construction.** A recipient is claimed before the mail is queued; the pre-send check shows exactly why each person is left out.

### 🚧 Where the leaders are still ahead

- **A drag-and-drop email builder and a visual journey canvas.** Campaigns offers an HTML editor with insertable blocks and a table of steps.
- **Landing pages, website tracking and pop-up forms.**
- **Send-time optimisation and delivery in each person's time zone.**
- **Ready-made e-commerce flows** (abandoned cart and the like) and shop connectors.
- **Provider-specific bounce events** (SES, Postmark, SendGrid, Mailgun) and reputation tools.
- **Frequency caps and automatic pause on high complaint rates.**

> [!NOTE]
> Leader columns are from public product information as of November 2026 and may change. Plans and add-ons differ; check each vendor.

## 📍 Roadmap

### ✅ Recently shipped

- [x] **0.5.2** — Accepts Mail 0.20.
- [x] **0.5.1** — Re-subscribing from the preference page can ask for an email confirmation; journeys need the consent of the channels their steps use; the warm-up counts journey emails.
- [x] **0.5.0** — Topics and a preference page, text and chat-app messages, journeys on record events, segments from any data, revenue attribution and sending-domain checks.
- [x] **0.4.4** — One-click unsubscribe headers on every campaign and journey email.
- [x] **0.4.0** — Unsubscribes and confirmed subscriptions go on the platform's one record of consent.

### 🔜 Next up

**Writing**
- [ ] A drag-and-drop block editor with brand colours, fonts and saved blocks. The most common first-day request.
- [ ] Pick pictures from the brand library (Ferrum Digital Assets).
- [ ] Dynamic content blocks by segment, attribute or language.

**Journeys**
- [ ] A visual canvas for journeys.
- [ ] Edit a live journey and version it.
- [ ] More steps: create a task or lead, notify a person, webhook, random split.
- [ ] Quiet hours and send windows per step.

**Sending and safety**
- [ ] Frequency caps and a sunset policy across campaigns and journeys.
- [ ] Automatic pause when bounce or complaint rates pass a threshold.
- [ ] Provider adapters for bounce and complaint events (SES, Postmark, SendGrid, Mailgun).
- [ ] Send-time optimisation and delivery in the recipient's time zone.

### 🔭 Later

- [ ] Landing pages with a builder and a custom domain.
- [ ] Website tracking: visitor identification, page-visit and form events.
- [ ] Sign-up forms with more fields, consent text, CAPTCHA and pop-ups.
- [ ] A continuing sync with CRM contacts, leads and customers, not a one-off import.
- [ ] Opens, clicks and form fills written to a lead's score in CRM.
- [ ] Spam score and inbox previews.
- [ ] A/B winner by revenue or conversion, send-time and from-name tests, holdouts.
- [ ] Recurring and digest campaigns.
- [ ] Content feeds (latest articles, products, cart contents) in an email.
- [ ] Content in the contact's language.
- [ ] Reports by device, mail client and location; click maps.
- [ ] Per-brand footer address, from-name and domain.
- [ ] Ready-made e-commerce flows and shop connectors.
- [ ] Ad audience sync (Meta and Google).
- [ ] Address validation at import (MX, disposable, role addresses).
- [ ] Web push, mobile push and in-app messages.

## 🚀 Install

Campaigns is compiled into the Ferrum server image and installed from **Apps** (`/store/`) by a workspace administrator. Self-hosters who build their own image enable the `campaigns` feature of the server. It needs Ferrum `>=0.3.2, <0.4` and the Mail app (comms `>=0.13.1, <0.21`) installed first.

After installing:

1. In Mail's **Communication Settings** set the **public URL** (campaigns cannot be scheduled until it is set, because tracking, unsubscribe and confirmation links must reach the server) and the hourly sending limit.
2. Add an outgoing **Email Account** in Mail; the default outgoing account is used when a campaign names none.
3. Open **Campaign Settings**: postal address (printed in every footer), soft-bounce limit, link expiry, double opt-in, seed list, and the revenue settings (sales order or invoice, window in days, clicks or clicks and opens). Rotate the webhook secret to get the bounce webhook's address; it is shown once.
4. Check your **Sending Domain** so SPF, DKIM and DMARC pass.
5. Give people the roles **Marketing User** and **Marketing Manager**.
6. Create topics if you want a preference page, and a first segment.

## 🔧 Develop

Clone it next to the platform:

```text
work/
├── ferrum/            https://github.com/CBFerrum/Ferrum (with Ferrum-Frontend cloned into ferrum/web)
└── ferrum-app-campaigns/
```

Point the platform dependencies at your checkout (this file is git-ignored):

```toml
# .cargo/config.toml
[patch."https://github.com/CBFerrum/Ferrum"]
ferrum-calc = { path = "../ferrum/crates/calc" }
ferrum-domain = { path = "../ferrum/crates/domain" }
ferrum-kernel = { path = "../ferrum/crates/kernel" }
ferrum-meta = { path = "../ferrum/crates/meta" }
ferrum-apps = { path = "../ferrum/crates/apps" }
```

It also builds on [ferrum-app-comms](https://github.com/CBFerrum/ferrum-app-comms), and its tests on [ferrum-app-crm](https://github.com/CBFerrum/ferrum-app-crm) and [ferrum-app-erp](https://github.com/CBFerrum/ferrum-app-erp), which Cargo takes from GitHub. To work on them alongside, clone them into the same folder and add:

```toml
[patch."https://github.com/CBFerrum/ferrum-app-comms"]
ferrum-app-comms = { path = "../ferrum-app-comms" }
[patch."https://github.com/CBFerrum/ferrum-app-crm"]
ferrum-app-crm = { path = "../ferrum-app-crm" }
[patch."https://github.com/CBFerrum/ferrum-app-erp"]
ferrum-app-erp = { path = "../ferrum-app-erp" }
```

The tests need PostgreSQL with an ordinary (non-superuser) role, so row-level security applies:

```bash
createuser ferrum --pwprompt && createdb -O ferrum ferrum_campaigns_test
FERRUM_CAMPAIGNS_TEST_DATABASE_URL=postgres://ferrum:ferrum@localhost:5432/ferrum_campaigns_test cargo test
```

The interface is in `ui/` (React, built with the packages of the sibling checkout `../ferrum/web`; nothing to install in `ui/`). The built files in `ui/dist/` are committed and embedded in the crate, so rebuild before you commit:

```bash
cd ui
npm run test       # the logic of every screen, without a browser
npm run typecheck
npm run build      # writes ui/dist and a stamp of the sources it came from
npm run check      # fails if dist/ is stale or over the size budget
```

CI (`.github/workflows/ci.yml`) runs the Rust checks on PostgreSQL and the interface checks on Node 22.

## 📂 Project layout

```text
app.yaml              the manifest: id, version, requirements, what it adds to Ferrum
packages/campaigns/   entities (contacts, segments, campaigns, journeys), roles, reports, SQL migrations
packages/campaigns_crm/  the CRM link: an email campaign belongs to a CRM campaign
src/                  Rust: sending, segments, journeys, tracking, deliverability, attribution, import
ui/                   its own screens (src/ is the code, dist/ the built release)
tests/                integration tests against a real PostgreSQL
```

## 📜 Changelog

Every version is in [CHANGELOG.md](CHANGELOG.md). The latest is **0.5.2**.

## 🤝 Contributing

Issues and pull requests are welcome at [github.com/CBFerrum/ferrum-app-campaigns](https://github.com/CBFerrum/ferrum-app-campaigns/issues). Run the checks in the Develop section before you open one. Rebuild `ui/dist/` if you change the screens.

## 📄 License

[AGPL-3.0-only](LICENSE), like the platform.

<div align="center">

<sub>Part of <a href="https://github.com/CBFerrum/Ferrum">Ferrum</a>, the open business platform · Made with 🦀 Rust and 🐘 PostgreSQL</sub>

</div>
