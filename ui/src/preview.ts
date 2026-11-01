// What a preview shows: the email the server built, in a page of its own, and the line an inbox
// would show for it. Pure, so it is tested without a browser.

/** What a preview frame may load: pictures and styles, nothing else. The frame is sandboxed (no
 *  scripts), and this policy is the second wall, as in the Mail app. */
const POLICY = "default-src 'none'; style-src 'unsafe-inline'; img-src https: data:; font-src https: data:; base-uri 'none'; form-action 'none'";

/** The server's email as a page for a sandboxed frame. Mail is read on white, whatever the app's
 *  theme: the page says it is a light page. Links open in a new tab. */
export function previewDoc(html: string): string {
  return `<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${POLICY}"><meta name="color-scheme" content="light"><base target="_blank">${html}`;
}

const FILLER = /(?:&#847;|&zwnj;|&nbsp;)+/g;
const HIDDEN = /<div style="display:none;font-size:1px;[^"]*">([\s\S]*?)<\/div>/i;

const unescape = (s: string) =>
  s
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&amp;/g, "&");

/** The preview text the sender hid at the top of the email, as an inbox would show it. */
export function preheaderOf(html: string): string {
  const m = HIDDEN.exec(html);
  return m ? unescape(m[1].replace(FILLER, "")).trim() : "";
}

/** A line cut to `max` characters, with an ellipsis when it was cut. */
export function clip(text: string, max: number): string {
  const t = text.trim();
  return t.length <= max ? t : `${t.slice(0, Math.max(0, max - 1)).trimEnd()}…`;
}

/** How much of a subject an inbox shows: a computer a lot, a phone less. */
export const SUBJECT_ROOM = { desktop: 70, phone: 38 } as const;
export const PREHEADER_ROOM = { desktop: 90, phone: 50 } as const;
