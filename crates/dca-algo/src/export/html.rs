//! Small helpers for the HTML pages of the export: escaping, numbers and
//! dates by language, base64 for the embedded images, the page skeleton.

use std::fmt::Write as _;

use super::text;
use crate::schedule::{Date, Lang};

/// Style sheet of the pages (the plugin's report style).
pub const CSS: &str = "body{font-family:Segoe UI,Helvetica,Arial,sans-serif;margin:24px;color:#1d2330;line-height:1.45;max-width:1100px}\
h1{font-size:22px;margin:0 0 4px}h2{font-size:17px;margin-top:28px;border-bottom:2px solid #d9dde6;padding-bottom:4px}\
table{border-collapse:collapse;width:100%;margin:8px 0;font-size:13px}th,td{border:1px solid #d9dde6;padding:4px 6px;text-align:right}\
th{background:#f1f3f7}td.l,th.l{text-align:left}.warn{background:#fff4e0}.bad{background:#fde8e8}.ok{color:#1a7f37}\
.note{font-size:12px;color:#5b6475}.box{border:1px solid #d9dde6;border-radius:6px;padding:10px;background:#fafbfd}\
.disc{border-left:4px solid #c0392b;padding:8px 12px;background:#fdf2f1;font-size:13px}img{max-width:48%;height:auto}";

/// Escapes text for HTML.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// A number with `nd` decimals, the decimal mark of the language.
pub fn num(lang: Lang, x: f64, nd: usize) -> String {
    let s = format!("{x:.nd$}");
    // no "-0.00"
    let s = if s
        .trim_start_matches('-')
        .chars()
        .all(|c| c == '0' || c == '.')
    {
        s.trim_start_matches('-').to_owned()
    } else {
        s
    };
    match lang {
        Lang::En => s,
        Lang::It => s.replace('.', ","),
    }
}

/// A date: `YYYY-MM-DD` in English, `DD/MM/YYYY` in Italian.
pub fn date(lang: Lang, d: Date) -> String {
    match lang {
        Lang::En => d.to_iso(),
        Lang::It => d.dmy(),
    }
}

/// Standard base64 with padding.
pub fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | (u32::from(b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(A[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A PNG as an `<img>` with the bytes inline.
pub fn png_img(png: &[u8], alt: &str) -> String {
    format!(
        "<img alt=\"{}\" src=\"data:image/png;base64,{}\">",
        esc(alt),
        base64(png)
    )
}

/// The page skeleton: the document with `body` inside.
pub fn page(lang: Lang, title: &str, body: &str) -> String {
    let code = match lang {
        Lang::En => "en",
        Lang::It => "it",
    };
    let mut out = String::new();
    let _ = write!(
        out,
        "<!doctype html>\n<html lang=\"{code}\"><head><meta charset=\"utf-8\"><title>{}</title><style>{CSS}</style></head><body>\n{body}\n</body></html>\n",
        esc(title)
    );
    out
}

/// Opening `<tr>` of a table row with an optional class.
pub fn tr(class: &str) -> String {
    if class.is_empty() {
        "<tr>".to_owned()
    } else {
        format!("<tr class=\"{class}\">")
    }
}

/// A header row of `<th>` cells; a key starting with `<` marks a left
/// aligned column (`<key`).
pub fn head(lang: Lang, keys: &[&str]) -> String {
    let mut out = String::from("<tr>");
    for k in keys {
        match k.strip_prefix('<') {
            Some(k) => {
                let _ = write!(out, "<th class=\"l\">{}</th>", esc(text::tr(lang, k)));
            }
            None => {
                let _ = write!(out, "<th>{}</th>", esc(text::tr(lang, k)));
            }
        }
    }
    out.push_str("</tr>");
    out
}

/// Checks that `html` is well formed enough for a browser to build the tree
/// the author meant: balanced tags (void elements aside), attribute quotes
/// closed, text escaped.
///
/// # Errors
/// The first problem found.
pub fn check_wellformed(html: &str) -> Result<usize, String> {
    const VOID: [&str; 6] = ["meta", "img", "br", "hr", "link", "input"];
    let mut stack: Vec<String> = Vec::new();
    let mut tags = 0;
    let b = html.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            if b[i] == b'>' {
                return Err(format!("stray > at {i}"));
            }
            i += 1;
            continue;
        }
        // a tag runs to the next > outside quotes
        let mut j = i + 1;
        let mut quote = 0u8;
        while j < b.len() && (quote != 0 || b[j] != b'>') {
            if quote != 0 {
                if b[j] == quote {
                    quote = 0;
                }
            } else if b[j] == b'"' || b[j] == b'\'' {
                quote = b[j];
            } else if b[j] == b'<' {
                return Err(format!("< inside a tag at {j}"));
            }
            j += 1;
        }
        if j >= b.len() {
            return Err(format!("unterminated tag at {i}"));
        }
        let inner = &html[i + 1..j];
        i = j + 1;
        if inner.starts_with('!') {
            continue;
        }
        tags += 1;
        if let Some(name) = inner.strip_prefix('/') {
            let name = name.trim().to_ascii_lowercase();
            match stack.pop() {
                Some(open) if open == name => {}
                other => return Err(format!("</{name}> closes {other:?}")),
            }
            continue;
        }
        let name: String = inner
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if name.is_empty() {
            return Err(format!("empty tag name at {i}"));
        }
        if !VOID.contains(&name.as_str()) && !inner.ends_with('/') {
            stack.push(name);
        }
    }
    if stack.is_empty() {
        Ok(tags)
    } else {
        Err(format!("not closed: {stack:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_vectors() {
        for (i, o) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(i.as_bytes()), o);
        }
    }

    #[test]
    fn text_is_escaped_and_numbers_follow_the_language() {
        assert_eq!(
            esc("a<b & \"c\" 'd'>"),
            "a&lt;b &amp; &quot;c&quot; &#39;d&#39;&gt;"
        );
        assert_eq!(num(Lang::En, 1.5, 2), "1.50");
        assert_eq!(num(Lang::It, 1.5, 2), "1,50");
        assert_eq!(num(Lang::En, -0.001, 2), "0.00");
        assert_eq!(num(Lang::En, -2.0, 1), "-2.0");
        assert_eq!(
            date(
                Lang::It,
                Date {
                    year: 2026,
                    month: 2,
                    day: 3
                }
            ),
            "03/02/2026"
        );
        assert_eq!(
            date(
                Lang::En,
                Date {
                    year: 2026,
                    month: 2,
                    day: 3
                }
            ),
            "2026-02-03"
        );
    }

    #[test]
    fn the_well_formedness_check_catches_broken_pages() {
        assert!(
            check_wellformed(
                "<!doctype html><html><body><p>a<br>b</p><img src=\"x>y\"></body></html>"
            )
            .is_ok()
        );
        assert!(check_wellformed("<p><b>x</p></b>").is_err());
        assert!(check_wellformed("<div><p>x</p>").is_err());
        assert!(check_wellformed("<p title=\"x>y</p>").is_err());
        assert!(check_wellformed("<p>1 > 0</p>").is_err());
    }
}
