//! Preview window: layout, running the command, drawing and scrolling.

use super::*;
use crate::preview::{Pos, Spec, scroll_offset};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Provider {
    Nvim,
    File,
    Command,
    /// A user previewer from the config (index into crate::previewers()).
    User(usize),
}

impl Provider {
    pub(crate) fn parse(s: &str) -> Result<Provider, String> {
        match s.trim() {
            "nvim" | "neovim" => Ok(Provider::Nvim),
            "file" | "builtin" => Ok(Provider::File),
            "command" | "cmd" => Ok(Provider::Command),
            o => match crate::previewers().iter().position(|p| p.name == o) {
                Some(i) => Ok(Provider::User(i)),
                None => Err(format!(
                    "unknown preview provider: {o} (nvim, file, command, or a [previewers.<name>] from the config)"
                )),
            },
        }
    }
}

impl App {
    fn preview_spec(&self, w: u16, h: u16) -> Spec {
        self.pv.spec.resolve(w, h).clone()
    }

    pub(super) fn preview_enabled(&self) -> bool {
        (!self.pv.order.is_empty() || self.pv.oneoff.is_some()) && self.pv.visible
    }

    /// The embedded Neovim shows the current item.
    pub(super) fn nvim_mode(&self) -> bool {
        self.pv.active == Some(Provider::Nvim) && self.pv.oneoff.is_none()
    }

    fn ensure_nvim(&mut self) -> bool {
        if self.pv.nvim.is_some() {
            return true;
        }
        let appname = self.opts.nvim_appname.clone();
        match crate::nvim::Nvim::start(&self.opts.nvim, appname.as_deref(), self.pv.wake.clone()) {
            Ok(n) => {
                self.pv.nvim = Some(n);
                true
            }
            Err(e) => {
                self.pv.nvim_failed = true;
                self.toast(
                    format!(
                        "cannot start Neovim ({}): {e}; using the normal preview",
                        self.opts.nvim
                    ),
                    true,
                );
                false
            }
        }
    }

    /// The item's preview target (`file[:line[:col]]`), placeholders expanded raw.
    pub(super) fn target(&mut self, template: &str) -> (String, usize, usize) {
        self.target_for_opt(self.current_index(), template)
    }

    /// `target` for item `idx` (a selected one, not the focused).
    pub(super) fn target_for(&mut self, idx: u32, template: &str) -> (String, usize, usize) {
        self.target_for_opt(Some(idx), template)
    }

    fn target_for_opt(&mut self, idx: Option<u32>, template: &str) -> (String, usize, usize) {
        let raw_tpl = raw_placeholders(template);
        let t = self.expand_for(idx, &raw_tpl);
        let (path, line, col) = crate::nvim::parse_target(&t);
        let path = if path.is_empty() {
            path
        } else {
            expand_home(&path)
        };
        (path, line, col)
    }

    /// Show the current item in the embedded Neovim.
    fn update_nvim(&mut self, idx: u32) {
        let tpl = self.pv.nvim_template.clone().unwrap_or_else(|| "{}".into());
        let (path, line, col) = self.target(&tpl);
        let text = {
            let items = self.engine.items.read().unwrap();
            items
                .get(idx as usize)
                .map(|it| it.raw().to_string())
                .unwrap_or_default()
        };
        let query = self.highlight_query();
        if let Some(n) = &self.pv.nvim {
            n.open(&path, line, col, &text, &query);
        }
        self.pv.label = if std::path::Path::new(&path).exists() {
            path.clone()
        } else {
            String::new()
        };
    }

    /// The query's literal terms (extended-search markers stripped), lower-cased.
    fn query_terms(&self) -> Vec<Vec<char>> {
        self.highlight_query()
            .split(' ')
            .filter(|t| !t.is_empty() && !t.starts_with('!') && *t != "|")
            .map(|t| {
                t.trim_start_matches(['\'', '^'])
                    .trim_end_matches(['$', '\''])
            })
            .filter(|t| !t.is_empty())
            .map(|t| t.to_lowercase().chars().collect())
            .collect()
    }

    /// The built-in file preview: line numbers, the target line, query matches.
    pub(super) fn draw_file(&mut self, cv: &mut Canvas, inner: Rect) {
        let Some((path, line, col)) = self.pv.file_target.clone() else {
            return;
        };
        let (text_st, nr_st, cur_st, hl_st, focus_st, info_st) = match &self.look {
            Some(l) => (
                l.panel,
                l.line_nr,
                l.cursor_line,
                l.preview_match,
                l.preview_focus,
                l.count,
            ),
            None => {
                let t = &self.theme;
                let base = Style {
                    fg: t.get(Slot::PreviewFg).fg,
                    bg: t.get(Slot::PreviewBg).bg,
                    ..Style::default()
                };
                (
                    base,
                    t.get(Slot::Info),
                    t.get(Slot::CurBg),
                    t.get(Slot::Hl),
                    t.get(Slot::CurHl),
                    t.get(Slot::Info),
                )
            }
        };
        let Some(view) = self.pv.loader.view(&path) else {
            let msg = format!("{} loading…", SPINNER[self.spinner]);
            cv.put(inner.x + 1, inner.y, &msg, info_st, inner.x + inner.w);
            return;
        };
        if let Some(note) = &view.note {
            cv.put(inner.x + 1, inner.y, note, info_st, inner.x + inner.w);
            return;
        }
        let h = inner.h as usize;
        let total = view.lines.len();
        let max_off = total.saturating_sub(h);
        if self.pv.offset == usize::MAX {
            // centre the target line
            self.pv.offset = if line > 0 {
                (line - 1).saturating_sub(h / 2).min(max_off)
            } else {
                0
            };
        }
        let off = self.pv.offset.min(max_off);
        let nr_w = if view.is_dir {
            0
        } else {
            total.max(1).to_string().len() as u16 + 1
        };
        let terms = self.query_terms();
        let tab = self.opts.tabstop;
        let mut buf = [0u8; 4];
        // syntax colours: syntect spans (parsed on the loader thread), palette accents
        let accent_color = |t: crate::syntax::Tok| -> Option<crossterm::style::Color> {
            let (a, _, _) = crate::syntax::accent(t);
            match &self.look {
                Some(l) => l.theme.color(a),
                None => Some(match a {
                    "comment" => crossterm::style::Color::DarkGrey,
                    "green" => crossterm::style::Color::DarkGreen,
                    "orange" => crossterm::style::Color::DarkYellow,
                    "red" => crossterm::style::Color::DarkMagenta,
                    "blue" => crossterm::style::Color::DarkBlue,
                    _ => crossterm::style::Color::DarkCyan,
                }),
            }
        };
        for (i, l) in view.lines.iter().skip(off).take(h).enumerate() {
            let y = inner.y + i as u16;
            let lnum = off + i + 1;
            let is_target = line > 0 && lnum == line;
            if is_target {
                cv.set_bg(inner.x, y, inner.w, cur_st.bg);
            }
            let mut x = inner.x;
            if nr_w > 0 {
                let n = format!("{:>w$} ", lnum, w = nr_w as usize - 1);
                let st = if is_target {
                    Style {
                        bg: cur_st.bg,
                        ..nr_st
                    }
                } else {
                    nr_st
                };
                x = cv.put(x, y, &n, st, inner.x + inner.w);
            }
            // match spans (char indices) of every query term on this line
            let chars: Vec<char> = l.chars().collect();
            let line_spans: &[crate::syntax::Span] =
                view.spans.get(off + i).map(Vec::as_slice).unwrap_or(&[]);
            let tok_at = |ci: usize| -> Option<crate::syntax::Tok> {
                let c = ci as u32;
                line_spans.iter().find(|s| s.0 <= c && c < s.1).map(|s| s.2)
            };
            let lower: Vec<char> = chars
                .iter()
                .map(|c| c.to_lowercase().next().unwrap_or(*c))
                .collect();
            let mut mark = vec![0u8; chars.len()];
            for t in &terms {
                if t.len() > lower.len() {
                    continue;
                }
                for p in 0..=lower.len() - t.len() {
                    if lower[p..p + t.len()] == t[..] {
                        let focus = is_target && col > 0 && p + 1 == col;
                        for m in mark.iter_mut().skip(p).take(t.len()) {
                            *m = if focus { 2 } else { (*m).max(1) };
                        }
                    }
                }
            }
            let mut colx = 0usize;
            for (ci, ch) in chars.iter().enumerate() {
                let mut base = if is_target {
                    Style {
                        bg: cur_st.bg,
                        ..text_st
                    }
                } else {
                    text_st
                };
                if let Some(t) = tok_at(ci)
                    && let Some(c) = accent_color(t)
                {
                    base.fg = Some(c);
                    let (_, italic, bold) = crate::syntax::accent(t);
                    if italic {
                        base.attrs.set(crossterm::style::Attribute::Italic);
                    }
                    if bold {
                        base.attrs.set(crossterm::style::Attribute::Bold);
                    }
                }
                let st = match mark[ci] {
                    2 => focus_st,
                    1 => Style {
                        bg: hl_st.bg.or(base.bg),
                        ..hl_st
                    },
                    _ => base,
                };
                if *ch == '\t' {
                    let n = tab - colx % tab;
                    x = cv.put(x, y, &" ".repeat(n), st, inner.x + inner.w);
                    colx += n;
                    continue;
                }
                if ch.is_control() {
                    continue;
                }
                x = cv.put(x, y, ch.encode_utf8(&mut buf), st, inner.x + inner.w);
                colx += canvas::char_width(*ch);
                if x >= inner.x + inner.w {
                    break;
                }
            }
        }
        if view.truncated && h > 0 {
            let note = " (truncated) ";
            let w = str_width(note) as u16;
            cv.put(
                (inner.x + inner.w).saturating_sub(w),
                inner.y + inner.h - 1,
                note,
                info_st,
                inner.x + inner.w,
            );
        }
    }

    fn draw_nvim(&mut self, cv: &mut Canvas, inner: Rect) {
        let Some(n) = self.pv.nvim.as_mut() else {
            return;
        };
        n.size(inner.w as usize, inner.h as usize);
        let g = n.grid.lock().unwrap();
        if g.exited {
            drop(g);
            self.pv.nvim = None;
            self.pv.nvim_failed = true;
            self.toast("Neovim exited; using the normal preview".into(), true);
            return;
        }
        if !g.ready {
            let msg = format!("{} starting Neovim…", SPINNER[self.spinner]);
            cv.put(
                inner.x + 1,
                inner.y,
                &msg,
                self.theme.get(Slot::Info),
                inner.x + inner.w,
            );
            return;
        }
        let (gw, gh) = (g.w.min(inner.w as usize), g.h.min(inner.h as usize));
        for r in 0..gh {
            let y = inner.y + r as u16;
            for c in 0..gw {
                let cell = &g.cells[r * g.w + c];
                if cell.text.is_empty() {
                    continue; // trailing half of a wide char
                }
                let st = g.style(cell.hl);
                let x = inner.x + c as u16;
                cv.fill(
                    Rect { x, y, w: 1, h: 1 },
                    Style {
                        bg: st.bg,
                        ..Style::default()
                    },
                );
                cv.put(x, y, &cell.text, st, inner.x + inner.w);
            }
        }
        self.nvim_cursor = None;
        if self.pv.focus {
            self.nvim_cursor = Some((inner.x + g.cursor.1 as u16, inner.y + g.cursor.0 as u16));
        }
    }

    /// Keys typed while the preview has focus go to Neovim.
    pub(super) fn nvim_key(&mut self, k: &crossterm::event::KeyEvent) {
        if let (Some(n), Some(keys)) = (&self.pv.nvim, crate::nvim::key_notation(k)) {
            n.input(&keys);
        }
    }

    pub(super) fn nvim_wheel(&mut self, up: bool, row: u16, col: u16) {
        if let (Some(n), Some(r)) = (&self.pv.nvim, self.pv.inner) {
            n.mouse(
                "wheel",
                if up { "up" } else { "down" },
                "",
                row.saturating_sub(r.y) as usize,
                col.saturating_sub(r.x) as usize,
            );
        }
    }

    pub(super) fn quit_nvim(&mut self) {
        if let Some(n) = self.pv.nvim.take() {
            n.quit();
        }
    }

    /// Split the preview off `area`; returns the preview rect and shrinks `area`.
    pub(super) fn layout_preview(&mut self, area: &mut Rect) -> Option<Rect> {
        if !self.preview_enabled() {
            self.pv.rect = None;
            return None;
        }
        let spec = self.preview_spec(area.w, area.h);
        let dim = match spec.pos {
            Pos::Left | Pos::Right => area.w,
            Pos::Up | Pos::Down => area.h,
        };
        let border = spec.border != BorderStyle::None;
        let mut size = if spec.percent {
            (dim as u32 * spec.size as u32 / 100) as u16
        } else {
            // an absolute size counts the content, not the border
            spec.size + if border { 2 } else { 0 }
        };
        // leave at least a few rows/cols for the finder itself
        let min_rest = match spec.pos {
            Pos::Left | Pos::Right => 10,
            Pos::Up | Pos::Down => 3,
        };
        size = size.min(dim.saturating_sub(min_rest));
        if size < 3 {
            self.pv.rect = None;
            return None;
        }
        let r = match spec.pos {
            Pos::Right => {
                area.w -= size;
                Rect {
                    x: area.x + area.w,
                    y: area.y,
                    w: size,
                    h: area.h,
                }
            }
            Pos::Left => {
                let r = Rect {
                    x: area.x,
                    y: area.y,
                    w: size,
                    h: area.h,
                };
                area.x += size;
                area.w -= size;
                r
            }
            Pos::Up => {
                let r = Rect {
                    x: area.x,
                    y: area.y,
                    w: area.w,
                    h: size,
                };
                area.y += size;
                area.h -= size;
                r
            }
            Pos::Down => {
                area.h -= size;
                Rect {
                    x: area.x,
                    y: area.y + area.h,
                    w: area.w,
                    h: size,
                }
            }
        };
        self.pv.rect = Some(r);
        Some(r)
    }

    /// Start the preview command when its expanded form changed.
    pub(super) fn update_preview(&mut self) {
        if !self.preview_enabled() || self.pv.rect.is_none() {
            return;
        }
        let text = self
            .current_index()
            .and_then(|i| {
                let items = self.engine.items.read().unwrap();
                items.get(i as usize).map(|it| it.raw().to_string())
            })
            .unwrap_or_default();
        let key = (
            self.current_index(),
            text,
            self.query_string(),
            self.selected.len(),
        );
        if !self.pv.force && self.pv.key.as_ref() == Some(&key) {
            return;
        }
        let force = std::mem::take(&mut self.pv.force);
        self.pv.key = Some(key);
        // a one-off preview(...) command wins over the chain
        if self.pv.oneoff.is_some() {
            self.pv.active = Some(Provider::Command);
            self.update_command(force);
            return;
        }
        let Some(idx) = self.current_index() else {
            self.pv.active = None;
            return;
        };
        // the first provider in the chain that can show this item
        let order = self.pv.order.clone();
        self.pv.active = None;
        for p in order {
            match p {
                Provider::Nvim => {
                    // Neovim previews FILES; anything else (a commit, a process) falls through
                    let tpl = self.pv.nvim_template.clone().unwrap_or_else(|| "{}".into());
                    let (path, _, _) = self.target(&tpl);
                    if path.is_empty() || !std::path::Path::new(&path).exists() {
                        continue;
                    }
                    if self.pv.nvim_failed || !self.ensure_nvim() {
                        continue;
                    }
                    self.pv.active = Some(p);
                    self.update_nvim(idx);
                    return;
                }
                Provider::File => {
                    let tpl = self.pv.file_template.clone();
                    let (path, line, col) = self.target(&tpl);
                    if path.is_empty() || !std::path::Path::new(&path).exists() {
                        continue;
                    }
                    self.pv.active = Some(p);
                    self.pv.file_target = Some((path, line, col));
                    self.pv.offset = usize::MAX; // centre on the target when drawn
                    return;
                }
                Provider::Command => {
                    if self.pv.template.is_none() {
                        continue;
                    }
                    self.pv.active = Some(p);
                    self.update_command(force);
                    return;
                }
                Provider::User(i) => {
                    let Some(pr) = crate::previewers().get(i) else {
                        continue;
                    };
                    let tpl = self.pv.file_template.clone();
                    let (path, line, col) = self.target(&tpl);
                    if !pr.applies(&path) {
                        continue;
                    }
                    let cmd = pr.command_for(&path, line, col);
                    self.pv.active = Some(p);
                    self.pv.file_target = (!path.is_empty()).then(|| (path.clone(), line, col));
                    self.run_command(&cmd, force);
                    return;
                }
            }
        }
    }

    fn update_command(&mut self, force: bool) {
        let template = match self.pv.oneoff.clone().or_else(|| self.pv.template.clone()) {
            Some(t) => t,
            None => return,
        };
        self.run_command(&template, force);
    }

    /// Run a preview command template (fzf placeholders) in the background.
    fn run_command(&mut self, template: &str, force: bool) {
        let template = template.to_string();
        if self.current_index().is_none() && template.contains("{}") {
            // nothing to preview
            self.pv.last_cmd = None;
            self.pv.version += 1;
            let mut o = self.pv.runner.as_ref().unwrap().output.lock().unwrap();
            o.version = self.pv.version;
            o.lines.clear();
            o.done = true;
            return;
        }
        let cmd = self.expand(&template);
        if !force && self.pv.last_cmd.as_deref() == Some(cmd.as_str()) {
            return;
        }
        self.pv.last_cmd = Some(cmd.clone());
        self.pv.version += 1;
        let r = self.pv.rect.unwrap();
        let (cols, lines) = self.pv.size;
        let env = vec![
            ("LVIM_FUZZY_PREVIEW_LINES".to_string(), lines.to_string()),
            ("LVIM_FUZZY_PREVIEW_COLUMNS".to_string(), cols.to_string()),
            ("LVIM_FUZZY_PREVIEW_TOP".to_string(), r.y.to_string()),
            ("LVIM_FUZZY_PREVIEW_LEFT".to_string(), r.x.to_string()),
            ("LVIM_FUZZY_QUERY".to_string(), self.query_string()),
            ("LVIM_FUZZY_POS".to_string(), (self.cursor + 1).to_string()),
            ("LVIM_FUZZY".to_string(), "1".to_string()),
        ];
        // initial scroll position
        let spec = self.preview_spec(self.term_w, self.term_h);
        self.pv.offset = match &spec.scroll {
            Some(expr) => {
                let e = self.expand(expr);
                scroll_offset(&e, lines as usize)
            }
            None => 0,
        };
        self.pv
            .runner
            .as_ref()
            .unwrap()
            .run(self.pv.version, cmd, env);
    }

    pub(super) fn draw_preview(&mut self, cv: &mut Canvas, r: Rect) {
        let spec = self.preview_spec(self.term_w, self.term_h);
        let th = &self.theme;
        let pfg = th.get(Slot::PreviewFg);
        let pbg = th.get(Slot::PreviewBg);
        let base = Style {
            fg: pfg.fg,
            bg: pbg.bg,
            ..Style::default()
        };
        cv.fill(r, base);
        let sides = super::border_sides(spec.border);
        if spec.border != BorderStyle::None {
            // a focused Neovim preview gets the pointer colour on its border
            let slot = if self.pv.focus && self.nvim_mode() {
                Slot::Pointer
            } else {
                Slot::PreviewBorder
            };
            let bst = Style {
                bg: pbg.bg,
                ..th.get(slot)
            };
            cv.border(
                r,
                &super::border_chars(spec.border, self.opts.unicode),
                sides,
                bst,
            );
        }
        let [t, rt, b, l] = sides;
        let inner = Rect {
            x: r.x + l as u16 + l as u16,
            y: r.y + t as u16,
            w: r.w.saturating_sub((l as u16 + rt as u16) * 2),
            h: r.h.saturating_sub(t as u16 + b as u16),
        };
        self.pv.size = (inner.w, inner.h);
        if inner.w == 0 || inner.h == 0 {
            return;
        }
        self.pv.inner = Some(inner);
        if self.nvim_mode() {
            self.draw_nvim(cv, inner);
            return;
        }
        if self.pv.active == Some(Provider::File) {
            self.draw_file(cv, inner);
            return;
        }
        if self.pv.active.is_none() && self.pv.oneoff.is_none() {
            let st = self
                .look
                .as_ref()
                .map(|l| l.count)
                .unwrap_or_else(|| self.theme.get(Slot::Info));
            cv.put(
                inner.x + 1,
                inner.y,
                "Nothing to preview",
                st,
                inner.x + inner.w,
            );
            return;
        }
        let out = self.pv.runner.as_ref().unwrap().output.clone();
        let o = out.lock().unwrap();
        let loading = !o.done;
        // render lines (with --preview-window wrap) into display rows
        let width = inner.w as usize;
        let mut rows: Vec<Vec<(char, Style)>> = Vec::new();
        let mut carry = Style::default();
        let wrap = spec.wrap;
        for line in o.lines.iter() {
            let (text, runs, end) = crate::ansi::parse(line, carry);
            carry = end;
            let mut row: Vec<(char, Style)> = Vec::new();
            let mut col = 0usize;
            for (bi, ch) in text.char_indices() {
                let st = crate::ansi::style_at(&runs, bi);
                let st = Style {
                    fg: st.fg.or(base.fg),
                    bg: st.bg.or(base.bg),
                    ..st
                };
                let chs: Vec<char> = if ch == '\t' {
                    vec![' '; self.opts.tabstop - col % self.opts.tabstop]
                } else if ch.is_control() {
                    continue;
                } else {
                    vec![ch]
                };
                for c in chs {
                    let cw = canvas::char_width(c);
                    if col + cw > width {
                        if !wrap {
                            break;
                        }
                        rows.push(std::mem::take(&mut row));
                        col = 0;
                        // --preview-wrap-sign starts the continuation line
                        for sc in self.opts.preview_wrap_sign.chars() {
                            row.push((
                                sc,
                                Style {
                                    fg: th.get(Slot::Info).fg,
                                    ..base
                                },
                            ));
                            col += canvas::char_width(sc);
                        }
                    }
                    row.push((c, st));
                    col += cw;
                }
            }
            rows.push(row);
            if !wrap && rows.len() > 200_000 {
                break;
            }
        }
        drop(o);
        let hdr = spec.header_lines.min(rows.len()).min(inner.h as usize);
        let body_h = inner.h as usize - hdr;
        let body_total = rows.len() - hdr;
        let max_off = body_total.saturating_sub(body_h);
        if spec.follow && !rows.is_empty() {
            self.pv.offset = max_off;
        }
        let off = self.pv.offset.min(max_off);
        let mut buf = [0u8; 4];
        let draw_row = |cv: &mut Canvas, y: u16, row: &Vec<(char, Style)>| {
            let mut x = inner.x;
            for (c, st) in row {
                x = cv.put(
                    x,
                    y,
                    c.encode_utf8(&mut buf.clone()),
                    *st,
                    inner.x + inner.w,
                );
            }
        };
        for (i, row) in rows.iter().take(hdr).enumerate() {
            draw_row(cv, inner.y + i as u16, row);
        }
        for (i, row) in rows.iter().skip(hdr + off).take(body_h).enumerate() {
            draw_row(cv, inner.y + (hdr + i) as u16, row);
        }
        let _ = &mut buf;
        // info: position / total in the top-right corner of the border
        if spec.info && (body_total > body_h || loading) {
            let spin = if loading {
                format!("{} ", SPINNER[self.spinner])
            } else {
                String::new()
            };
            let info = format!(" {spin}{}/{} ", off + 1, body_total.max(1));
            let iw = str_width(&info) as u16;
            let y = if t { r.y } else { inner.y };
            let x = (r.x + r.w).saturating_sub(iw + if rt { 2 } else { 0 });
            cv.put(
                x,
                y,
                &info,
                Style {
                    bg: pbg.bg,
                    ..th.get(Slot::Info)
                },
                r.x + r.w,
            );
        }
        // scrollbar
        if body_total > body_h && body_h > 0 && self.opts.scrollbar.as_deref() != Some("") {
            let bar = ((body_h * body_h) / body_total).max(1);
            let pos = if max_off == 0 {
                0
            } else {
                off * (body_h - bar) / max_off
            };
            let x = if rt {
                r.x + r.w - 1
            } else {
                inner.x + inner.w - 1
            };
            let st = Style {
                bg: pbg.bg,
                ..th.get(Slot::PreviewScrollbar)
            };
            let ch = if self.opts.unicode { "┃" } else { "|" };
            for k in 0..bar {
                cv.put(x, inner.y + (hdr + pos + k) as u16, ch, st, x + 1);
            }
        }
    }

    fn preview_lines_total(&self) -> usize {
        if self.pv.active == Some(Provider::File)
            && let Some((path, _, _)) = &self.pv.file_target
        {
            return self.pv.loader.view(path).map_or(0, |v| v.lines.len());
        }
        self.pv
            .runner
            .as_ref()
            .map_or(0, |r| r.output.lock().unwrap().lines.len())
    }

    /// Preview actions; returns false when `name` is not a preview action.
    pub(super) fn preview_action(&mut self, name: &str, arg: &str, has_arg: bool) -> bool {
        if self.nvim_mode() && self.pv.nvim.is_some() {
            let keys = match name {
                "preview-up" => Some("<C-y>"),
                "preview-down" => Some("<C-e>"),
                "preview-page-up" => Some("<C-b>"),
                "preview-page-down" => Some("<C-f>"),
                "preview-half-page-up" => Some("<C-u>"),
                "preview-half-page-down" => Some("<C-d>"),
                "preview-top" => Some("gg"),
                "preview-bottom" => Some("G"),
                "toggle-preview-wrap" | "toggle-preview-wrap-word" => Some("<Cmd>set wrap!<CR>"),
                _ => None,
            };
            if let Some(k) = keys {
                if let Some(n) = &self.pv.nvim {
                    n.input(k);
                }
                return true;
            }
        }
        if name == "rotate-preview" {
            // right → left → down → up → right (lvim-ui rotates the preview side)
            let next = match self.pv.spec.pos {
                Pos::Right => Pos::Left,
                Pos::Left => Pos::Down,
                Pos::Down => Pos::Up,
                Pos::Up => Pos::Right,
            };
            self.pv.spec.pos = next;
            self.pv.spec.alt = None;
            self.pv.visible = true;
            self.pv.force = true;
            return true;
        }
        if name == "toggle-preview-focus" {
            if self.nvim_mode() {
                self.pv.focus = !self.pv.focus;
            } else {
                self.toast("preview focus needs --preview-nvim".into(), false);
            }
            return true;
        }
        let page = self.pv.size.1.max(1) as usize;
        let total = self.preview_lines_total();
        if self.pv.offset == usize::MAX {
            self.pv.offset = 0;
        }
        match name {
            "toggle-preview" => self.pv.visible = !self.pv.visible,
            "show-preview" => self.pv.visible = true,
            "hide-preview" => self.pv.visible = false,
            "preview-up" => self.pv.offset = self.pv.offset.saturating_sub(1),
            "preview-down" => self.pv.offset = (self.pv.offset + 1).min(total.saturating_sub(1)),
            "preview-page-up" => self.pv.offset = self.pv.offset.saturating_sub(page),
            "preview-page-down" => {
                self.pv.offset = (self.pv.offset + page).min(total.saturating_sub(page))
            }
            "preview-half-page-up" => self.pv.offset = self.pv.offset.saturating_sub(page / 2),
            "preview-half-page-down" => {
                self.pv.offset = (self.pv.offset + page / 2).min(total.saturating_sub(page))
            }
            "preview-top" => self.pv.offset = 0,
            "preview-bottom" => self.pv.offset = total.saturating_sub(page),
            "toggle-preview-wrap" | "toggle-preview-wrap-word" => {
                let w = !self.pv.spec.wrap;
                self.pv.spec.wrap = w;
                if let Some((_, alt)) = self.pv.spec.alt.as_mut() {
                    alt.wrap = w;
                }
            }
            "change-preview" => {
                self.pv.template = has_arg.then(|| arg.to_string()).filter(|s| !s.is_empty());
                self.pv.oneoff = None;
                self.pv.force = true;
            }
            "preview" => {
                self.pv.oneoff = Some(arg.to_string());
                self.pv.visible = true;
                self.pv.force = true;
            }
            "refresh-preview" => self.pv.force = true,
            "change-preview-label" => self.pv.label = arg.to_string(),
            "transform-preview-label" => self.pv.label = self.capture(arg),
            "change-preview-window" => {
                // `a|b|c` cycles through the alternatives on each call
                let alts: Vec<String> = arg.split('|').map(str::to_string).collect();
                if alts != self.pv.cycle {
                    self.pv.cycle = alts;
                    self.pv.cycle_idx = 0;
                } else {
                    self.pv.cycle_idx = (self.pv.cycle_idx + 1) % self.pv.cycle.len().max(1);
                }
                let mut spec = Spec {
                    border: crate::options::BorderStyle::None,
                    ..Spec::default()
                };
                for w in &self.opts.preview_window {
                    let _ = spec.apply(w.split('|').next().unwrap_or(""));
                }
                let chosen = self
                    .pv
                    .cycle
                    .get(self.pv.cycle_idx)
                    .cloned()
                    .unwrap_or_default();
                match spec.apply(&chosen) {
                    Ok(()) => {
                        self.pv.visible = !spec.hidden;
                        self.pv.spec = spec;
                        self.pv.force = true;
                    }
                    Err(e) => self.toast(e, true),
                }
            }
            _ => return false,
        }
        true
    }
}

/// `{x}` -> `{rx}`: expand placeholders without shell quoting.
fn raw_placeholders(t: &str) -> String {
    let mut out = String::new();
    let mut it = t.chars().peekable();
    while let Some(c) = it.next() {
        out.push(c);
        if c == '{' {
            let mut inner = String::new();
            let mut closed = false;
            for d in it.by_ref() {
                if d == '}' {
                    closed = true;
                    break;
                }
                inner.push(d);
            }
            let (plus, rest) = match inner.strip_prefix('+') {
                Some(r) => ("+", r),
                None => ("", inner.as_str()),
            };
            if rest.starts_with('r') {
                out.push_str(&inner);
            } else {
                out.push_str(plus);
                out.push('r');
                out.push_str(rest);
            }
            if closed {
                out.push('}');
            }
        }
    }
    out
}

fn expand_home(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(rest) => std::env::var("HOME")
            .map(|h| format!("{h}/{rest}"))
            .unwrap_or_else(|_| p.to_string()),
        None => p.to_string(),
    }
}
