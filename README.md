<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/banner-dark.svg">
  <img alt="Ferrum Campaigns" src="docs/assets/banner-light.svg" width="100%">
</picture>

[![CI](https://github.com/CBFerrum/ferrum-app-campaigns/actions/workflows/ci.yml/badge.svg)](https://github.com/CBFerrum/ferrum-app-campaigns/actions/workflows/ci.yml) ![Version](https://img.shields.io/badge/version-0.1.1-a63089?style=flat-square) ![Ferrum](https://img.shields.io/badge/Ferrum-%3E%3D0.3%2C%20%3C0.4-2f5bea?style=flat-square) ![License](https://img.shields.io/badge/license-AGPL--3.0-3b82f6?style=flat-square) ![Rust](https://img.shields.io/badge/Rust-native%20app-b7410e?style=flat-square&logo=rust&logoColor=white) ![MCP](https://img.shields.io/badge/AI%20agents-MCP-8b5cf6?style=flat-square)

**A native app for [Ferrum](https://github.com/CBFerrum/Ferrum), the open-source, AI-first business platform.**

[Overview](#overview) · [Highlights](#highlights) · [Screenshots](#screenshots) · [How it works](#how-it-works) · [Data model](#data-model) · [Install](#install) · [Develop](#develop) · [Changelog](#changelog)

</div>

---

## Overview

Ferrum Campaigns runs email marketing on top of the Mail app: marketing contacts with consent
evidence, segments compiled to parameterized SQL, campaigns with A/B tests and schedules, open and
click tracking with signed links, a suppression list honoured by every send, double opt-in signup
forms, and journeys that send, wait and branch over time.


| At a glance | |
|---|---|
| 📦 Version | `0.1.1` |
| 🧱 Platform | Ferrum `>=0.3, <0.4` |
| 🔗 Requires | comms `>=0.7, <0.8` |
| 🖥️ Interface | Ferrum's records screens (`/data/campaigns/`) |
| 🗃️ Records | 14 entities in 1 package |
| 👥 Roles | 2: Marketing User, Marketing Manager |
| 📊 Reports | 5 |
| 🧪 Tests | 19 Rust tests |


## Highlights

<table>
<tr>
<td width="50%" valign="top">

### 👥 Contacts with consent
One per address, consent evidence, attributes; only the person can opt back in.

</td>
<td width="50%" valign="top">

### 🎯 Segments
Fields, attributes, groups and engagement, compiled to parameterized SQL with live counts.

</td>
</tr>
<tr>
<td width="50%" valign="top">

### 🧪 Campaigns and A/B tests
Subjects and content tested on a share; the winner goes to the rest.

</td>
<td width="50%" valign="top">

### 📈 Tracking
Signed open and click tracking; privacy proxies and scanners counted apart.

</td>
</tr>
<tr>
<td width="50%" valign="top">

### 🚫 Suppression and bounces
A suppression list and a signed bounce and complaint webhook.

</td>
<td width="50%" valign="top">

### 🛤️ Journeys
Triggers, waits in local time, branches and attribute updates.

</td>
</tr>
</table>

## Screenshots

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/campaigns-dark.webp">
  <img alt="Marketing contacts with their consent status." src="docs/assets/campaigns-light.webp" width="100%">
</picture>

<p align="center"><sub>Marketing contacts with their consent status.</sub></p>

> [!TIP]
> The screenshots follow your GitHub theme: switch between light and dark to see both.

## How it works

### Sending a campaign

```mermaid
flowchart LR
    AUD["🎯 Audience<br/><small>segments · groups − exclusions</small>"] --> SUP{{"Suppressed?"}}
    SUP -->|no| CL["🔒 Recipient claimed<br/><small>never sent twice</small>"]
    SUP -->|yes| SK["Skipped"]
    CL --> MAIL["✉️ Mail app<br/><small>hourly limits</small>"]
    MAIL --> TR["📈 Opens · clicks<br/><small>signed links</small>"]
    MAIL --> BN["↩️ Bounces · complaints → suppression"]
    classDef app fill:#a63089,color:#fff,stroke:#a63089
    class AUD,CL app
```

### Inside Ferrum

```mermaid
flowchart LR
    subgraph Browser
        UI["Records screens<br/><small>/data/campaigns/</small>"]
        BAR["Ferrum bar<br/><small>launcher · search · bell</small>"]
    end
    subgraph Server["Ferrum server"]
        API["REST · MCP · live"]
        K["Kernel<br/><small>permissions · row-level security · audit</small>"]
        APP["Campaigns<br/><small>14 entities · 2 roles</small>"]
        OB[("Outbox")]
    end
    DB[("PostgreSQL")]
    UI --> API --> K --> DB
    BAR --> API
    K <--> APP
    K --> OB --> EV["Notifications · webhooks<br/>automations"]
    APP -. requires .-> D0["comms >=0.7, <0.8"]
    classDef app fill:#a63089,color:#fff,stroke:#a63089
    class APP app
```

Everything Campaigns adds is ordinary Ferrum metadata and Rust: its records get lists, forms,
search, permissions, an audit trail, exports and the REST and MCP APIs for free, so AI agents work
with them under the same rules as people.

## Data model

The main records and how they connect (14 entities in all; `packages/` has every field).

```mermaid
erDiagram
    EmailCampaign {
        string campaign_name PK
        string status
        string subject
    }
    MarketingJourney {
        string journey_name PK
        string status
        string trigger_type
    }
    CampaignRecipient {
        string email PK
        string status
    }
    JourneyEnrollment {
        string email PK
        string status
    }
    MarketingContact {
        string email PK
        string consent_status
    }
    MarketingSegment {
        string segment_name PK
        string match_type
    }
    EmailCampaign ||--|{ CampaignAudience : "audience"
    EmailCampaign ||--|{ CampaignVariant : "variants"
    MarketingSegment ||--o{ MarketingJourney : "trigger_segment"
    MarketingSegment ||--o{ MarketingJourney : "goal_segment"
    EmailCampaign ||--o{ CampaignRecipient : "campaign"
    MarketingJourney ||--o{ CampaignRecipient : "journey"
    JourneyEnrollment ||--o{ CampaignRecipient : "enrollment"
    MarketingContact ||--o{ CampaignRecipient : "contact"
    MarketingJourney ||--o{ JourneyEnrollment : "journey"
    MarketingContact ||--o{ JourneyEnrollment : "contact"
    CampaignRecipient ||--o{ JourneyEnrollment : "last_recipient"
```

### Roles

| Role | What it is for | Entities |
|---|---|---|
| **Marketing User** | Day-to-day email marketing - keeps the audience (contacts, segments, imports), drafts and tests their own campaigns and reads the results; a manager sends them. | 13 |
| **Marketing Manager** | Runs email marketing - sends and cancels campaigns, builds journeys and signup forms, manages consent, the suppression list, settings and the bounce webhook. | 15 |

<details>
<summary><b>Reports (5)</b></summary>

- Campaign Performance
- Ab Test Results
- Link Clicks
- Journey Funnel
- Audience Growth

</details>

## Install

Workspace administrators install it from **Apps** (`/store/`) in Ferrum; it ships in the
official Ferrum server image. It needs **comms** `>=0.7, <0.8` installed first. Self-hosters building their own image enable the
`campaigns` feature of the server. Everything it adds is also available to AI agents over
Ferrum's MCP server, within each person's permissions.

## Develop

Clone it next to the platform:

```text
work/
├── ferrum/            https://github.com/CBFerrum/Ferrum
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
```

The tests need PostgreSQL with an ordinary (non-superuser) role, so row-level security
applies:

```bash
createuser ferrum --pwprompt && createdb -O ferrum ferrum_campaigns_test
cargo test
```

`FERRUM_CAMPAIGNS_TEST_DATABASE_URL` overrides the database URL.


It also builds on [ferrum-app-comms](https://github.com/CBFerrum/ferrum-app-comms), and its tests on [ferrum-app-crm](https://github.com/CBFerrum/ferrum-app-crm) and [ferrum-app-erp](https://github.com/CBFerrum/ferrum-app-erp), which Cargo takes from GitHub.
To work on them alongside this app, clone them into the same folder and add to the same file:

```toml
[patch."https://github.com/CBFerrum/ferrum-app-comms"]
ferrum-app-comms = { path = "../ferrum-app-comms" }
[patch."https://github.com/CBFerrum/ferrum-app-crm"]
ferrum-app-crm = { path = "../ferrum-app-crm" }
[patch."https://github.com/CBFerrum/ferrum-app-erp"]
ferrum-app-erp = { path = "../ferrum-app-erp" }
```


### Project layout

```text
app.yaml      the manifest: id, version, requirements, what it adds to Ferrum
packages/     metadata: entities, roles, reports, workflows, print formats
src/          the app's logic in Rust: hooks, actions, endpoints, upkeep jobs
tests/        integration tests against a real PostgreSQL
CHANGELOG.md  what changed in each version
```

## Changelog

| Version | Highlights |
|---|---|
| **0.1.1** | Installs beside today's Mail app: it required Comms 0.1, so a workspace with Mail, Chat and Calendar as they are now (Comms 0.7) could not install … |
| **0.1.0** | First release: email marketing on top of the Mail app. Marketing contacts with consent evidence (time, source, keyed IP hash) and custom … |

Every version is in [CHANGELOG.md](CHANGELOG.md).

## License

[AGPL-3.0-only](LICENSE), like the platform. Part of [Ferrum](https://github.com/CBFerrum/Ferrum) ·
[Report an issue](https://github.com/CBFerrum/ferrum-app-campaigns/issues)
