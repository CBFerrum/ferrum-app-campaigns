//! An email's design as a tree of rows, columns and blocks, turned into MJML and rendered to responsive HTML. The tree is
//! the contract: it is checked strictly (unknown keys, types and values) before any of it is written into MJML, so a
//! design cannot carry markup, an address of another kind, or CSS. Pure: the same tree gives the same HTML; no loader is
//! given to the renderer, so `mj-include` reads nothing.

use serde::Deserialize;
use serde_json::Value;

const MAX_DESIGN_BYTES: usize = 256 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Design {
    version: u32,
    #[serde(default)]
    page: Page,
    rows: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, default)]
struct Page {
    width: u32,
    background: String,
    content_background: String,
    font: String,
    text_color: String,
    link_color: String,
}

impl Default for Page {
    fn default() -> Page {
        Page {
            width: 600,
            background: "#f3f4f6".into(),
            content_background: "#ffffff".into(),
            font: "sans".into(),
            text_color: "#374151".into(),
            link_color: "#2f5bea".into(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    #[serde(default)]
    background: Option<String>,
    #[serde(default = "default_padding")]
    padding: u32,
    columns: Vec<Value>,
}

fn default_padding() -> u32 {
    24
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Column {
    blocks: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Block {
    Heading {
        text: String,
        #[serde(default = "one")]
        level: u8,
        #[serde(default)]
        align: Align,
        #[serde(default)]
        color: Option<String>,
    },
    Text {
        html: String,
        #[serde(default)]
        align: Align,
    },
    Button {
        label: String,
        href: String,
        #[serde(default)]
        background: Option<String>,
        #[serde(default)]
        color: Option<String>,
        #[serde(default)]
        align: Align,
    },
    Image {
        src: String,
        #[serde(default)]
        alt: String,
        #[serde(default)]
        href: Option<String>,
        #[serde(default)]
        width: Option<u32>,
    },
    Divider {
        #[serde(default)]
        color: Option<String>,
    },
    Spacer {
        height: u32,
    },
}

fn one() -> u8 {
    1
}

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Align {
    #[default]
    Left,
    Center,
    Right,
}

impl Align {
    fn word(self) -> &'static str {
        match self {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
        }
    }
}

fn colour_ok(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|ch| ch.is_ascii_hexdigit())
}

/// Characters an address or image source may not hold: they would end the attribute or open markup.
fn address_chars_ok(v: &str) -> bool {
    !v.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | '\'' | '<' | '>' | '`' | '\\'))
}

/// A web address, a `mailto:` or `tel:` address, or exactly one template value such as `{{ unsubscribe_url }}`.
fn href_ok(h: &str) -> bool {
    if h.len() > 2000 {
        return false;
    }
    if let Some(inner) = h.strip_prefix("{{").and_then(|r| r.strip_suffix("}}")) {
        let name = inner.strip_prefix(' ').unwrap_or(inner);
        let name = name.strip_suffix(' ').unwrap_or(name);
        let mut chars = name.chars();
        return chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
    }
    ["https://", "http://", "mailto:", "tel:"].iter().any(|p| h.starts_with(p)) && address_chars_ok(h)
}

fn text_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn attr_escape(s: &str) -> String {
    text_escape(s).replace('"', "&quot;")
}

fn font_stack(font: &str) -> Option<&'static str> {
    Some(match font {
        "sans" => "Helvetica, Arial, sans-serif",
        "serif" => "Georgia, 'Times New Roman', serif",
        "mono" => "'Courier New', Courier, monospace",
        _ => return None,
    })
}

fn colour(c: &Option<String>, fallback: &str, what: &str) -> Result<String, String> {
    match c {
        Some(c) if colour_ok(c) => Ok(c.clone()),
        Some(_) => Err(format!("{what} must be a colour like #2f5bea")),
        None => Ok(fallback.to_string()),
    }
}

fn parse<T: for<'de> Deserialize<'de>>(v: &Value, at: &str) -> Result<T, String> {
    serde_json::from_value(v.clone()).map_err(|e| format!("{at}{e}"))
}

/// The MJML a design is written as, after every rule of the tree is checked.
pub fn mjml(design: &Value) -> Result<String, String> {
    if serde_json::to_vec(design).map_or(0, |b| b.len()) > MAX_DESIGN_BYTES {
        return Err("the design is limited to 256 KB".into());
    }
    let d: Design = parse(design, "")?;
    if d.version != 1 {
        return Err("the design's version must be 1".into());
    }
    let p = &d.page;
    if !(320..=800).contains(&p.width) {
        return Err("the page width must be 320 to 800".into());
    }
    let stack = font_stack(&p.font).ok_or("the font must be sans, serif or mono")?;
    for (what, c) in [
        ("the page background", &p.background),
        ("the content background", &p.content_background),
        ("the text colour", &p.text_color),
        ("the link colour", &p.link_color),
    ] {
        if !colour_ok(c) {
            return Err(format!("{what} must be a colour like #2f5bea"));
        }
    }
    if d.rows.is_empty() || d.rows.len() > 50 {
        return Err("a design has 1 to 50 rows".into());
    }
    let mut blocks_total = 0;
    let mut body = String::new();
    for (r, row) in d.rows.iter().enumerate() {
        let at = format!("row {}: ", r + 1);
        let row: Row = parse(row, &at)?;
        if row.padding > 64 {
            return Err(format!("{at}the padding is 0 to 64"));
        }
        if row.columns.is_empty() || row.columns.len() > 3 {
            return Err(format!("{at}a row has 1 to 3 columns"));
        }
        let background = colour(&row.background, &p.content_background, &format!("{at}the background"))?;
        body.push_str(&format!("<mj-section background-color=\"{background}\" padding=\"{}px\">", row.padding));
        for (c, column) in row.columns.iter().enumerate() {
            let at = format!("row {}, column {}: ", r + 1, c + 1);
            let column: Column = parse(column, &at)?;
            if column.blocks.len() > 20 {
                return Err(format!("{at}a column has at most 20 blocks"));
            }
            body.push_str("<mj-column>");
            for (b, block) in column.blocks.iter().enumerate() {
                let at = format!("row {}, column {}, block {}: ", r + 1, c + 1, b + 1);
                let block: Block = parse(block, &at)?;
                blocks_total += 1;
                body.push_str(&write_block(block, p, &at)?);
            }
            body.push_str("</mj-column>");
        }
        body.push_str("</mj-section>");
    }
    if blocks_total == 0 {
        return Err("add at least one block".into());
    }
    Ok(format!(
        "<mjml><mj-head><mj-attributes><mj-all font-family=\"{}\" /><mj-text color=\"{}\" font-size=\"16px\" line-height=\"1.5\" /></mj-attributes>\
         <mj-style>a {{ color: {}; }}</mj-style></mj-head><mj-body width=\"{}px\" background-color=\"{}\">{body}</mj-body></mjml>",
        attr_escape(stack),
        p.text_color,
        p.link_color,
        p.width,
        p.background
    ))
}

fn write_block(block: Block, p: &Page, at: &str) -> Result<String, String> {
    Ok(match block {
        Block::Heading { text, level, align, color } => {
            if text.is_empty() || text.chars().count() > 500 {
                return Err(format!("{at}a heading has 1 to 500 characters"));
            }
            let size = match level {
                1 => "26px",
                2 => "22px",
                3 => "18px",
                _ => return Err(format!("{at}a heading's level is 1 to 3")),
            };
            let color = colour(&color, &p.text_color, &format!("{at}the colour"))?;
            format!("<mj-text font-size=\"{size}\" font-weight=\"bold\" align=\"{}\" color=\"{color}\">{}</mj-text>", align.word(), text_escape(&text))
        }
        Block::Text { html, align } => {
            if html.chars().count() > 20_000 {
                return Err(format!("{at}a text block holds at most 20,000 characters"));
            }
            if html.to_ascii_lowercase().contains("<mj-") || html.to_ascii_lowercase().contains("</mj-") {
                return Err(format!("{at}a text block cannot hold MJML"));
            }
            format!("<mj-text align=\"{}\">{html}</mj-text>", align.word())
        }
        Block::Button { label, href, background, color, align } => {
            if label.is_empty() || label.chars().count() > 200 {
                return Err(format!("{at}a button's label has 1 to 200 characters"));
            }
            if !href_ok(&href) {
                return Err(format!("{at}the address must be https://, http://, mailto:, tel: or one template value such as {{{{ unsubscribe_url }}}}"));
            }
            let background = colour(&background, &p.link_color, &format!("{at}the background"))?;
            let color = colour(&color, "#ffffff", &format!("{at}the colour"))?;
            format!(
                "<mj-button href=\"{}\" background-color=\"{background}\" color=\"{color}\" align=\"{}\" border-radius=\"6px\">{}</mj-button>",
                attr_escape(&href),
                align.word(),
                text_escape(&label)
            )
        }
        Block::Image { src, alt, href, width } => {
            if !src.starts_with("https://") || src.len() > 2000 || !address_chars_ok(&src) {
                return Err(format!("{at}the picture's address must be https:// without spaces or quotes"));
            }
            if alt.chars().count() > 500 {
                return Err(format!("{at}the alternative text holds at most 500 characters"));
            }
            if let Some(h) = &href
                && !href_ok(h)
            {
                return Err(format!("{at}the picture's link must be https://, http://, mailto:, tel: or one template value"));
            }
            if let Some(w) = width
                && !(16..=p.width).contains(&w)
            {
                return Err(format!("{at}the picture's width is 16 to the page's width"));
            }
            let mut out = format!("<mj-image src=\"{}\" alt=\"{}\"", attr_escape(&src), attr_escape(&alt));
            if let Some(h) = href {
                out.push_str(&format!(" href=\"{}\"", attr_escape(&h)));
            }
            if let Some(w) = width {
                out.push_str(&format!(" width=\"{w}px\""));
            }
            out.push_str(" />");
            out
        }
        Block::Divider { color } => {
            let color = colour(&color, "#e5e7eb", &format!("{at}the colour"))?;
            format!("<mj-divider border-width=\"1px\" border-color=\"{color}\" />")
        }
        Block::Spacer { height } => {
            if !(4..=120).contains(&height) {
                return Err(format!("{at}a spacer's height is 4 to 120"));
            }
            format!("<mj-spacer height=\"{height}px\" />")
        }
    })
}

/// The responsive HTML a design renders to.
pub fn html(design: &Value) -> Result<String, String> {
    let m = mjml(design)?;
    let root = mrml::parse(&m).map_err(|e| format!("the design could not be rendered: {e}"))?;
    root.element.render(&mrml::prelude::render::RenderOptions::default()).map_err(|e| format!("the design could not be rendered: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tree() -> Value {
        json!({"version": 1,
               "page": {"width": 600, "background": "#f3f4f6", "content_background": "#ffffff", "font": "sans", "text_color": "#374151", "link_color": "#2f5bea"},
               "rows": [{"background": null, "padding": 24, "columns": [{"blocks": [
                   {"type": "heading", "text": "Hello {{ contact.first_name or \"there\" }},", "level": 1, "align": "left", "color": "#111827"},
                   {"type": "text", "html": "<p>Tell people what is new.</p>", "align": "left"},
                   {"type": "button", "label": "Read more", "href": "https://example.com", "background": "#2f5bea", "color": "#ffffff", "align": "left"},
                   {"type": "image", "src": "https://example.com/a.png", "alt": "A red kettle", "href": null, "width": null},
                   {"type": "divider", "color": "#e5e7eb"},
                   {"type": "spacer", "height": 24}]}]}]})
    }

    fn with_block(block: Value) -> Value {
        let mut t = tree();
        t["rows"][0]["columns"][0]["blocks"] = json!([block]);
        t
    }

    #[test]
    fn a_design_becomes_responsive_html() {
        let h = html(&tree()).expect("renders");
        for needle in ["Hello", "https://example.com", "A red kettle", "Read more"] {
            assert!(h.contains(needle), "{needle} in {h}");
        }
        assert!(h.contains("max-width") || h.contains("@media"), "responsive");
        assert_eq!(h.matches("<body").count(), 1);
    }

    #[test]
    fn text_is_escaped_and_templates_survive() {
        let h = html(&with_block(json!({"type": "heading", "text": "Tom & <Jerry> {{ contact.first_name or \"there\" }}"}))).expect("renders");
        assert!(h.contains("Tom &amp; &lt;Jerry&gt;") && h.contains("{{ contact.first_name or \"there\" }}"), "{h}");
        assert!(crate::render::check(&h).is_ok(), "the template syntax holds");
    }

    #[test]
    fn an_unsubscribe_value_passes_as_an_address() {
        let h = html(&with_block(json!({"type": "button", "label": "Stop", "href": "{{ unsubscribe_url }}"}))).expect("renders");
        assert!(h.contains("{{ unsubscribe_url }}"), "{h}");
    }

    #[test]
    fn a_design_cannot_carry_mjml_or_includes() {
        for evil in ["<mj-include path=\"/etc/passwd\" />", "</mj-text><mj-raw><script>x</script></mj-raw>", "<MJ-RAW>x</MJ-RAW>"] {
            let e = html(&with_block(json!({"type": "text", "html": evil}))).unwrap_err();
            assert!(e.starts_with("row 1, column 1, block 1:"), "{e}");
        }
    }

    #[test]
    fn values_out_of_form_are_refused() {
        let bad = |block: Value, says: &str| {
            let e = html(&with_block(block)).unwrap_err();
            assert!(e.contains(says), "{e} should say {says}");
        };
        bad(json!({"type": "heading", "text": "x", "color": "red;background:url(x)"}), "colour");
        bad(json!({"type": "button", "label": "x", "href": "javascript:alert(1)"}), "address");
        bad(json!({"type": "button", "label": "x", "href": "data:text/html,x"}), "address");
        bad(json!({"type": "button", "label": "x", "href": "https://a.test/\"onclick=1"}), "address");
        bad(json!({"type": "image", "src": "http://a.test/a.png"}), "https://");
        bad(json!({"type": "image", "src": "https://a.test/a.png", "width": 5000}), "width");
        bad(json!({"type": "video", "src": "x"}), "video");
        bad(json!({"type": "text", "html": "x", "onclick": "x"}), "onclick");
        let mut four = tree();
        four["rows"][0]["columns"] = json!([{"blocks": []}, {"blocks": []}, {"blocks": []}, {"blocks": []}]);
        assert!(html(&four).unwrap_err().contains("1 to 3 columns"));
        let mut v2 = tree();
        v2["version"] = json!(2);
        assert!(html(&v2).unwrap_err().contains("version"));
    }

    #[test]
    fn limits_hold() {
        let mut many = tree();
        many["rows"] = json!(vec![tree()["rows"][0].clone(); 51]);
        assert!(html(&many).unwrap_err().contains("1 to 50 rows"));
        let blocks = vec![json!({"type": "spacer", "height": 10}); 21];
        let mut wide = tree();
        wide["rows"][0]["columns"][0]["blocks"] = json!(blocks);
        assert!(html(&wide).unwrap_err().contains("at most 20 blocks"));
        let mut big = tree();
        big["rows"][0]["columns"][0]["blocks"] = json!([{"type": "text", "html": "x".repeat(300_000)}]);
        assert!(html(&big).unwrap_err().contains("256 KB"));
        let mut empty = tree();
        empty["rows"][0]["columns"][0]["blocks"] = json!([]);
        assert_eq!(html(&empty).unwrap_err(), "add at least one block");
    }

    #[test]
    fn the_same_design_renders_the_same_html() {
        assert_eq!(html(&tree()).unwrap(), html(&tree()).unwrap());
    }
}
