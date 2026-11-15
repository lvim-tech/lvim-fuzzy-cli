//! The lvim look: lvim-picker's layout drawn in the terminal.
//!
//!   ┌ title bar ─────────────────────────────── counter ┐
//!   │  badge  input field     │  icon file  dir/        │
//!   │  rows (striped)         │  preview                │
//!   └──────── footer: key badges + labels ──────────────┘
//!
//! The button bar can sit at the top or the bottom (`--bar`), and the preview
//! on any side (`--preview-window`, `rotate-preview`).

use super::*;
use crate::lvim::Look;
use crate::preview::Pos;

/// A clickable footer button: its columns, row, and the actions it runs.
pub(super) struct Button {
    pub x0: u16,
    pub x1: u16,
    pub y: u16,
    pub actions: Vec<Action>,
}

/// Vim-like notation for a key name, as lvim-picker shows it.
fn key_label(k: &str) -> String {
    match k {
        "enter" => "<CR>".into(),
        "esc" => "Esc".into(),
        "tab" => "Tab".into(),
        "shift-tab" => "S-Tab".into(),
        "bspace" => "BS".into(),
        "space" => "Space".into(),
        "up" => "↑".into(),
        "down" => "↓".into(),
        "left" => "←".into(),
        "right" => "→".into(),
        "page-up" => "PgUp".into(),
        "page-down" => "PgDn".into(),
        _ => {
            let mut s = k.to_string();
            for (a, b) in [("ctrl-", "C-"), ("alt-", "M-"), ("shift-", "S-")] {
                s = s.replace(a, b);
            }
            s = s.replace("S-up", "S-↑").replace("S-down", "S-↓");
            if s.len() == 2 && s.starts_with('f') {
                return s.to_uppercase();
            }
            if let Some(n) = s.strip_prefix('f')
                && n.parse::<u8>().is_ok()
            {
                return format!("F{n}");
            }
            s
        }
    }
}

impl App {
    /// Keys bound to `action` (exactly that one action), best first.
    fn keys_for(&self, action: &str) -> Vec<String> {
        let mut v: Vec<String> = self
            .keymap
            .iter()
            .filter(|(k, a)| {
                a.len() == 1
                    && a[0].name == action
                    && a[0].arg.is_none()
                    && !is_event(k)
                    && !k.contains("scroll")
                    && !k.contains("click")
            })
            .map(|(k, _)| k.clone())
            .collect();
        // prefer enter (the main key of whatever it does), then ctrl-chords,
        // then named keys, then the rest
        v.sort_by_key(|k| {
            let rank = if k == "enter" {
                -1
            } else if k.starts_with("ctrl-") && k.len() == 6 {
                0
            } else if k == "enter" || k == "esc" || k == "tab" || k.starts_with('f') {
                1
            } else {
                2
            };
            (rank, k.clone())
        });
        v
    }

    /// The footer buttons: (key label, label, actions), in groups.
    fn footer_groups(&self) -> Vec<Vec<(String, String, Vec<Action>)>> {
        let act = |n: &str| {
            vec![Action {
                name: n.to_string(),
                arg: None,
            }]
        };
        let one = |a: &str, label: &str| -> Option<(String, String, Vec<Action>)> {
            self.keys_for(a)
                .first()
                .map(|k| (key_label(k), label.to_string(), act(a)))
        };
        let pair = |a: &str, b: &str, label: &str| -> Option<(String, String, Vec<Action>)> {
            let ka = self.keys_for(a);
            let kb = self.keys_for(b);
            let (x, y) = (ka.first()?, kb.first()?);
            let (lx, ly) = (key_label(x), key_label(y));
            // C-j / C-k → C-j/k
            let text = match (lx.rsplit_once('-'), ly.rsplit_once('-')) {
                (Some((p1, s1)), Some((p2, s2))) if p1 == p2 => format!("{p1}-{s1}/{s2}"),
                _ => format!("{lx}/{ly}"),
            };
            Some((text, label.to_string(), act(a)))
        };
        let mut g1 = Vec::new();
        // enter prints the choice (fzf's accept); `open` puts it in the editor
        g1.extend(one("open", "open"));
        g1.extend(one("accept", "print"));
        let mut g2 = Vec::new();
        g2.extend(pair("down", "up", "move"));
        if self.opts.multi.is_some() {
            g2.extend(one("toggle+down", "mark"));
        }
        // shown by configuration, not by what the current item happens to be
        // previewed with: buttons that come and go while typing make the bar flicker
        if self.preview_enabled() {
            g2.extend(pair("preview-down", "preview-up", "preview"));
            if self.pv.order.contains(&pview::Provider::Nvim) && self.pv.oneoff.is_none() {
                g2.extend(one("toggle-preview-focus", "focus"));
            }
        }
        let live = self.is_live();
        if live || self.live_stash.is_some() {
            g2.extend(one(
                "toggle-grep-filter",
                if live { "grep" } else { "filter" },
            ));
        }
        // the scope picks what the FILTER reads; a live grep has no filter
        if (self.grep_input || self.opts.scope.is_some()) && !live {
            let label = format!("filter: {}", super::actions::scope_label(self.scope));
            g2.extend(one("toggle-scope", &label));
        }
        let mut g3 = Vec::new();
        g3.extend(one("mode-menu", "modes"));
        g3.extend(one("help", "help"));
        g3.extend(one("abort", "close"));
        vec![g1, g2, g3]
            .into_iter()
            .filter(|g| !g.is_empty())
            .collect()
    }

    fn lvim_title(&self) -> String {
        let t = self
            .opts
            .title
            .clone()
            .unwrap_or_else(|| self.default_title.clone());
        let q = self.query_string();
        let base = if q.is_empty() {
            t.to_uppercase()
        } else {
            format!("{}: {}", t, q).to_uppercase()
        };
        // live grep / grep results: which part the filter reads
        if self.is_live() {
            format!("{base} · grep")
        } else if self.grep_input || self.opts.scope.is_some() {
            format!("{base} · {}", super::actions::scope_label(self.scope))
        } else {
            base
        }
    }

    fn lvim_counter(&self) -> String {
        let total = self.engine.items.read().unwrap().len();
        let spin = if self.loading {
            format!("{} ", SPINNER[self.spinner])
        } else {
            String::new()
        };
        let mut s = if self.query.is_empty() || !self.search {
            format!("{spin}{total}")
        } else {
            format!("{spin}{}/{total}", self.matched_count())
        };
        if !self.selected.is_empty() {
            s.push_str(&format!(" · {} marked", self.selected.len()));
        }
        s
    }

    fn draw_title_bar(&self, cv: &mut Canvas, l: &Look, y: u16, w: u16) {
        cv.fill(Rect { x: 0, y, w, h: 1 }, l.title);
        let counter = if self.hidden("counter") {
            String::new()
        } else {
            format!(" {} ", self.lvim_counter())
        };
        let cw = str_width(&counter) as u16;
        let title = format!(" {} ", self.lvim_title());
        let tw = (str_width(&title) as u16).min(w.saturating_sub(cw + 1));
        let title = truncate(&title, tw as usize);
        let x = match self.opts.title_pos.as_str() {
            "center" => (w.saturating_sub(tw)) / 2,
            "right" => w.saturating_sub(tw + cw + 1),
            _ => 0,
        };
        cv.put(x, y, &title, l.title, w);
        cv.put(w.saturating_sub(cw), y, &counter, l.counter, w);
    }

    fn draw_footer(&mut self, cv: &mut Canvas, l: &Look, y: u16, w: u16) {
        cv.fill(Rect { x: 0, y, w, h: 1 }, l.panel);
        let groups = self.footer_groups();
        // buttons " key " + " label ", one space apart — the way lvim-picker's footer
        // draws them (no separator between the groups)
        let mut spans: Vec<(String, Style, Option<usize>)> = Vec::new(); // text, style, button index
        let mut actions: Vec<Vec<Action>> = Vec::new();
        for (gi, g) in groups.iter().enumerate() {
            if gi > 0 {
                spans.push((" ".into(), l.panel, None));
            }
            for (bi, (k, label, acts)) in g.iter().enumerate() {
                if bi > 0 {
                    spans.push((" ".into(), l.panel, None));
                }
                let idx = actions.len();
                actions.push(acts.clone());
                spans.push((format!(" {k} "), l.footer_key, Some(idx)));
                spans.push((format!(" {label} "), l.footer_label, Some(idx)));
            }
        }
        let total: usize = spans.iter().map(|s| str_width(&s.0)).sum();
        let mut x = if (total as u16) < w {
            (w - total as u16) / 2
        } else {
            0
        };
        let mut over = false;
        self.buttons.clear();
        for (text, st, bi) in spans {
            let sw = str_width(&text) as u16;
            if x + sw > w.saturating_sub(2) && total as u16 >= w {
                over = true;
                break;
            }
            let x0 = x;
            x = cv.put(x, y, &text, st, w);
            if let Some(i) = bi {
                match self.buttons.last_mut() {
                    Some(b) if b.x1 == x0 && b.actions == actions[i] => b.x1 = x,
                    _ => self.buttons.push(Button {
                        x0,
                        x1: x,
                        y,
                        actions: actions[i].clone(),
                    }),
                }
            }
        }
        if over {
            cv.put(w.saturating_sub(2), y, " ❯", l.chevron, w);
        }
    }

    /// Prompt row: the badge, a gap, the input field. Returns the cursor column.
    fn draw_lvim_prompt(&self, cv: &mut Canvas, l: &Look, r: Rect) -> u16 {
        let limit = r.x + r.w;
        let badge = if self.opts.prompt != "> " {
            format!(" {} ", self.opts.prompt.trim_end())
        } else {
            " Search ".to_string()
        };
        let mut x = r.x;
        if !self.hidden("search-label") {
            x = cv.put(x, r.y, &badge, l.prompt, limit);
            x = cv.put(x, r.y, " ", l.panel, limit);
        }
        let field = Rect {
            x,
            y: r.y,
            w: limit.saturating_sub(x),
            h: 1,
        };
        cv.fill(field, l.input);
        let fx = x + 1;
        let room = limit.saturating_sub(fx + 1) as usize;
        let mut skip = 0usize;
        while skip < self.qcur
            && str_width(&self.query[skip..self.qcur].iter().collect::<String>()) >= room.max(1)
        {
            skip += 1;
        }
        if self.query.is_empty() {
            let ghost = if self.ghost.is_empty() {
                "type to search…"
            } else {
                &self.ghost
            };
            cv.put(
                fx,
                r.y,
                &truncate(ghost, room),
                Style {
                    bg: l.input.bg,
                    ..l.count
                },
                limit,
            );
        } else {
            let shown: String = self.query[skip..].iter().collect();
            let st = if self.search {
                l.input
            } else {
                Style {
                    bg: l.input.bg,
                    ..l.count
                }
            };
            cv.put(fx, r.y, &truncate(&shown, room), st, limit);
        }
        fx + str_width(&self.query[skip..self.qcur].iter().collect::<String>()) as u16
    }

    fn draw_lvim_rows(&mut self, cv: &mut Canvas, l: &Look, r: Rect) {
        let rows = r.h as usize;
        self.list_rows = rows;
        let len = self.hits.len();
        let so = self.opts.scroll_off.min(rows.saturating_sub(1) / 2);
        if rows > 0 {
            if self.cursor < self.offset + so {
                self.offset = self.cursor.saturating_sub(so);
            }
            if self.cursor + so + 1 > self.offset + rows {
                self.offset = self.cursor + so + 1 - rows;
            }
            self.offset = self.offset.min(len.saturating_sub(rows));
        }
        if self.opts.wrap && rows > 0 {
            // wrapped items take several lines: scroll until the cursor's item fits
            let items = self.engine.items.read().unwrap();
            let sign_w = str_width(&self.opts.wrap_sign);
            let height = |hit: usize| -> usize {
                let Some(it) = self.hits.get(hit).and_then(|i| items.get(i as usize)) else {
                    return 1;
                };
                let w = canvas::str_width(it.display()) + 3; // marker + icon
                let first = r.w as usize;
                let rest = (r.w as usize).saturating_sub(sign_w).max(1);
                if w <= first {
                    1
                } else {
                    1 + (w - first).div_ceil(rest)
                }
            };
            if self.cursor < self.offset {
                self.offset = self.cursor;
            }
            while self.offset < self.cursor
                && (self.offset..=self.cursor).map(height).sum::<usize>() > rows
            {
                self.offset += 1;
            }
        }
        self.row_map.clear();
        if len == 0 {
            let msg = if self.loading {
                self.opts.loading_text.as_str()
            } else {
                self.opts.empty_text.as_str()
            };
            // where the rows' text starts (after the one-column marker)
            cv.put(r.x + 1, r.y, msg, l.count, r.x + r.w);
            return;
        }
        // the query of the rows on screen (not what is typed meanwhile)
        let hq = self.highlight_query();
        // a live source (grep): the finder does not search, rg does; its
        // pattern still marks the rows
        let pat = ((self.search || self.is_live()) && !hq.is_empty())
            .then(|| Pattern::parse(&hq, &crate::match_settings(&self.opts).cfg));
        let items = self.engine.items.read().unwrap();
        let limit = r.x + r.w;
        let bottom = r.y + r.h;
        let mut y = r.y;
        let mut k = 0usize;
        while y < bottom {
            let hit = self.offset + k;
            k += 1;
            let Some(idx) = self.hits.get(hit) else { break };
            let Some(it) = items.get(idx as usize) else {
                continue;
            };
            let k = k - 1;
            self.row_map.push((y, hit));
            let current = hit == self.cursor;
            let stripe = hit % 2;
            // raw mode: an item that does not match is dimmed
            let dim = self.raw_set.as_ref().is_some_and(|set| !set.contains(&idx));
            let base = if current {
                l.sel[stripe]
            } else {
                l.rows[stripe]
            };
            let base = if dim {
                Style {
                    fg: l.count.fg,
                    ..base
                }
            } else {
                base
            };
            cv.fill(
                Rect {
                    x: r.x,
                    y,
                    w: r.w,
                    h: 1,
                },
                base,
            );
            let mut x = r.x;
            // marker (in jump mode: the row's label)
            if self.jump.is_some() {
                let label: String = self
                    .opts
                    .jump_labels
                    .chars()
                    .nth(k)
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| " ".into());
                x = cv.put(
                    x,
                    y,
                    &label,
                    Style {
                        bg: base.bg,
                        ..l.marker
                    },
                    limit,
                );
            } else if self.selset.contains(&idx) {
                x = cv.put(
                    x,
                    y,
                    self.opts.marker.as_deref().unwrap_or("➤"),
                    Style {
                        bg: base.bg,
                        ..l.marker
                    },
                    limit,
                );
            } else {
                x = cv.put(x, y, " ", base, limit);
            }
            // file icon when the item names a file
            let disp = it.display();
            let path_part = disp.split(':').next().unwrap_or(disp);
            let p = std::path::Path::new(path_part);
            if !path_part.is_empty()
                && p.exists()
                && let Some((ic, accent)) = crate::lvim::icon_for(&self.opts, path_part, p.is_dir())
            {
                let fg = l.theme.color(accent).or(base.fg);
                x = cv.put(
                    x,
                    y,
                    ic,
                    Style {
                        fg,
                        bg: base.bg,
                        ..base
                    },
                    limit,
                );
                x = cv.put(x, y, " ", base, limit);
            }
            // text, scrolled so the last match is visible
            let hl = match &pat {
                Some(p) if !dim => self.row_highlights(p, it),
                _ => Vec::new(),
            };
            let chars: Vec<char> = disp
                .chars()
                .map(|c| if c == '\t' { ' ' } else { c })
                .collect();
            let room = limit.saturating_sub(x) as usize;
            let widths: Vec<usize> = chars.iter().map(|c| canvas::char_width(*c)).collect();
            let total: usize = widths.iter().sum();
            // A grep result's LOCATOR (path:line:col:) is never cut — it is what
            // identifies the hit; only the text after it scrolls to the match.
            let lock = if self.opts.freeze_left > 0 {
                // --freeze-left N: the first N fields never scroll away
                let toks = self.opts.delimiter.tokens(disp);
                toks.get(self.opts.freeze_left.min(toks.len()).saturating_sub(1))
                    .map(|t| disp[..t.1].chars().count())
                    .unwrap_or(0)
            } else {
                it.grep
                    .map(|(_, ts)| disp[..ts as usize].chars().count())
                    .unwrap_or(0)
            }
            .min(chars.len());
            let lock_w: usize = widths[..lock].iter().sum();
            let mut start = lock;
            let clip = "…";
            if !self.opts.wrap
                && total > room
                && lock_w < room
                && let Some(&last) = hl.iter().rev().find(|&&p| p >= lock)
            {
                let avail = room - lock_w;
                let end_col: usize = widths[lock..=last.min(widths.len() - 1)]
                    .iter()
                    .sum::<usize>()
                    + self.opts.hscroll_off;
                let mut col = 0;
                while start < chars.len() && end_col.saturating_sub(col) > avail.saturating_sub(1) {
                    col += widths[start];
                    start += 1;
                }
            }
            let mut buf = [0u8; 4];
            let mut hi = hl.iter().peekable();
            for (ci, ch) in chars.iter().enumerate() {
                while hi.peek().is_some_and(|&&p| p < ci) {
                    hi.next();
                }
                let is_hl = hi.peek().is_some_and(|&&p| p == ci);
                if ci == start && start > lock {
                    x = cv.put(
                        x,
                        y,
                        clip,
                        Style {
                            bg: base.bg,
                            ..l.count
                        },
                        limit,
                    );
                }
                if *ch == '\n' && self.opts.multi_line && start == lock && y + 1 < bottom {
                    // a multi-line item (--read0): the rest on the next line
                    y += 1;
                    self.row_map.push((y, hit));
                    cv.fill(
                        Rect {
                            x: r.x,
                            y,
                            w: r.w,
                            h: 1,
                        },
                        base,
                    );
                    x = r.x + 2;
                    continue;
                }
                if (ci >= lock && ci < start) || ch.is_control() {
                    continue;
                }
                let st = if is_hl {
                    Style {
                        bg: base.bg,
                        ..l.matched
                    }
                } else {
                    base
                };
                if x + widths[ci] as u16 > limit {
                    if !self.opts.wrap || y + 1 >= bottom {
                        break;
                    }
                    // continue on the next line, after the wrap sign
                    y += 1;
                    self.row_map.push((y, hit));
                    cv.fill(
                        Rect {
                            x: r.x,
                            y,
                            w: r.w,
                            h: 1,
                        },
                        base,
                    );
                    x = cv.put(
                        r.x,
                        y,
                        &self.opts.wrap_sign,
                        Style {
                            bg: base.bg,
                            ..l.count
                        },
                        limit,
                    );
                }
                x = cv.put(x, y, ch.encode_utf8(&mut buf), st, limit);
            }
            y += 1;
        }
    }

    /// The preview's top bar: icon · file name · its directory, on the bar fill.
    fn draw_winbar(&self, cv: &mut Canvas, l: &Look, r: Rect) {
        // with no matches there is nothing to name: no bar either
        if self.hits.len() == 0 {
            return;
        }
        cv.fill(
            Rect {
                x: r.x,
                y: r.y,
                w: r.w,
                h: 1,
            },
            l.file_bar,
        );
        let limit = r.x + r.w;
        let target = match self.pv.active {
            Some(pview::Provider::File | pview::Provider::User(_)) => {
                self.pv.file_target.as_ref().map(|t| t.0.clone())
            }
            Some(pview::Provider::Nvim) => Some(self.pv.label.clone()).filter(|s| !s.is_empty()),
            _ => None,
        };
        // one look for every case: a label that names a file is drawn as a
        // file; with no matches the bar stays empty (no stale name)
        let target = target.or_else(|| {
            let label = &self.pv.label;
            (self.hits.len() > 0 && !label.is_empty() && std::path::Path::new(label).exists())
                .then(|| label.clone())
        });
        let Some(path) = target else {
            if self.hits.len() > 0 && !self.pv.label.is_empty() {
                cv.put(
                    r.x,
                    r.y,
                    &format!(" {} ", self.pv.label),
                    l.file_name,
                    limit,
                );
            }
            return;
        };
        let p = std::path::Path::new(&path);
        let name = p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or(path.clone());
        let dir = p
            .parent()
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (ic, accent) = crate::lvim::icon_for(&self.opts, &path, p.is_dir()).unwrap_or(("", ""));
        let icon_st = Style {
            fg: l.theme.color(accent).or(l.file_icon.fg),
            ..l.file_icon
        };
        let mut x = cv.put(r.x, r.y, &format!(" {ic}"), icon_st, limit);
        x = cv.put(x, r.y, &format!(" {name} "), l.file_name, limit);
        if !dir.is_empty() && dir != "." {
            let d = dir.trim_start_matches("./");
            cv.put(x, r.y, &format!(" {d}/ "), l.file_dir, limit);
        }
    }

    pub(super) fn draw_lvim(&mut self, term: &mut Term) -> std::io::Result<()> {
        let l = self.look.take().expect("lvim look");
        // --margin: the finder is drawn into the inner frame, then placed
        let frame = super::view::inset(
            Rect {
                x: 0,
                y: 0,
                w: term.width,
                h: term.height,
            },
            &self.opts.margin,
        );
        let (w, h) = (frame.w, frame.h);
        self.term_w = w;
        self.term_h = h;
        let mut cv = Canvas::new(w, h);
        cv.base_bg = l.panel.bg;
        cv.base_fg = l.panel.fg;
        cv.clear();
        let mut cursor_pos = None;
        if w >= 10 && h >= 3 {
            let bar_top = self.opts.bar == "top";
            // the title bar and the button bar each take a row, unless hidden
            let show_title = !self.hidden("title");
            let show_footer = !self.hidden("footer");
            let mut row = 0u16;
            let mut bar_y = None;
            if bar_top && show_footer {
                bar_y = Some(row);
                row += 1;
            }
            if show_title {
                self.draw_title_bar(&mut cv, &l, row, w);
                row += 1;
            }
            if !bar_top && show_footer {
                bar_y = Some(h - 1);
            }
            // panels: between the title and the bar, inset by the blank content ring
            let top = row;
            let bottom = if !bar_top && show_footer { h - 1 } else { h };
            let mut area = Rect {
                x: 0,
                y: top,
                w,
                h: bottom.saturating_sub(top),
            };
            let preview = self.layout_preview(&mut area);
            // separator between the list and the preview
            if let Some(pr) = preview
                && !self.hidden("separator")
            {
                let spec = self.pv.spec.resolve(w, h).clone();
                match spec.pos {
                    Pos::Right | Pos::Left => {
                        let x = if spec.pos == Pos::Right {
                            pr.x
                        } else {
                            pr.x + pr.w - 1
                        };
                        for y in area.y..area.y + area.h {
                            cv.put(x, y, "│", l.border, x + 1);
                        }
                    }
                    Pos::Up | Pos::Down => {
                        let y = if spec.pos == Pos::Down {
                            pr.y
                        } else {
                            pr.y + pr.h - 1
                        };
                        cv.put(
                            area.x,
                            y,
                            &"─".repeat(area.w as usize),
                            l.border,
                            area.x + area.w,
                        );
                    }
                }
            }
            if let Some(pr) = preview {
                let spec = self.pv.spec.resolve(w, h).clone();
                // the preview panel without its separator cell, inset by the ring
                let inner = match spec.pos {
                    Pos::Right => Rect {
                        x: pr.x + 2,
                        y: pr.y + 1,
                        w: pr.w.saturating_sub(3),
                        h: pr.h.saturating_sub(2),
                    },
                    Pos::Left => Rect {
                        x: pr.x + 1,
                        y: pr.y + 1,
                        w: pr.w.saturating_sub(3),
                        h: pr.h.saturating_sub(2),
                    },
                    Pos::Down => Rect {
                        x: pr.x + 1,
                        y: pr.y + 2,
                        w: pr.w.saturating_sub(2),
                        h: pr.h.saturating_sub(3),
                    },
                    Pos::Up => Rect {
                        x: pr.x + 1,
                        y: pr.y + 1,
                        w: pr.w.saturating_sub(2),
                        h: pr.h.saturating_sub(3),
                    },
                };
                if inner.h >= 2 && inner.w >= 4 {
                    let titled = !self.hidden("preview-title");
                    if titled {
                        self.draw_winbar(&mut cv, &l, inner);
                    }
                    let skip = titled as u16;
                    let body = Rect {
                        x: inner.x,
                        y: inner.y + skip,
                        w: inner.w,
                        h: inner.h - skip,
                    };
                    self.draw_preview(&mut cv, body);
                    if self.pv.focus && self.nvim_mode() {
                        cursor_pos = self.nvim_cursor;
                    }
                }
            }
            let list = Rect {
                x: area.x + 1,
                y: area.y + 1,
                w: area.w.saturating_sub(2),
                h: area.h.saturating_sub(2),
            };
            if list.h >= 1 && list.w >= 4 {
                let cx = self.draw_lvim_prompt(&mut cv, &l, Rect { h: 1, ..list });
                if !(self.pv.focus && self.nvim_mode()) {
                    cursor_pos = Some((cx, list.y));
                }
                // header lines (--header / --header-lines) under the prompt
                let mut top = list.y + 1;
                if !self.header_hidden {
                    let headers: Vec<String> = self
                        .header_items
                        .iter()
                        .chain(self.header.iter())
                        .cloned()
                        .collect();
                    let room = list.h.saturating_sub(2) as usize;
                    for line in headers.iter().take(room) {
                        let text: String = line
                            .chars()
                            .map(|c| if c == '\t' { ' ' } else { c })
                            .collect();
                        cv.put(
                            list.x + 2,
                            top,
                            &text,
                            Style {
                                attrs: l.title.attrs,
                                ..l.count
                            },
                            list.x + list.w,
                        );
                        top += 1;
                    }
                }
                let rows = Rect {
                    y: top,
                    h: (list.y + list.h).saturating_sub(top),
                    ..list
                };
                self.draw_lvim_rows(&mut cv, &l, rows);
            }
            if let Some(y) = bar_y {
                self.draw_footer(&mut cv, &l, y, w);
            }
            let full = Rect { x: 0, y: 0, w, h };
            self.draw_toast(
                &mut cv,
                Rect {
                    y: top,
                    h: area.h,
                    ..full
                },
            );
            if self.help.is_some() {
                self.draw_help(&mut cv, full);
                cursor_pos = None;
            }
            if self.menu.is_some() {
                self.draw_menu(&mut cv, full, Some(&l));
                cursor_pos = None;
            }
            if self.theme_menu.is_some() {
                self.draw_theme_menu(&mut cv, full, Some(&l));
                cursor_pos = None;
            }
        }
        let (fx, fy) = (frame.x, frame.y);
        let (cv, cursor_pos) = if fx == 0 && fy == 0 && w == term.width && h == term.height {
            (cv, cursor_pos)
        } else {
            let mut full = Canvas::new(term.width, term.height);
            full.base_bg = l.panel.bg;
            full.base_fg = l.panel.fg;
            full.clear();
            full.blit(&cv, fx, fy);
            // what was recorded for clicks and the preview is in frame coordinates
            for (y, _) in self.row_map.iter_mut() {
                *y += fy;
            }
            for b in self.buttons.iter_mut() {
                b.x0 += fx;
                b.x1 += fx;
                b.y += fy;
            }
            for r in [&mut self.pv.rect, &mut self.pv.inner]
                .into_iter()
                .flatten()
            {
                r.x += fx;
                r.y += fy;
            }
            (full, cursor_pos.map(|(x, y)| (x + fx, y + fy)))
        };
        self.look = Some(l);
        self.flush(term, cv, cursor_pos)
    }

    pub(super) fn click_footer(&mut self, col: u16, row: u16) -> Option<Vec<Action>> {
        self.buttons
            .iter()
            .find(|b| b.y == row && col >= b.x0 && col < b.x1)
            .map(|b| b.actions.clone())
    }
}

fn is_event(k: &str) -> bool {
    matches!(
        k,
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
}
