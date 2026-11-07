import { t } from "@ferrum/client";
// The pieces a person drops into an email: merge tags for what changes with each person, and a few
// blocks of email-safe HTML (tables and inline styles: mail programs ignore most of the rest).
// Pure, so inserting at the caret is tested without a browser.

export interface Insertable {
  id: string;
  label: string;
  code: string;
}

/** What changes with each person. The server fills them in; a missing value renders empty. */
export const MERGE_TAGS: Insertable[] = [
  { id: "first_name", get label() { return t("First name"); }, code: '{{ contact.first_name or "there" }}' },
  { id: "last_name", get label() { return t("Last name"); }, code: "{{ contact.last_name }}" },
  { id: "full_name", get label() { return t("Full name"); }, code: '{{ contact.full_name or "there" }}' },
  { id: "email", get label() { return t("Email address"); }, code: "{{ contact.email }}" },
];

export const UNSUBSCRIBE: Insertable = { id: "unsubscribe", get label() { return t("Unsubscribe link"); }, code: '<a href="{{ unsubscribe_url }}" style="color:inherit;">Unsubscribe</a>' };

const KEY = /^[a-z][a-z0-9_]{0,39}$/;

/** An attribute key is lowercase letters, digits and _ , starting with a letter (at most 40). */
export const validKey = (key: string): boolean => KEY.test(key);

export const attributeTag = (key: string): string => `{{ contact.attributes.${key} }}`;

const FONT = "font-family:Arial,Helvetica,sans-serif;";

export const BLOCKS: Insertable[] = [
  { id: "heading", get label() { return t("Heading"); }, code: `<h1 style="margin:0 0 16px;${FONT}font-size:26px;line-height:1.25;color:#111827;">A clear headline</h1>` },
  { id: "paragraph", get label() { return t("Paragraph"); }, code: `<p style="margin:0 0 16px;${FONT}font-size:16px;line-height:1.5;color:#374151;">Say something worth reading.</p>` },
  {
    id: "button",
    get label() { return t("Button"); },
    code:
      '<table role="presentation" cellpadding="0" cellspacing="0" border="0" style="margin:8px 0 24px;"><tr><td style="border-radius:6px;background:#2f5bea;">' +
      `<a href="https://example.com" style="display:inline-block;padding:12px 22px;${FONT}font-size:16px;font-weight:bold;color:#ffffff;text-decoration:none;border-radius:6px;">Read more</a>` +
      "</td></tr></table>",
  },
  { id: "image", get label() { return t("Picture"); }, code: '<img src="https://" alt="Describe the picture" width="560" style="display:block;width:100%;max-width:560px;height:auto;border:0;margin:0 0 16px;">' },
  { id: "divider", get label() { return t("Divider"); }, code: '<hr style="border:0;border-top:1px solid #e5e7eb;margin:24px 0;">' },
];

/** A whole email to start from: a centred column on a grey page. The sender adds the unsubscribe
 *  footer and your postal address if the content has no unsubscribe link of its own. */
export const STARTER = `<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title></title></head>
<body style="margin:0;padding:0;background:#f3f4f6;">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="background:#f3f4f6;"><tr><td align="center" style="padding:24px 12px;">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="max-width:600px;background:#ffffff;border-radius:8px;"><tr><td style="padding:32px;${FONT}font-size:16px;line-height:1.5;color:#374151;">
<h1 style="margin:0 0 16px;${FONT}font-size:26px;line-height:1.25;color:#111827;">Hello {{ contact.first_name or "there" }},</h1>
<p style="margin:0 0 16px;">Tell people what is new, in a sentence or two.</p>
${BLOCKS.find((b) => b.id === "button")!.code}
<p style="margin:0;">Thanks,<br>The team</p>
</td></tr></table>
</td></tr></table>
</body>
</html>
`;

/** `text` with the selection replaced by `code`, and where the caret goes (after it). */
export function insertAt(text: string, start: number, end: number, code: string): { text: string; caret: number } {
  const a = Math.max(0, Math.min(start, text.length));
  const b = Math.max(a, Math.min(end, text.length));
  return { text: text.slice(0, a) + code + text.slice(b), caret: a + code.length };
}
