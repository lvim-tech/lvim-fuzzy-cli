//! Drawing: the finder, the F1 help popup and toasts.

use super::*;
use crate::item::ItemView;

/// Category used to order the help popup.
fn category(action: &str) -> (u8, &'static str) {
    let a = action.split(['(', '+']).next().unwrap_or(action);
    match a {
        "accept"
        | "accept-non-empty"
        | "accept-or-print-query"
        | "abort"
        | "cancel"
        | "become"
        | "print-query" => (0, "Accept / quit"),
        "up" | "down" | "first" | "last" | "page-up" | "page-down" | "half-page-up"
        | "half-page-down" | "pos" | "offset-up" | "offset-down" | "jump" => (1, "Move"),
        "toggle" | "select" | "deselect" | "select-all" | "deselect-all" | "toggle-all"
        | "clear-selection" | "toggle-in" | "toggle-out" => (2, "Select"),
        a if a.starts_with("preview") || a.ends_with("preview") => (4, "Preview"),
        "beginning-of-line"
        | "end-of-line"
        | "backward-char"
        | "forward-char"
        | "backward-word"
        | "forward-word"
        | "backward-delete-char"
        | "delete-char"
        | "unix-line-discard"
        | "unix-word-rubout"
        | "kill-word"
        | "backward-kill-word"
        | "kill-line"
        | "yank"
        | "clear-query"
        | "put" => (3, "Edit query"),
        _ => (5, "Other"),
    }
}

/// `rect` shrunk by a --margin spec ([t, r, b, l], N or N%).
pub(super) fn inset(rect: Rect, spec: &[(u16, bool); 4]) -> Rect {
    let v = |(n, pct): (u16, bool), total: u16| {
        if pct {
            (total as u32 * n as u32 / 100) as u16
        } else {
            n
        }
    };
    let t = v(spec[0], rect.h);
    let r = v(spec[1], rect.w);
    let b = v(spec[2], rect.h);
    let l = v(spec[3], rect.w);
    Rect {
        x: rect.x + l,
        y: rect.y + t,
        w: rect.w.saturating_sub(l + r),
        h: rect.h.saturating_sub(t + b),
    }
}

impl App {
    /// How many items match the query (in raw mode the list shows them all).
    pub(super) fn matched_count(&self) -> usize {
        self.raw_set.as_ref().map_or(self.hits.len(), |s| s.len())
    }

    /// Highlight positions (chars of the display text) for a row, following
    /// the filter scope of grep results.
    pub(super) fn row_highlights(&self, p: &Pattern, it: ItemView) -> Vec<usize> {
        use crate::engine::Scope;
        match (it.grep, self.scope) {
            (Some((_, ts)), Scope::Line) => {
                let d = it.display();
                let off = d[..ts as usize].chars().count();
                p.highlights(&d[ts as usize..])
                    .map(|v| v.into_iter().map(|x| x + off).collect())
                    .unwrap_or_default()
            }
            (Some((pe, _)), Scope::Path) => p
                .highlights(&it.display()[..pe as usize])
                .unwrap_or_default(),
            // the file matched somewhere: mark the query in the row too, when it is there
            (Some(_), Scope::File) => {
                let mut cfg = crate::match_settings(&self.opts).cfg;
                cfg.exact = true;
                Pattern::parse(&self.highlight_query(), &cfg)
                    .highlights(it.display())
                    .unwrap_or_default()
            }
            _ => p
                .highlights(it.target())
                .map(|pos| it.map_positions(&pos))
                .unwrap_or_default(),
        }
    }

    fn help_lines(&self) -> Vec<(String, String, String)> {
        let mut v: Vec<(u8, String, String, String)> = self
            .keymap
            .iter()
            .filter(|(k, _)| {
                !matches!(
                    k.as_str(),
                    "start"
                        | "load"
                        | "change"
                        | "focus"
                        | "result"
                        | "resize"
                        | "one"
                        | "zero"
                        | "multi"
                        | "backward-eof"
                )
            })
            .map(|(k, acts)| {
                let desc = acts
                    .iter()
                    .map(|a| match &a.arg {
                        Some(x) => format!("{}({x})", a.name),
                        None => a.name.clone(),
                    })
                    .collect::<Vec<_>>()
                    .join(" + ");
                let (c, cat) = category(&desc);
                (c, cat.to_string(), k.clone(), desc)
            })
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0).then(a.3.cmp(&b.3)).then(a.2.cmp(&b.2)));
        v.into_iter().map(|(_, c, k, d)| (c, k, d)).collect()
    }

    pub(super) fn help_key(&mut self, name: &str) {
        let Some(h) = self.help.as_mut() else { return };
        match name {
            "esc" | "f1" | "ctrl-c" | "ctrl-g" | "ctrl-q" => self.help = None,
            "up" | "ctrl-p" | "ctrl-k" | "scroll-up" => h.offset = h.offset.saturating_sub(1),
            // never past the last screen, or `up` would first have to undo the overshoot
            "down" | "ctrl-n" | "ctrl-j" | "scroll-down" => {
                h.offset = (h.offset + 1).min(h.max_off.get())
            }
            "page-up" => h.offset = h.offset.saturating_sub(10),
            "page-down" => h.offset = (h.offset + 10).min(h.max_off.get()),
            "home" => h.offset = 0,
            "end" => h.offset = h.max_off.get(),
            "bspace" => {
                h.query.pop();
                h.offset = 0;
            }
            "ctrl-u" => {
                h.query.clear();
                h.offset = 0;
            }
            "space" => h.query.push(' '),
            n if n.chars().count() == 1 => {
                h.query.push_str(n);
                h.offset = 0;
            }
            _ => {}
        }
    }

    pub(super) fn draw_separator(&self, cv: &mut Canvas, x: u16, y: u16, w: u16) {
        if w == 0 {
            return;
        }
        let s = (if self.opts.unicode { "─" } else { "-" }).repeat(w as usize);
        cv.put(x, y, &s, self.theme.get(Slot::Separator), x + w);
    }

    pub(super) fn draw_toast(&self, cv: &mut Canvas, area: Rect) {
        let Some(t) = &self.toast else { return };
        let maxw = (area.w as usize).saturating_sub(4).clamp(10, 60);
        let mut lines = Vec::new();
        let mut cur = String::new();
        for word in t.text.split(' ') {
            if str_width(&cur) + str_width(word) + 1 > maxw && !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(word);
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
        lines.truncate((area.h as usize).saturating_sub(2).max(1));
        // the common popup frame; the mark in the accent colour
        let th = &self.theme;
        let st = self.popup_style(None);
        let mark = if t.error { "✗ " } else { "● " };
        let mw = str_width(mark) as u16;
        let inner_w = lines.iter().map(|l| str_width(l)).max().unwrap_or(0) as u16;
        let (bw, bh) = self.popup_outer(inner_w + mw, lines.len() as u16);
        let (bw, bh) = (bw.min(area.w), bh.min(area.h));
        let r = Rect {
            x: area.x + area.w.saturating_sub(bw + 1),
            y: area.y,
            w: bw,
            h: bh,
        };
        let inner = self.popup_frame(cv, r, None, &st);
        let bg = st.panel.bg;
        let accent = if t.error {
            th.get(Slot::Pointer)
        } else {
            th.get(Slot::Info)
        };
        let limit = inner.x + inner.w;
        cv.put(inner.x, inner.y, mark, Style { bg, ..accent }, limit);
        for (i, l) in lines.iter().enumerate().take(inner.h as usize) {
            cv.put(inner.x + mw, inner.y + i as u16, l, st.panel, limit);
        }
    }

    pub(super) fn draw_help(&self, cv: &mut Canvas, area: Rect) {
        let Some(h) = &self.help else { return };
        let th = &self.theme;
        let q = h.query.to_lowercase();
        let all = self.help_lines();
        let rows: Vec<&(String, String, String)> = all
            .iter()
            .filter(|(c, k, d)| {
                q.is_empty()
                    || k.to_lowercase().contains(&q)
                    || d.to_lowercase().contains(&q)
                    || c.to_lowercase().contains(&q)
            })
            .collect();
        let w = area.w.saturating_sub(4).min(84).max(20.min(area.w));
        let hgt = area.h.saturating_sub(2).max(3.min(area.h));
        let r = Rect {
            x: area.x + (area.w - w) / 2,
            y: area.y + (area.h - hgt) / 2,
            w,
            h: hgt,
        };
        let st = self.popup_style(None);
        let panel_bg = st.panel.bg;
        let inner = self.popup_frame(cv, r, Some("Keys · lvim-fuzzy"), &st);
        if inner.h < 3 {
            return;
        }
        let limit = inner.x + inner.w;
        // search line
        let mut cx = cv.put(
            inner.x,
            inner.y,
            "/ ",
            Style {
                bg: panel_bg,
                ..th.get(Slot::Prompt)
            },
            limit,
        );
        if h.query.is_empty() {
            cv.put(
                cx,
                inner.y,
                "type to filter keys and actions",
                Style {
                    bg: panel_bg,
                    ..th.get(Slot::Ghost)
                },
                limit,
            );
        } else {
            cx = cv.put(
                cx,
                inner.y,
                &h.query,
                Style {
                    bg: panel_bg,
                    ..th.get(Slot::Query)
                },
                limit,
            );
            cv.put(
                cx,
                inner.y,
                "▏",
                Style {
                    bg: panel_bg,
                    ..th.get(Slot::Pointer)
                },
                limit,
            );
        }
        let count = format!("{} keys", rows.len());
        cv.put(
            limit.saturating_sub(str_width(&count) as u16),
            inner.y,
            &count,
            Style {
                bg: panel_bg,
                ..th.get(Slot::Info)
            },
            limit,
        );
        let body_h = inner.h.saturating_sub(3) as usize;
        let key_w = rows
            .iter()
            .map(|r| str_width(&r.1))
            .max()
            .unwrap_or(0)
            .min(22) as u16;
        // render with category headings
        let mut lines: Vec<(Option<String>, Option<&(String, String, String)>)> = Vec::new();
        let mut last_cat = String::new();
        for r in &rows {
            if r.0 != last_cat {
                if !lines.is_empty() {
                    lines.push((None, None));
                }
                lines.push((Some(r.0.clone()), None));
                last_cat = r.0.clone();
            }
            lines.push((None, Some(r)));
        }
        let max_off = lines.len().saturating_sub(body_h);
        h.max_off.set(max_off);
        let off = h.offset.min(max_off);
        for (i, line) in lines.iter().skip(off).take(body_h).enumerate() {
            let y = inner.y + 2 + i as u16;
            match line {
                (Some(cat), _) => {
                    let x = cv.put(
                        inner.x,
                        y,
                        cat,
                        Style {
                            bg: panel_bg,
                            ..th.get(Slot::Header)
                        },
                        limit,
                    );
                    self.draw_separator(cv, x + 1, y, limit.saturating_sub(x + 1));
                }
                (None, Some((_, k, d))) => {
                    if let Some(l) = &self.look {
                        // the lvim cheatsheet: a key box + a description box, striped
                        let s = (off + i) % 2;
                        let kw = key_w + 2;
                        cv.fill(
                            Rect {
                                x: inner.x,
                                y,
                                w: kw,
                                h: 1,
                            },
                            l.help_key[s],
                        );
                        cv.put(inner.x + 1, y, k, l.help_key[s], inner.x + kw);
                        cv.fill(
                            Rect {
                                x: inner.x + kw,
                                y,
                                w: limit.saturating_sub(inner.x + kw),
                                h: 1,
                            },
                            l.help_desc[s],
                        );
                        cv.put(inner.x + kw + 1, y, d, l.help_desc[s], limit);
                    } else {
                        cv.put(
                            inner.x + 1,
                            y,
                            k,
                            Style {
                                bg: panel_bg,
                                ..th.get(Slot::Hl)
                            },
                            inner.x + 1 + key_w,
                        );
                        cv.put(
                            inner.x + 2 + key_w,
                            y,
                            d,
                            Style {
                                bg: panel_bg,
                                ..th.get(Slot::Fg)
                            },
                            limit,
                        );
                    }
                }
                _ => {}
            }
        }
        let hint = " esc close · type to filter · ↑↓ PgUp PgDn scroll ";
        let hx = r.x + (r.w.saturating_sub(str_width(hint) as u16)) / 2;
        cv.put(
            hx,
            r.y + r.h - 1,
            hint,
            Style {
                bg: panel_bg,
                ..th.get(Slot::Info)
            },
            r.x + r.w - 1,
        );
        if lines.len() > body_h && body_h > 0 {
            let bar = ((body_h * body_h) / lines.len()).max(1);
            let pos = if max_off == 0 {
                0
            } else {
                off * (body_h - bar) / max_off
            };
            for k in 0..bar {
                // in the padding column right of the content, inside the border
                let sx = inner.x + inner.w;
                cv.put(
                    sx,
                    inner.y + 2 + (pos + k) as u16,
                    "┃",
                    Style {
                        bg: panel_bg,
                        ..th.get(Slot::Scrollbar)
                    },
                    sx + 1,
                );
            }
        }
    }

    pub(super) fn draw(&mut self, term: &mut Term) -> std::io::Result<()> {
        self.draw_lvim(term)
    }

    pub(super) fn flush(
        &self,
        term: &mut Term,
        cv: Canvas,
        cursor_pos: Option<(u16, u16)>,
    ) -> std::io::Result<()> {
        let out = &mut term.out;
        queue!(out, terminal::BeginSynchronizedUpdate, cursor::Hide)?;
        // icon images drawn for the first time in this frame: sent before the cells that show them
        let pending = lvim_fuzzy_ui::iconimg::take_pending();
        if !pending.is_empty() {
            std::io::Write::write_all(out, &pending)?;
        }
        cv.flush(out, term.prev.as_ref(), &mut term.pos)?;
        if let Some((x, y)) = cursor_pos {
            term.pos.goto(out, x, y)?;
            queue!(out, cursor::Show)?;
        }
        queue!(out, terminal::EndSynchronizedUpdate)?;
        out.flush()?;
        term.prev = Some(cv);
        Ok(())
    }
}
