# Changelog

## 0.1.1

- Installs beside today's Mail app: it required Comms 0.1, so a workspace with Mail, Chat and
  Calendar as they are now (Comms 0.7) could not install it. It now requires Comms 0.7, the
  version its tests run against.
- Declares where it keeps people's personal data, so Administration → Personal data exports and
  erases it for a person (by email).

## 0.1.0

- First release: email marketing on top of the Mail app. Marketing contacts with consent evidence
  (time, source, keyed IP hash) and custom attributes, imported and deduplicated from Mail email
  groups, customers and CRM contacts and leads; segments on fields, attributes, group membership
  and engagement compiled to parameterized SQL; a workspace suppression list; email campaigns with
  preheaders, sandboxed templates, plain-text parts, audiences with exclusions, schedules and hourly
  limits, sent as Mail `Communication`s with every recipient claimed first (never mailed twice under
  concurrent workers); deterministic A/B tests decided by opens or clicks; signed open, click (no
  open redirect) and unsubscribe links with automatic opens counted apart; a signed bounce and
  complaint webhook; double opt-in signup forms; journeys with group, segment, attribute and
  anniversary triggers, waits in local time across daylight saving, branches, attribute and group
  steps, goals and exits; five reports and the Marketing Manager and Marketing User roles.
