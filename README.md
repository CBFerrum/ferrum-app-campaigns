# Ferrum Campaigns

A native app for [Ferrum](https://github.com/CBFerrum/Ferrum), the open-source, AI-first ERP.

- Marketing contacts, one per address (case-insensitive), with consent status and evidence (time, source, keyed IP hash), custom attributes and a link to the record they came from; only the person can opt back in
- Imports from Mail email groups, customers, CRM contacts and CRM leads, deduplicated, with opt-outs carried over
- Segments on contact fields, attributes, email group membership and engagement (opened, clicked or was sent mail in the last N days), compiled to parameterized SQL, with live preview counts
- Email campaigns with preheader, sandboxed templates (or a Mail template), a plain-text part, audiences of segments and email groups with exclusions, schedules and per-campaign hourly limits within the workspace's limit
- A/B tests of subjects and content on a test share, the winner by opens or clicks after N hours sent to the rest; the split is a deterministic hash
- Every recipient claimed before its email goes to the Mail app, so concurrent workers never send twice
- Open and click tracking with signed links (automatic opens from privacy proxies and scanners counted apart, no open redirect), signed unsubscribe pages and one-click POST
- A suppression list honoured by every send; a signed bounce and complaint webhook (hard bounces and complaints suppress at once, soft bounces at a limit)
- Double opt-in signup forms with a honeypot and signed, expiring confirmation links
- Journeys: triggers (joined an email group, entered a segment, attribute changed, date anniversary), steps (send, wait in local time across DST, branch on opens, clicks, attributes or segments, update attributes, join or leave groups, exit), goals and exits on unsubscribe
- Reports: campaign performance, A/B results, link clicks, journey funnel, audience growth; Marketing Manager and Marketing User roles
- `packages/campaigns/README.md` documents every entity, action, endpoint, setting and background job

## Install

Workspace administrators install it from **Apps** in Ferrum; it ships in the official Ferrum
server image and needs the Mail app (`comms`), which it installs with it. Self-hosters building
their own image enable the `campaigns` feature of the server.
Everything it adds is also available to AI agents over Ferrum's MCP server.

## Develop

Clone it next to the platform and the Mail app (and the CRM app, which the tests use) and point
the dependencies at your checkouts (this file is git-ignored):

```toml
# .cargo/config.toml
[patch."https://github.com/CBFerrum/Ferrum"]
ferrum-kernel = { path = "../ferrum/crates/kernel" }
ferrum-meta = { path = "../ferrum/crates/meta" }
ferrum-domain = { path = "../ferrum/crates/domain" }
ferrum-calc = { path = "../ferrum/crates/calc" }

[patch."https://github.com/CBFerrum/ferrum-app-comms"]
ferrum-app-comms = { path = "../ferrum-app-comms" }

[patch."https://github.com/CBFerrum/ferrum-app-crm"]
ferrum-app-crm = { path = "../ferrum-app-crm" }
```

The tests need PostgreSQL with an ordinary (non-superuser) role — row-level security must apply:

```bash
createuser ferrum --pwprompt && createdb -O ferrum ferrum_campaigns_test
```

```bash
cargo test
```

`FERRUM_CAMPAIGNS_TEST_DATABASE_URL` overrides the database URL. The app's metadata lives in `packages/` and is embedded
at build time; `app.yaml` is its manifest (see Ferrum's `docs/plan/apps.md`).

## License

AGPL-3.0-only, like the platform.
