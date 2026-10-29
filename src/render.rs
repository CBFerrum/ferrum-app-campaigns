//! Email content: templates in the Mail app's dialect (MiniJinja with no loader, so templates
//! cannot reach files or each other; a fuel limit against runaway loops; chainable undefined
//! values, so a missing field renders empty; HTML auto-escaping for bodies), and the HTML
//! changes a marketing email needs: tracked links, the open pixel, the preheader and the
//! unsubscribe footer.

use crate::util::html_escape;
use ferrum_kernel::{Error, Result};
use minijinja::{AutoEscape, Environment, UndefinedBehavior, Value as JValue};
use regex::{Captures, Regex};
use serde_json::Value;
use std::sync::LazyLock;

const FUEL: u64 = 200_000;
const MAX_OUTPUT: usize = 2 * 1024 * 1024;
/// Links tracked per campaign or journey at most; later ones go out untracked.
pub const MAX_LINKS: usize = 100;

fn env(html: bool) -> Environment<'static> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Chainable);
    env.set_fuel(Some(FUEL));
    env.set_recursion_limit(20);
    env.set_auto_escape_callback(move |_| if html { AutoEscape::Html } else { AutoEscape::None });
    env
}

/// The template context. Top-level `*_url` strings are links the app built (unsubscribe,
/// confirmation): they go in as they are, since entity-encoded URLs look like obfuscation to spam
/// filters. Everything else — contact data above all — is escaped.
fn context(ctx: &Value) -> JValue {
    match ctx {
        Value::Object(m) => JValue::from(
            m.iter()
                .map(|(k, v)| {
                    let v = match v {
                        Value::String(url) if k.ends_with("_url") => JValue::from_safe_string(url.clone()),
                        other => JValue::from_serialize(other),
                    };
                    (k.clone(), v)
                })
                .collect::<std::collections::BTreeMap<String, JValue>>(),
        ),
        other => JValue::from_serialize(other),
    }
}

fn render(src: &str, ctx: &Value, html: bool) -> Result<String> {
    let out = env(html).render_str(src, context(ctx)).map_err(|e| Error::invalid(format!("template: {e}")))?;
    if out.len() > MAX_OUTPUT {
        return Err(Error::invalid("the template produced more than 2 MB"));
    }
    Ok(out)
}

/// An HTML body; every `{{ value }}` is HTML-escaped.
pub fn render_html(src: &str, ctx: &Value) -> Result<String> {
    render(src, ctx, true)
}

/// A plain-text part.
pub fn render_text(src: &str, ctx: &Value) -> Result<String> {
    render(src, ctx, false)
}

/// A subject or preheader: plain text on one line.
pub fn render_line(src: &str, ctx: &Value) -> Result<String> {
    let out = render(src, ctx, false)?;
    Ok(out.split(['\r', '\n']).map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" "))
}

/// Syntax check, as a message.
pub fn check(src: &str) -> std::result::Result<(), String> {
    env(true).template_from_str(src).map(|_| ()).map_err(|e| e.to_string())
}

static HREF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?is)(<a\b[^>]*?\bhref\s*=\s*)(?:"([^"]*)"|'([^']*)')"#).expect("href pattern"));
static BODY_OPEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<body\b[^>]*>").expect("body pattern"));
static BODY_CLOSE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)</body\s*>").expect("body pattern"));

/// Decodes the character references an attribute value may contain.
pub fn html_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let decoded = rest.find(';').filter(|&end| end <= 10).and_then(|end| {
            let name = &rest[1..end];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => match name.strip_prefix('#') {
                    Some(hex) if hex.starts_with(['x', 'X']) => u32::from_str_radix(&hex[1..], 16).ok().and_then(char::from_u32),
                    Some(dec) => dec.parse::<u32>().ok().and_then(char::from_u32),
                    None => None,
                },
            };
            c.map(|c| (c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn href_value<'a>(c: &'a Captures<'_>) -> &'a str {
    c.get(2).or(c.get(3)).map(|m| m.as_str()).unwrap_or_default()
}

/// The links a template always renders the same way: `http(s)` hrefs of `<a>` tags without
/// template code, decoded, in order of appearance, each once. Only these are tracked, so the
/// click endpoint can only ever redirect to a URL written in the content.
pub fn static_links(src: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in HREF.captures_iter(src) {
        let raw = href_value(&c).trim();
        if raw.contains("{{") || raw.contains("{%") {
            continue;
        }
        let url = html_unescape(raw);
        let lower = url.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) || url.len() > 2000 || url.chars().any(char::is_whitespace) {
            continue;
        }
        if !out.contains(&url) {
            out.push(url);
            if out.len() == MAX_LINKS {
                break;
            }
        }
    }
    out
}

/// Replaces the hrefs `track` knows (by their decoded URL) with what it returns.
pub fn rewrite_links(html: &str, track: impl Fn(&str) -> Option<String>) -> String {
    HREF.replace_all(html, |c: &Captures<'_>| match track(&html_unescape(href_value(c).trim())) {
        Some(url) => format!("{}\"{}\"", &c[1], html_escape(&url)),
        None => c[0].to_string(),
    })
    .into_owned()
}

/// The preview text, hidden at the top of the body.
pub fn add_preheader(html: &str, text: &str) -> String {
    let block = format!(
        "<div style=\"display:none;font-size:1px;line-height:1px;max-height:0;max-width:0;opacity:0;overflow:hidden;mso-hide:all\">{}{}</div>",
        html_escape(text),
        "&#847;&zwnj;&nbsp;".repeat(30)
    );
    match BODY_OPEN.find(html) {
        Some(m) => format!("{}{block}{}", &html[..m.end()], &html[m.end()..]),
        None => format!("{block}{html}"),
    }
}

/// Inserts `snippet` before the closing body tag (or at the end).
pub fn append_to_body(html: &str, snippet: &str) -> String {
    match BODY_CLOSE.find_iter(html).last() {
        Some(m) => format!("{}{snippet}{}", &html[..m.start()], &html[m.start()..]),
        None => format!("{html}{snippet}"),
    }
}

/// The 1×1 open-tracking image.
pub fn pixel(url: &str) -> String {
    format!("<img src=\"{}\" width=\"1\" height=\"1\" alt=\"\" style=\"display:block;width:1px;height:1px;border:0;margin:0;padding:0\">", html_escape(url))
}

/// The unsubscribe footer (with the sender's postal address) for content without its own link.
pub fn footer(unsubscribe_url: &str, postal_address: Option<&str>) -> String {
    let address =
        postal_address.map(|a| format!("<p style=\"margin:4px 0 0\">{}</p>", a.lines().map(html_escape).collect::<Vec<_>>().join("<br>"))).unwrap_or_default();
    format!(
        "<div style=\"margin-top:32px;padding-top:16px;border-top:1px solid #d0d7de;font:12px/1.5 sans-serif;color:#6e7781\">\
         <p style=\"margin:0\">You receive this email because you subscribed to our mailing list. \
         <a href=\"{}\" style=\"color:#6e7781\">Unsubscribe</a></p>{address}</div>",
        html_escape(unsubscribe_url)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn templates_escape_and_tolerate_missing_values() {
        let ctx = json!({"contact": {"first_name": "<b>Asha</b> & co", "attributes": {"city": "Pune"}}});
        assert_eq!(
            render_html("Hi {{ contact.first_name }} from {{ contact.attributes.city }}{{ contact.attributes.nope.deeper }}!", &ctx).unwrap(),
            "Hi &lt;b&gt;Asha&lt;&#x2f;b&gt; &amp; co from Pune!"
        );
        assert_eq!(render_line("Hello\n {{ contact.first_name }}", &ctx).unwrap(), "Hello <b>Asha</b> & co");
        assert_eq!(render_text("{{ contact.first_name }}", &ctx).unwrap(), "<b>Asha</b> & co");
        assert!(check("{% for x in %}").is_err() && check("{{ contact.email | upper }}").is_ok());
        assert!(render_html("{% for i in range(100000) %}{% for j in range(100000) %}x{% endfor %}{% endfor %}", &ctx).is_err(), "fuel");
        assert!(render_html("{% include '/etc/passwd' %}", &ctx).is_err(), "no loader");
        let urls = json!({"unsubscribe_url": "https://t.test/u/1.2", "contact": {"first_name": "a/b"}, "note_url": 5});
        assert_eq!(render_html("{{ unsubscribe_url }} {{ contact.first_name }} {{ note_url }}", &urls).unwrap(), "https://t.test/u/1.2 a&#x2f;b 5");
    }

    #[test]
    fn only_static_links_are_tracked() {
        let src = r##"<a href="https://shop.test/a?x=1&amp;y=2">A</a> <A class=b HREF='https://shop.test/b'>B</A>
            <a href="{{ contact.attributes.url }}">dynamic</a> <a href="https://shop.test/?e={{ contact.email }}">mixed</a>
            <a href="mailto:x@y.test">mail</a> <a href="#top">top</a> <a href="https://shop.test/a?x=1&amp;y=2">again</a>
            <a href="javascript:alert(1)">js</a> <link href="https://cdn.test/style.css">"##;
        assert_eq!(static_links(src), vec!["https://shop.test/a?x=1&y=2".to_string(), "https://shop.test/b".to_string()]);
        let out = rewrite_links(src, |u| (u == "https://shop.test/a?x=1&y=2").then(|| "https://t.test/c/1?a=b&c=d".to_string()));
        assert_eq!(out.matches("href=\"https://t.test/c/1?a=b&amp;c=d\"").count(), 2);
        assert!(out.contains("HREF='https://shop.test/b'") && out.contains("href=\"mailto:x@y.test\""));
        assert_eq!(html_unescape("a&amp;b&#39;c&#x41;&bogus;&"), "a&b'cA&bogus;&");
    }

    #[test]
    fn body_parts_go_inside_the_body() {
        let html = "<html><BODY style=\"x\"><p>Hi</p></body></html>";
        let with = append_to_body(&add_preheader(html, "Sale <now>"), &pixel("https://t.test/o/1.gif"));
        assert!(with.starts_with("<html><BODY style=\"x\"><div style=\"display:none"));
        assert!(with.contains("Sale &lt;now&gt;"));
        assert!(with.ends_with("<img src=\"https://t.test/o/1.gif\" width=\"1\" height=\"1\" alt=\"\" style=\"display:block;width:1px;height:1px;border:0;margin:0;padding:0\"></body></html>"));
        let bare = append_to_body("<p>Hi</p>", &footer("https://t.test/u/1?x=<y>", Some("1 Main St\nPune")));
        assert!(bare.starts_with("<p>Hi</p><div") && bare.contains("href=\"https://t.test/u/1?x=&lt;y&gt;\"") && bare.contains("1 Main St<br>Pune"));
    }
}
