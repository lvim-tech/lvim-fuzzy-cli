//! fzf command templates: `{}`, `{+}`, `{q}`, `{n}`, `{+n}`, `{1}`, `{2..}`,
//! with the `s` (keep whitespace), `r` (raw, unquoted), `f` (temp file) and
//! `+` (all selected) flags.

use crate::fields::{self, Delimiter};

pub struct Context<'a> {
    pub current: Option<(&'a str, u32)>,
    pub selected: Vec<(&'a str, u32)>,
    pub query: &'a str,
    pub delimiter: &'a Delimiter,
}

pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Expand placeholders in `template`. Temporary files created for `{f}` are
/// pushed to `temps` so the caller can remove them.
pub fn expand(template: &str, ctx: &Context, temps: &mut Vec<std::path::PathBuf>) -> String {
    let mut out = String::new();
    let b = template.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() && b[i + 1] == b'{' {
            out.push('{');
            i += 2;
            continue;
        }
        if b[i] == b'{'
            && let Some(close) = template[i + 1..].find('}')
        {
            let inner = &template[i + 1..i + 1 + close];
            if let Some(rep) = expand_one(inner, ctx, temps) {
                out.push_str(&rep);
                i += close + 2;
                continue;
            }
        }
        let ch = template[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn expand_one(inner: &str, ctx: &Context, temps: &mut Vec<std::path::PathBuf>) -> Option<String> {
    let mut plus = false;
    let mut raw = false;
    let mut keep_ws = false;
    let mut file = false;
    let mut rest = inner;
    while let Some(c) = rest.chars().next() {
        match c {
            '+' => plus = true,
            'r' if rest.len() > 1 || rest == "r" => raw = true,
            's' if rest.len() > 1 || rest == "s" => keep_ws = true,
            'f' if rest.len() > 1 || rest == "f" => file = true,
            _ => break,
        }
        rest = &rest[1..];
    }
    let field: Option<Vec<fields::Range>> = match rest {
        "" => None,
        "q" | "n" => None,
        r => Some(fields::parse_nth(r).ok()?),
    };
    if rest == "q" {
        let q = ctx.query;
        return Some(if raw { q.to_string() } else { quote(q) });
    }
    let items: Vec<(&str, u32)> = if plus {
        if ctx.selected.is_empty() {
            ctx.current.into_iter().collect()
        } else {
            ctx.selected.clone()
        }
    } else {
        ctx.current.into_iter().collect()
    };
    let vals: Vec<String> = items
        .iter()
        .map(|(text, idx)| {
            if rest == "n" {
                return idx.to_string();
            }
            let v = match &field {
                None => text.to_string(),
                Some(r) => fields::extract(text, ctx.delimiter, r),
            };
            let v = if keep_ws || field.is_none() {
                v
            } else {
                v.trim().to_string()
            };
            if raw || rest == "n" { v } else { quote(&v) }
        })
        .collect();
    if file {
        let path =
            std::env::temp_dir().join(format!("lvim-fuzzy-{}-{}", std::process::id(), temps.len()));
        let body: String = items
            .iter()
            .map(|(t, idx)| {
                let v = if rest == "n" {
                    idx.to_string()
                } else {
                    match &field {
                        None => t.to_string(),
                        Some(r) => fields::extract(t, ctx.delimiter, r),
                    }
                };
                v + "\n"
            })
            .collect();
        std::fs::write(&path, body).ok()?;
        let p = path.to_string_lossy().into_owned();
        temps.push(path);
        return Some(quote(&p));
    }
    if items.is_empty() {
        return Some(if raw { String::new() } else { "''".into() });
    }
    Some(vals.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands() {
        let d = Delimiter::Awk;
        let ctx = Context {
            current: Some(("a b'c", 3)),
            selected: vec![("x 1", 0), ("y 2", 1)],
            query: "q q",
            delimiter: &d,
        };
        let mut t = Vec::new();
        assert_eq!(expand("cat {}", &ctx, &mut t), r"cat 'a b'\''c'");
        assert_eq!(expand("{q}|{n}|{+n}", &ctx, &mut t), "'q q'|3|0 1");
        assert_eq!(expand("{+1}", &ctx, &mut t), "'x' 'y'");
        assert_eq!(expand("{r2}", &ctx, &mut t), "b'c");
        assert_eq!(expand(r"\{} {x}", &ctx, &mut t), "{} {x}");
    }
}
