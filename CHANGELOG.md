# Changelog

## 0.4.0

One record of consent for every app (Ferrum's closure review, FEO-015).

- An unsubscribe or a complaint records the person's withdrawal of consent to marketing with the platform (Kernel::record_consent), and confirming a subscription they asked for records their consent; a list imported as subscribed records nothing, so it overrides no withdrawal heard elsewhere.
- Before a campaign or a journey hands an email to Mail, it asks the platform's one decision for marketing (consent withdrawn in any app, or processing restricted), and marks the email Marketing, so Mail asks again as it sends. The double opt-in confirmation and test sends are transactional.

## 0.3.16

The reports it defines in code (their titles and parameters) have names in Arabic and Hindi too, with its models' and manifest's (373 each in ui/src/locales/names). Translated by the model, to be reviewed by native speakers.

## 0.3.15

Rebuilt with Ferrum-Frontend c59c9a2: the names of its models, fields and manifest in Arabic and Hindi (368 each in ui/src/locales/names, which the platform serves to every page of the workspace, so a record's labels, the launcher and the bar read in the person's language), and the sentences its pages still showed in English (split around a link or a figure, built in code from a template or word by word) whole and translated (618 words each in ui/src/locales). Translated by the model, to be reviewed by native speakers.

## 0.3.14

Rebuilt with Ferrum-Frontend 155518c: the sentences its pages built in code (counts, dates, what a toast or a screen reader says) go through t() too, as whole sentences where code chose a word, and carry built-in Arabic and Hindi (560 words each in ui/src/locales, translated by the model and to be reviewed by native speakers).

## 0.3.13

The assistant knows what its records mean in its pages (contributes.shell.assistant): their states, what counts and where figures come from, with questions it suggests asking there.

## 0.3.12

Rebuilt with Ferrum-Frontend 5b22e7b: its own words carry built-in Arabic and Hindi (478 words each, ui/src/locales, translated by the model and to be reviewed by native speakers), fetched for the page's language only. The labels it keeps in objects, its dialogs, its page titles and the words it keeps in variables go through t() as well, and the names and sizes it shows as they are keep their order right to left. Its pages are drawn once their words are in, and its entry carries only the parts of the kit it uses.

## 0.3.11

Rebuilt with Ferrum-Frontend 6537d65: an action's link, code or token shown once is kept in a dialog to copy, and on a page that reads right to left, English left untranslated keeps its own order. Its own words go through the kit's t() (383 places), so a workspace's own words for its language reach them, and Admin, Interface words lists them under it (dist/words.json).

## 0.3.10

Rebuilt with Ferrum-Frontend d5d98f2: a workspace's own words for its interface (Admin, Interface words) reach its pages as they load; an attachment field that holds a picture shows it; and a value another record already has is said so, where the form said the record had been changed by someone else. Its own screens' margins, paddings, borders, alignment and insets go by the line's start and end, so they mirror right to left as the kit's do; what it places by coordinates (boards, timelines, charts, handles) stays as it was.

## 0.3.9

Rebuilt with Ferrum-Frontend 63280d3: right to left, its pages mirror as they read (icons that point along the line, margins and alignment by the line's start and end, arrow keys along it), and the records pages' words are in Arabic and Hindi.

## 0.3.8

Rebuilt with Ferrum-Frontend a64416a: a new record's box whose default is 1 shows ticked, as it is saved.

## 0.3.7

Rebuilt with Ferrum-Frontend 922fcfb: an action's dialog says every refusal, the errors on what the action made (another record's rows) among them, and an action with parameters and a confirmation asks once.

## 0.3.6

Rebuilt with Ferrum-Frontend a76295e: its pages show times on a person's own clock when they chose one on their Account page, while today and due dates stay on the workspace's day.

## 0.3.5

Sending times, results and the review are on the workspace's clock, and a scheduled send is entered on it, its zone named beside the box. Rebuilt with Ferrum-Frontend 24c3951, so its records' pages show times on the workspace's clock too, a draft numbered on submit is headed as a draft, and a record of an app that is not installed says so.

## 0.3.4

Rebuilt with Ferrum-Frontend bb60115: a record page reads an empty number or check as the server does, so an action whose condition needs one shows, and a workspace's settings open as their one record.

## 0.3.3

- Before a campaign goes out, the review's Public URL and postal address findings open the settings page that fixes them (Mail's Communication Settings, Campaign Settings): they named the page and left people to find it.
- A subscription form has **HTML for Your Website**: a form to paste into a page, posting the address, the first name and the allowed attributes to the form's full address, with the hidden field that catches bots. Its page gave only the endpoint's path.
- The campaign report is **Email Campaign Performance** (`email_campaign_performance`): it shared its name with CRM's Campaign Performance, so with CRM installed `/api/reports/campaign_performance` ran CRM's, and a sent campaign's Results tab said "No results yet".

Both found walking marketing to consent in a browser. Built with Ferrum-Frontend 0f1a544.

## 0.3.2

Requires Comms 0.10, the release it is built and tested with: it asked for Comms 0.7, so with the platform's Comms 0.10 it could not be installed (found walking marketing to consent in a browser). Rebuilt with Ferrum-Frontend 0f1a544.

## 0.3.1

Rebuilt with Ferrum-Frontend 00dea07: in a workflow state that names what may change, the other fields show read-only, and the fields the state needs show required.

## 0.3.0: campaigns that belong to CRM's

With CRM installed, the **Campaigns and CRM** package (`campaigns_crm`) gives an email campaign the
CRM campaign it is part of, and CRM's Campaign Attribution counts the clicks on it from the address of
a won deal's lead or contact, before the deal was won, toward that campaign. Marketing users may read
CRM's campaigns to choose one.

## 0.2.2

Rebuilt with Ferrum-Frontend daeadda: after an action runs, a record page shows what the action said it did rather than only that it is done.

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
