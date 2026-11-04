# Changelog

## 0.2.1

Rebuilt with Ferrum-Frontend 80ff179: a field's label names its control for assistive technology, and a record whose title is a link shows that record's title, not its id.

## 0.2.0

- Campaigns has an interface of its own at /campaigns/. Each campaign is one page, with its tab in the address:
  Setup (name, sender, subject, the line an inbox shows, who it goes to, A/B tests), Content, Review and
  Results. A draft saves by itself a moment after the last change, against the version on screen; if someone
  else changed the same thing, nothing is overwritten without asking.
- Write the email in HTML with what is needed to hand (details that change with each person, a few blocks that
  mail programs understand, a Mail template to start from) and see it as the server builds it, as it looks on
  a computer or a phone, as a sample person or a real contact, and for each version of an A/B test. Send a test
  to up to ten addresses from the same page.
- Before anything goes out, Review says how many people will get it and why the others are left out: those who
  unsubscribed, bounced, complained, are on the suppression list, have not confirmed, or are left out by your
  rules, counted the way sending counts. It also says what would stop the send (no Public URL, a sender that
  cannot send, an email that does not build) and what deserves a look (no postal address, a long subject,
  pictures without alternative text, an email Gmail would cut off). The count is taken again at the last
  moment, and you confirm the number you will actually reach.
- Results show the headline numbers with what each is a share of, and charts over time, hour by hour or day by
  day: emails sent, opens and clicks, unsubscribes and bounces. An A/B test shows each version's rate as the send
  went on, and when the winner was chosen. Links show their share of the emails that arrived. A sending
  campaign refreshes every minute. Each chart can be read as a table.
- Audiences: contacts by standing (subscribed, waiting to confirm, unsubscribed, bounced, blocked), how many
  joined and left in 30 days, and a segment builder whose count and sample of people follow every change
  without saving (with who would not be emailed, and why) and which says where a segment is used. Contacts can
  be imported from a CSV file: choose the columns, say what the people agreed to (the basis is kept on each
  contact), check who is new and who is blocked, then import in batches that can be stopped. An opt-out list
  unsubscribes the people on it, including those you already have.
- Campaigns and segments open on these pages from anywhere, and a campaign can be duplicated. The rest of the
  records stay in the records screens.
- New for other screens and agents: the campaign actions `audience_check` (who would get it, and what is
  missing) and `timeline` (events by hour or day), and the segment action `count` (counts rules without saving
  them). `preview` no longer locks the campaign or leaves an entry in its history, and is not a button on the
  records form any more (the campaign page shows it); Send Test Email there asks for the addresses.
- Needs Ferrum 0.3.2.

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
