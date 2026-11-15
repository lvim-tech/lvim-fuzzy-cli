//! Input items and their derived texts (display / match target).

use crate::ansi::{self, Run};
use crate::fields::{self, Delimiter, Range};
use crate::theme::Style;

/// One input line, as the finder shows it. Its match target (the line itself,
/// or the `--with-nth` / `--nth` text) lives in the search store of `Items`;
/// the item keeps only what differs from it.
#[derive(Debug)]
pub struct Item {
    pub index: u32,
    /// A grep result (`path:line[:col]:text`): byte offsets in the display
    /// text where the path ends and the line's text starts.
    pub grep: Option<(u32, u32)>,
    /// What only some items have (None for a plain line: 24 bytes an item).
    pub extra: Option<Box<Extra>>,
}

/// The rarer parts of an item.
#[derive(Debug, Default)]
pub struct Extra {
    /// The original line, printed on accept, when it is not the match target
    /// (with --with-nth / --nth); None: the target is the line itself.
    pub raw: Option<Box<str>>,
    /// `--with-nth` transformed text, shown in the list (None = raw).
    pub display: Option<Box<str>>,
    /// Byte offsets in the display text of each `--nth` span, for highlighting.
    pub spans: Option<Box<[(u32, u32)]>>,
    /// `--ansi` style runs over the display text.
    pub ansi: Option<Box<[Run]>>,
}

/// `path:line[:col]:text` → (end of path, start of text), in bytes.
pub fn grep_shape(s: &str) -> Option<(u32, u32)> {
    let b = s.as_bytes();
    let mut i = 0;
    // the path: up to the first `:` followed by digits and `:`
    while i < b.len() {
        if b[i] == b':' && i > 0 {
            let mut j = i + 1;
            let d0 = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j > d0 && j < b.len() && b[j] == b':' {
                let path_end = i;
                let mut text = j + 1;
                // optional `col:`
                let mut k = text;
                while k < b.len() && b[k].is_ascii_digit() {
                    k += 1;
                }
                if k > text && k < b.len() && b[k] == b':' {
                    text = k + 1;
                }
                return Some((path_end as u32, text as u32));
            }
        }
        i += 1;
    }
    None
}

pub struct ItemBuilder {
    pub delimiter: Delimiter,
    pub with_nth: Option<Vec<Range>>,
    pub nth: Option<Vec<Range>>,
    pub ansi: bool,
}

impl Item {
    pub fn raw(&self) -> Option<&str> {
        self.extra.as_ref()?.raw.as_deref()
    }

    pub fn display(&self) -> Option<&str> {
        self.extra.as_ref()?.display.as_deref()
    }

    pub fn spans(&self) -> Option<&[(u32, u32)]> {
        self.extra.as_ref()?.spans.as_deref()
    }

    pub fn ansi(&self) -> Option<&[Run]> {
        self.extra.as_ref()?.ansi.as_deref()
    }

    /// A plain item (no --with-nth / --nth / --ansi): its text is all in the
    /// store.
    pub fn plain(index: u32, text: &str) -> Item {
        Item {
            index,
            grep: grep_shape(text),
            extra: None,
        }
    }
}

impl ItemBuilder {
    /// The item for `raw`, and its match target (for the store).
    pub fn build(&self, index: u32, raw: String) -> (Item, String) {
        // With --ansi the escape codes are stripped (the printed line is the
        // plain text, as in fzf) and kept as style runs.
        let (raw, raw_runs) = if self.ansi && raw.contains('\x1b') {
            let (t, r, _) = ansi::parse(&raw, Style::default());
            (t, Some(r))
        } else {
            (raw, None)
        };
        let mut disp_runs = raw_runs.clone();
        let display = self.with_nth.as_ref().map(|r| {
            let toks = self.delimiter.tokens(&raw);
            let mut s = String::new();
            let mut runs = Vec::new();
            for (b, e) in fields::select(r, toks.len()) {
                let (a, z) = (toks[b].0, toks[e].1);
                if let Some(rr) = &raw_runs {
                    runs.push(Run {
                        start: s.len() as u32,
                        style: ansi::style_at(rr, a),
                    });
                    for run in rr
                        .iter()
                        .filter(|x| (x.start as usize) > a && (x.start as usize) < z)
                    {
                        runs.push(Run {
                            start: (s.len() + run.start as usize - a) as u32,
                            style: run.style,
                        });
                    }
                }
                s.push_str(&raw[a..z]);
            }
            if raw_runs.is_some() {
                disp_runs = Some(runs);
            }
            s.trim_end_matches(['\n']).to_string().into_boxed_str()
        });
        let (target, spans) = match &self.nth {
            None => (None, None),
            Some(r) => {
                let base: &str = display.as_deref().unwrap_or(&raw);
                let mut sp = fields::match_spans(base, &self.delimiter, r);
                // as fzf: the last field loses its trailing delimiter, so `lua$` matches `lua/`
                if let Some(last) = sp.last_mut() {
                    let kept = self.delimiter.strip_trailing(&base[last.0..last.1]).len();
                    last.1 = last.0 + kept;
                }
                let mut t = String::new();
                for &(b, e) in &sp {
                    t.push_str(&base[b..e]);
                }
                let spans: Box<[(u32, u32)]> =
                    sp.iter().map(|&(b, e)| (b as u32, e as u32)).collect();
                (Some(t.into_boxed_str()), Some(spans))
            }
        };
        let grep = if self.nth.is_none() {
            grep_shape(display.as_deref().unwrap_or(&raw))
        } else {
            None
        };
        // the target goes to the store; the line is kept only when it differs
        let (raw, text) = match (target, &display) {
            (Some(t), _) => (Some(raw.into_boxed_str()), t.into_string()),
            (None, Some(d)) => (Some(raw.into_boxed_str()), d.to_string()),
            (None, None) => (None, raw),
        };
        let extra = Extra {
            raw,
            display,
            spans,
            ansi: disp_runs.map(Vec::into_boxed_slice),
        };
        let any = extra.raw.is_some()
            || extra.display.is_some()
            || extra.spans.is_some()
            || extra.ansi.is_some();
        let item = Item {
            index,
            grep,
            extra: any.then(|| Box::new(extra)),
        };
        (item, text)
    }

    /// Whether every item is its own line (no --with-nth / --nth / --ansi):
    /// then an item holds no text at all.
    pub fn plain(&self) -> bool {
        self.with_nth.is_none() && self.nth.is_none() && !self.ansi
    }
}

/// The items of the finder: what each shows, and the search store holding
/// every match target (item i is candidate i).
#[derive(Default)]
pub struct Items {
    list: Vec<Item>,
    store: lvim_fuzzy_core::Store,
    /// --nth: the targets are selected fields (no grep scopes then)
    pub nth: bool,
}

/// A part built on a reading thread: items and their store, in step.
pub struct Part {
    pub items: Vec<Item>,
    pub store: lvim_fuzzy_core::Store,
}

impl Part {
    pub fn new() -> Part {
        Part {
            items: Vec::new(),
            store: lvim_fuzzy_core::Store::new(),
        }
    }

    pub fn push(&mut self, (item, text): (Item, String)) {
        self.store.push(text.as_bytes());
        self.items.push(item);
    }

    pub fn get(&self, i: usize) -> Option<ItemView<'_>> {
        Some(ItemView {
            it: self.items.get(i)?,
            text: self.store.text(i),
        })
    }
}

impl Default for Part {
    fn default() -> Part {
        Part::new()
    }
}

impl Items {
    pub fn len(&self) -> usize {
        self.list.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn store(&self) -> &lvim_fuzzy_core::Store {
        &self.store
    }

    pub fn get(&self, i: usize) -> Option<ItemView<'_>> {
        let it = self.list.get(i)?;
        Some(ItemView {
            it,
            text: self.store.text(i),
        })
    }

    /// Item `i` (which must exist).
    pub fn at(&self, i: usize) -> ItemView<'_> {
        self.get(i).expect("item index in range")
    }

    pub fn reserve(&mut self, n: usize) {
        self.list.reserve(n);
    }

    pub fn push(&mut self, (item, text): (Item, String)) {
        self.store.push(text.as_bytes());
        self.list.push(item);
    }

    /// Append a part (taken as it is when empty: moved, not copied).
    pub fn extend(&mut self, part: Part) {
        if self.list.is_empty() && self.list.capacity() <= part.items.capacity() {
            self.list = part.items;
        } else {
            self.list.extend(part.items);
        }
        self.store.absorb(part.store);
    }

    /// --tail: keep the last `n` items, numbered from 0 again.
    pub fn keep_last(&mut self, n: usize) {
        if self.list.len() <= n {
            return;
        }
        let drop = self.list.len() - n;
        self.list.drain(..drop);
        for (i, it) in self.list.iter_mut().enumerate() {
            it.index = i as u32;
        }
        self.store.keep_last(n);
    }

    /// Every item built again from its line (change-nth / change-with-nth).
    pub fn rebuild(&mut self, b: &ItemBuilder) {
        let mut next = Items {
            nth: b.nth.is_some(),
            ..Items::default()
        };
        next.list.reserve(self.list.len());
        for i in 0..self.list.len() {
            let v = self.at(i);
            next.push(b.build(v.index, v.raw().to_string()));
        }
        *self = next;
    }
}

/// An item with its texts (from the store when the item does not hold them).
#[derive(Clone, Copy)]
pub struct ItemView<'a> {
    it: &'a Item,
    text: &'a str,
}

impl std::ops::Deref for ItemView<'_> {
    type Target = Item;
    fn deref(&self) -> &Item {
        self.it
    }
}

impl<'a> ItemView<'a> {
    /// The original line (printed on accept).
    pub fn raw(&self) -> &'a str {
        self.it.raw().unwrap_or(self.text)
    }

    /// What the list shows (--with-nth applied).
    pub fn display(&self) -> &'a str {
        self.it.display().unwrap_or_else(|| self.raw())
    }

    /// What the pattern runs against (--nth applied).
    pub fn target(&self) -> &'a str {
        self.text
    }
}

impl ItemView<'_> {
    /// Map char positions in the match target back to char positions in the
    /// display text.
    pub fn map_positions(&self, pos: &[usize]) -> Vec<usize> {
        let Some(spans) = self.it.spans() else {
            return pos.to_vec();
        };
        let disp = self.display();
        // char index of each span start in the display text, and its length in chars
        let mut table = Vec::with_capacity(spans.len());
        for &(b, e) in spans.iter() {
            let start_chars = disp[..b as usize].chars().count();
            let len = disp[b as usize..e as usize].chars().count();
            table.push((start_chars, len));
        }
        let mut out = Vec::with_capacity(pos.len());
        for &p in pos {
            let mut off = p;
            for &(start, len) in &table {
                if off < len {
                    out.push(start + off);
                    break;
                }
                off -= len;
            }
        }
        out
    }
}
