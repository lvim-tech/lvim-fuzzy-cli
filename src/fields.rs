//! fzf field handling: AWK-style or regex-delimited tokens and the
//! `--nth` / `--with-nth` / `--accept-nth` index expressions.

use regex::Regex;

#[derive(Clone, Debug)]
pub enum Delimiter {
    Awk,
    Literal(String),
    Regex(Regex),
}

impl Delimiter {
    /// fzf treats the value as a regex, falling back to a literal string
    /// when it does not compile.
    pub fn parse(s: &str) -> Delimiter {
        match Regex::new(s) {
            Ok(re) => Delimiter::Regex(re),
            Err(_) => Delimiter::Literal(s.to_string()),
        }
    }

    /// Byte ranges of the tokens; each token includes its trailing delimiter.
    pub fn tokens(&self, text: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        match self {
            Delimiter::Awk => {
                let b = text.as_bytes();
                let mut i = 0;
                while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
                    i += 1;
                }
                while i < b.len() {
                    let start = i;
                    while i < b.len() && b[i] != b' ' && b[i] != b'\t' {
                        i += 1;
                    }
                    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
                        i += 1;
                    }
                    out.push((start, i));
                }
            }
            Delimiter::Literal(d) => {
                let mut start = 0;
                if !d.is_empty() {
                    while let Some(p) = text[start..].find(d.as_str()) {
                        let e = start + p + d.len();
                        out.push((start, e));
                        start = e;
                    }
                }
                if start < text.len() {
                    out.push((start, text.len()));
                }
            }
            Delimiter::Regex(re) => {
                let mut start = 0;
                for m in re.find_iter(text) {
                    if m.end() == m.start() {
                        continue;
                    }
                    out.push((start, m.end()));
                    start = m.end();
                }
                if start < text.len() {
                    out.push((start, text.len()));
                }
            }
        }
        out
    }

    /// Strip one trailing delimiter (or trailing whitespace for AWK style).
    pub fn strip_trailing<'a>(&self, s: &'a str) -> &'a str {
        match self {
            Delimiter::Awk => s.trim_end_matches([' ', '\t']),
            Delimiter::Literal(d) => s.strip_suffix(d.as_str()).unwrap_or(s),
            Delimiter::Regex(re) => {
                let mut cut = s.len();
                for m in re.find_iter(s) {
                    if m.end() == s.len() && m.end() > m.start() {
                        cut = m.start();
                    }
                }
                &s[..cut]
            }
        }
    }
}

/// One element of an index expression: `N`, `-N`, `A..B`, `..B`, `A..`, `..`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub begin: Option<i32>,
    pub end: Option<i32>,
}

pub fn parse_nth(s: &str) -> Result<Vec<Range>, String> {
    let mut out = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        let bad = || format!("invalid field index expression: {part}");
        if let Some((a, b)) = part.split_once("..") {
            let begin = if a.is_empty() {
                None
            } else {
                Some(a.parse::<i32>().map_err(|_| bad())?)
            };
            let end = if b.is_empty() {
                None
            } else {
                Some(b.parse::<i32>().map_err(|_| bad())?)
            };
            if begin == Some(0) || end == Some(0) {
                return Err(bad());
            }
            out.push(Range { begin, end });
        } else {
            let n: i32 = part.parse().map_err(|_| bad())?;
            if n == 0 {
                return Err(bad());
            }
            out.push(Range {
                begin: Some(n),
                end: Some(n),
            });
        }
    }
    Ok(out)
}

fn resolve(idx: i32, count: usize) -> i64 {
    if idx > 0 {
        idx as i64 - 1
    } else {
        count as i64 + idx as i64
    }
}

/// Token index spans selected by `ranges` (inclusive, clamped), in order.
pub fn select(ranges: &[Range], count: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if count == 0 {
        return out;
    }
    for r in ranges {
        let b = r.begin.map_or(0, |v| resolve(v, count));
        let e = r.end.map_or(count as i64 - 1, |v| resolve(v, count));
        let b = b.max(0);
        let e = e.min(count as i64 - 1);
        if b > e || b >= count as i64 {
            continue;
        }
        out.push((b as usize, e as usize));
    }
    out
}

/// Concatenate the selected tokens' raw text (as fzf does) and strip the
/// trailing delimiter.
pub fn extract(text: &str, delim: &Delimiter, ranges: &[Range]) -> String {
    let toks = delim.tokens(text);
    let mut s = String::new();
    for (b, e) in select(ranges, toks.len()) {
        s.push_str(&text[toks[b].0..toks[e].1]);
    }
    delim.strip_trailing(&s).to_string()
}

/// Byte ranges of `text` that `--nth` limits matching to.
pub fn match_spans(text: &str, delim: &Delimiter, ranges: &[Range]) -> Vec<(usize, usize)> {
    let toks = delim.tokens(text);
    select(ranges, toks.len())
        .into_iter()
        .map(|(b, e)| (toks[b].0, toks[e].1))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acc(line: &str, d: &Delimiter, spec: &str) -> String {
        extract(line, d, &parse_nth(spec).unwrap())
    }

    #[test]
    fn awk_matches_fzf() {
        let d = Delimiter::Awk;
        let l = "  a b  c ";
        assert_eq!(acc(l, &d, "1"), "a");
        assert_eq!(acc(l, &d, "3"), "c");
        assert_eq!(acc(l, &d, "-1"), "c");
        assert_eq!(acc(l, &d, "2.."), "b  c");
        assert_eq!(acc(l, &d, "..2"), "a b");
        assert_eq!(acc(l, &d, "1,3"), "a c");
        assert_eq!(acc("a b c", &d, "3,1"), "ca");
    }

    #[test]
    fn delimiter_matches_fzf() {
        let d = Delimiter::parse(":");
        let l = "a:b::c";
        assert_eq!(acc(l, &d, "1"), "a");
        assert_eq!(acc(l, &d, "-1"), "c");
        assert_eq!(acc(l, &d, "2.."), "b::c");
        assert_eq!(acc(l, &d, "1,3"), "a:");
        assert_eq!(acc(l, &d, "3,1"), ":a");
        assert_eq!(acc("a:b:c", &d, "5"), "");
    }
}
