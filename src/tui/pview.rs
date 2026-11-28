//! Preview window: layout, running the command, drawing and scrolling.

use super::*;
use crate::preview::{Pos, Spec, scroll_offset};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Provider {
    /// The preview plugins (lvim-fuzzy-plugin-preview ...): they choose the engine for the file's type and compute the
    /// answer; lfz draws it.
    Plugin,
    Nvim,
    File,
    Command,
    /// A user previewer from the config (index into crate::previewers()).
    User(usize),
}

/// The next place of the preview in the cycle right → left → up → hidden → right (`dir` 1), or the one before
/// (-1): the same cycle the Neovim picker has. A preview put at the bottom (`--preview-window=down`) is not on
/// the cycle: forward from it goes to hidden, back to up.
pub(crate) fn rotate_side(pos: Pos, visible: bool, dir: i32) -> (Pos, bool) {
    // 0 right, 1 left, 2 up, 3 down (the bottom), 4 hidden
    let at = match (visible, pos) {
        (false, _) => 4,
        (true, Pos::Right) => 0,
        (true, Pos::Left) => 1,
        (true, Pos::Up) => 2,
        (true, Pos::Down) => 3,
    };
    match (at + dir).rem_euclid(5) {
        0 => (Pos::Right, true),
        1 => (Pos::Left, true),
        2 => (Pos::Up, true),
        3 => (Pos::Down, true),
        // hiding keeps the side it was on (`toggle-preview` brings it back there)
        _ => (pos, false),
    }
}

impl Provider {
    pub(crate) fn parse(s: &str) -> Result<Provider, String> {
        match s.trim() {
            "plugin" | "plugins" => Ok(Provider::Plugin),
            "nvim" | "neovim" => Ok(Provider::Nvim),
            "file" | "builtin" => Ok(Provider::File),
            "command" | "cmd" => Ok(Provider::Command),
            o => match crate::previewers().iter().position(|p| p.name == o) {
                Some(i) => Ok(Provider::User(i)),
                None => Err(format!(
                    "unknown preview provider: {o} (plugin, nvim, file, command, or a [previewers.<name>] from the config)"
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
        match crate::nvim::Nvim::start(&self.opts.nvim, appname.as_deref(), self.opts.pool, self.pv.wake.clone()) {
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
        // the instant file preview, shown until Neovim has painted this item
        if !path.is_empty() {
            if self.pv.file_target.as_ref().map(|t| &t.0) != Some(&path) {
                self.pv.offset = usize::MAX;
            }
            self.pv.file_target = Some((path.clone(), line, col));
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

    /// Whether the Neovim grid shows the current item (its open has been painted).
    fn nvim_caught_up(&self) -> bool {
        let (Some(n), Some((path, _, _))) = (self.pv.nvim.as_ref(), self.pv.file_target.as_ref()) else {
            return true; // nothing to compare (a non-file target): Neovim as before
        };
        n.grid.lock().unwrap().shown.as_deref() == Some(path.as_str())
    }

    fn draw_nvim(&mut self, cv: &mut Canvas, inner: Rect) {
        // the scrollbar thumb (the preview's), from where Neovim says its window is in the buffer
        let bar_on = self.opts.scrollbar.as_deref() != Some("") && !self.hidden("scrollbar-right");
        let bar_style = self.preview_bar_style();
        let bar_char = if self.opts.unicode { "┃" } else { "|" };
        let bar_x = self.preview_bar_x(inner);
        let Some(n) = self.pv.nvim.as_mut() else {
            return;
        };
        n.size(inner.w as usize, inner.h as usize);
        let slow = n.started.elapsed() > std::time::Duration::from_millis(300);
        let g = n.grid.lock().unwrap();
        if g.exited {
            drop(g);
            self.pv.nvim = None;
            self.pv.nvim_failed = true;
            self.toast("Neovim exited; using the normal preview".into(), true);
            return;
        }
        if !g.ready {
            // a warm instance paints within tens of ms: an empty pane until then, the note only when
            // Neovim is slow (a cold start) — a note shown for a frame read as a flicker
            if !slow {
                return;
            }
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
        let (top, bot, total) = g.view;
        let shown = bot.saturating_sub(top) + 1;
        if bar_on && top >= 1 && total > shown && gh > 1 && inner.w >= 2 {
            // as long at every place: the rows of the pane over the lines of the file (the lines shown vary where long ones wrap)
            let bar = (gh * gh / total).max(1).min(gh);
            let pos = ((top - 1) * (gh - bar) / (total - shown)).min(gh - bar);
            let x = bar_x;
            for k in 0..bar {
                cv.put(x, inner.y + (pos + k).min(gh - 1) as u16, bar_char, bar_style, x + 1);
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
        self.pv.area = Some(*area);
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
        // the typed query (the command placeholders) AND the one the matches belong to (what the
        // preview marks): the second lags the first while results come in, and keyed on the typed one
        // alone the preview kept the marks of the previous keystroke ("lvim" typed, "lvi" marked)
        let key = (
            self.current_index(),
            text,
            format!("{}\u{1}{}", self.query_string(), self.highlight_query()),
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
        let mut order = self.pv.order.clone();
        // a plugin that answered "show it with Neovim / the built-in highlighter" for this file: that is the
        // provider now (the plugin chose the engine from its configuration; lfz only carries it out)
        if order.contains(&Provider::Plugin) {
            let tpl = self.pv.file_template.clone();
            let ppath = std::path::PathBuf::from(self.target(&tpl).0);
            let redirect = match self.pv.plug_answers.get(&(ppath, self.pv.plug_skip)) {
                Some(lvim_fuzzy_ui::provided::Answer::Preview(p)) if p.missing.is_empty() => match p.renderer.as_deref() {
                    Some("nvim") => Some(Provider::Nvim),
                    Some("builtin") => Some(Provider::File),
                    _ => None,
                },
                _ => None,
            };
            if let Some(to) = redirect {
                order.retain(|x| *x != Provider::Plugin);
                order.insert(0, to);
            }
        }
        self.pv.active = None;
        for p in order {
            match p {
                Provider::Plugin => {
                    if self.look.is_none() || self.pv.plug.is_none() {
                        continue;
                    }
                    let tpl = self.pv.file_template.clone();
                    let path = std::path::PathBuf::from(self.target(&tpl).0);
                    if path.as_os_str().is_empty() {
                        continue;
                    }
                    // another file: its first position, its picture waits a moment
                    if self.pv.list_target.as_ref() != Some(&path) {
                        self.pv.offset = 0;
                        self.pv.plug_skip = 0;
                        self.pv.drawer.moved();
                        self.pv.list_target = Some(path.clone());
                    }
                    let skip = self.pv.plug_skip;
                    match self.pv.plug_answers.get(&(path.clone(), skip)) {
                        // no plugin wants this file: the rest of the chain
                        Some(lvim_fuzzy_ui::provided::Answer::Pass) => continue,
                        Some(_) => {}
                        None => {
                            let asked = self.pv.plug_wait.as_ref().is_some_and(|(_, p, k)| *p == path && *k == skip);
                            if !asked {
                                let (w, h) = self.pv.size;
                                let (pw, ph) = lvim_fuzzy_ui::provided::wanted_px(Rect { x: 0, y: 0, w, h });
                                let extra = serde_json::json!({"cols": w, "rows": h, "host": "lvim-fuzzy-cli", "skip": skip, "max_px": [pw, ph]});
                                let ticket = self.pv.plug.as_mut().and_then(|pl| pl.request(&path, extra));
                                match ticket {
                                    Some(t) => self.pv.plug_wait = Some((t, path.clone(), skip)),
                                    // no provider matches this file at all
                                    None => {
                                        self.pv.plug_answers.insert((path, skip), lvim_fuzzy_ui::provided::Answer::Pass);
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                    self.pv.active = Some(p);
                    self.pv.label = path.to_string_lossy().into_owned();
                    return;
                }
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

    /// The plugins' answers that came in: kept per file, and the preview is chosen again (the answer may say
    /// "Neovim shows this one", or "not mine").
    pub(super) fn poll_plugins(&mut self) {
        let Some(pl) = self.pv.plug.as_mut() else { return };
        let mut changed = false;
        for r in pl.poll() {
            if self.pv.plug_wait.as_ref().is_some_and(|(t, ..)| *t == r.ticket) {
                self.pv.plug_wait = None;
            }
            let skip = r.extra["skip"].as_u64().unwrap_or(0) as u32;
            let answer = r
                .answer
                .and_then(|text| lvim_fuzzy_ui::provided::parse(&text))
                .unwrap_or(lvim_fuzzy_ui::provided::Answer::Pass);
            if self.pv.plug_answers.len() > 256 {
                self.pv.plug_answers.clear();
            }
            self.pv.plug_answers.insert((r.path, skip), answer);
            changed = true;
        }
        if changed {
            self.pv.force = true;
            self.pv.key = None;
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

    /// `look`: the lvim look while it is out of `self` (the lvim view takes it for the frame).
    pub(super) fn draw_preview(&mut self, cv: &mut Canvas, r: Rect, look: Option<&crate::lvim::Look>) {
        self.pv.bar_panel_bg = look.map(|l| l.panel.bg);
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
        let mut inner = Rect {
            x: r.x + l as u16 + l as u16,
            y: r.y + t as u16,
            w: r.w.saturating_sub((l as u16 + rt as u16) * 2),
            h: r.h.saturating_sub(t as u16 + b as u16),
        };
        // the lvim look: the text starts one cell in, under the icon of the file bar above it (as in lvc)
        if look.is_some() && !l && inner.w > 1 {
            inner.x += 1;
            inner.w -= 1;
        }
        // a text preview (the file, or the Neovim that paints it) has the blue bar of the lines shown above it, as in lvc
        let mut bar: Option<Rect> = None;
        if look.is_some()
            && !l
            && self.pv.active != Some(Provider::Plugin)
            && (self.nvim_mode() || self.pv.active == Some(Provider::File))
            && inner.h > 1
        {
            // from the text's column less one: the bar's own leading space is the indent the text has
            bar = Some(Rect { x: inner.x - 1, y: inner.y, w: inner.w + 1, h: 1 });
            inner.y += 1;
            inner.h -= 1;
        }
        self.pv.size = (inner.w, inner.h);
        if inner.w == 0 || inner.h == 0 {
            return;
        }
        self.pv.inner = Some(inner);
        // a picture put on the screen stays until it is taken off: only the plugin provider keeps one
        if self.pv.active != Some(Provider::Plugin) {
            self.pv.drawer.forget();
        }
        if self.pv.active == Some(Provider::Plugin)
            && let (Some(look), Some(path)) = (look.or(self.look.as_ref()), self.pv.list_target.clone())
        {
            let skip = self.pv.plug_skip;
            match self.pv.plug_answers.get(&(path.clone(), skip)) {
                Some(lvim_fuzzy_ui::provided::Answer::Preview(p)) => {
                    let p = p.clone();
                    let key = format!("{}|{skip}", path.display());
                    self.pv.rows_total = self.pv.drawer.draw(cv, look, inner, &p, &key, &mut self.pv.offset);
                }
                _ => {
                    cv.put(inner.x + 1, inner.y, "…", look.counter, inner.x + inner.w);
                    self.pv.rows_total = 0;
                }
            }
            self.draw_preview_scrollbar(cv, inner, self.pv.rows_total, self.pv.offset);
            return;
        }
        if self.nvim_mode() {
            // Neovim still on an earlier item (keys held down, a slow file type, a cold start):
            // the built-in preview shows the current file at once, as fzf + bat would; Neovim
            // takes over the moment it has painted this item. Not while the preview has focus.
            if !self.pv.focus && !self.nvim_caught_up() {
                // keep Neovim attached at the pane's size, so its frame is ready to take over
                if let Some(n) = self.pv.nvim.as_mut() {
                    n.size(inner.w as usize, inner.h as usize);
                }
                self.draw_file(cv, inner);
                if let (Some(r), Some(lk)) = (bar, look) {
                    lvim_fuzzy_ui::listview::scroll_bar(cv, lk, r, self.pv.offset, self.preview_lines_total(), inner.h as usize);
                }
                return;
            }
            self.draw_nvim(cv, inner);
            // the indent is padding, not margin: the cell left of the text is the preview's own colour (Neovim's), as the text rows are
            if bar.is_some() && inner.x > 0 {
                let bg = self.pv.nvim.as_ref().and_then(|n| n.grid.lock().unwrap().default_bg);
                if bg.is_some() {
                    for y in inner.y..inner.y + inner.h {
                        cv.fill(Rect { x: inner.x - 1, y, w: 1, h: 1 }, Style { bg, ..Style::default() });
                    }
                }
            }
            if let (Some(r), Some(lk)) = (bar, look) {
                let (top, bot, total) = self.pv.nvim.as_ref().map(|n| n.grid.lock().unwrap().view).unwrap_or((0, 0, 0));
                if total > 0 {
                    let shown = (bot + 1).saturating_sub(top.max(1));
                    lvim_fuzzy_ui::listview::scroll_bar(cv, lk, r, top.max(1) - 1, total, shown.max(1));
                }
            }
            return;
        }
        if self.pv.active == Some(Provider::File) {
            self.draw_file(cv, inner);
            self.draw_preview_scrollbar(cv, inner, self.preview_lines_total(), self.pv.offset);
            if let (Some(r), Some(lk)) = (bar, look) {
                lvim_fuzzy_ui::listview::scroll_bar(cv, lk, r, self.pv.offset, self.preview_lines_total(), inner.h as usize);
            }
            return;
        }
        if self.pv.active.is_none() && self.pv.oneoff.is_none() {
            // "Nothing to preview" only for a current item no provider can show — never while there is
            // no current item (the items or the first matches still coming; with no matches the list
            // says so), and never for an item whose preview is chosen right after this frame (the draw
            // runs before the choice): each of those showed the note for a moment, read as a flicker
            let decided = self.pv.key.as_ref().map(|k| k.0) == Some(self.current_index());
            if self.current_index().is_none() || !decided {
                return;
            }
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
            // the rows in view and how many there are: 3-12/12 at the end (the first row alone read as "3 of 12" with the last one on screen)
            let last = (off + body_h).min(body_total.max(1));
            let info = format!(" {spin}{}-{}/{} ", off + 1, last, body_total.max(1));
            let iw = str_width(&info) as u16;
            let y = if t { r.y } else { inner.y };
            let x = (r.x + r.w).saturating_sub(iw + if rt { 2 } else { 0 });
            // in the green of the counter of the title bar (the theme's), else the info colour
            let fg = look.and_then(|l| l.counter.fg).or(th.get(Slot::Info).fg);
            cv.put(
                x,
                y,
                &info,
                Style {
                    fg,
                    bg: pbg.bg,
                    ..th.get(Slot::Info)
                },
                r.x + r.w,
            );
        }
        // scrollbar
        if body_total > body_h
            && body_h > 0
            && self.opts.scrollbar.as_deref() != Some("")
            && !self.hidden("scrollbar-right")
        {
            let bar = ((body_h * body_h) / body_total).max(1);
            let pos = if max_off == 0 {
                0
            } else {
                off * (body_h - bar) / max_off
            };
            let x = if rt && self.pv.bar_panel_bg.is_none() {
                r.x + r.w - 1
            } else {
                self.preview_bar_x(inner)
            };
            let st = self.preview_bar_style();
            let ch = if self.opts.unicode { "┃" } else { "|" };
            for k in 0..bar {
                cv.put(x, inner.y + (hdr + pos + k) as u16, ch, st, x + 1);
            }
        }
    }

    /// The preview's thumb: in the lvim look on the panel's background (it stands in the blank ring outside the preview, as the list's does), else on
    /// the preview's.
    fn preview_bar_style(&self) -> Style {
        let bg = self.pv.bar_panel_bg.unwrap_or(self.theme.get(Slot::PreviewBg).bg);
        Style {
            bg,
            ..self.theme.get(Slot::PreviewScrollbar)
        }
    }

    /// The column of the preview's scrollbar: in the lvim look the blank one just outside the content (as the list's is, outside its rows), so both
    /// stand outside their boxes; in the plain look the last column of the content.
    fn preview_bar_x(&self, inner: Rect) -> u16 {
        if self.pv.bar_panel_bg.is_some() { inner.x + inner.w } else { inner.x + inner.w - 1 }
    }

    /// The thumb at the right edge of the preview, whenever the content is longer than the pane (every provider but Neovim, which draws its own).
    fn draw_preview_scrollbar(&self, cv: &mut Canvas, inner: Rect, total: usize, offset: usize) {
        let body_h = inner.h as usize;
        if self.opts.scrollbar.as_deref() == Some("")
            || self.hidden("scrollbar-right")
            || body_h == 0
            || total <= body_h
            || inner.w < 2
        {
            return;
        }
        let max_off = total - body_h;
        let off = if offset == usize::MAX { 0 } else { offset.min(max_off) };
        let bar = ((body_h * body_h) / total).max(1);
        let pos = off * (body_h - bar) / max_off;
        let st = self.preview_bar_style();
        let ch = if self.opts.unicode { "┃" } else { "|" };
        let x = self.preview_bar_x(inner);
        for k in 0..bar {
            cv.put(x, inner.y + (pos + k) as u16, ch, st, x + 1);
        }
    }

    fn preview_lines_total(&self) -> usize {
        if matches!(self.pv.active, Some(Provider::Plugin)) {
            return self.pv.rows_total;
        }
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
        // a media preview with positions (PDF / document pages, a video's points): the paging keys move them
        if self.pv.active == Some(Provider::Plugin)
            && let Some(path) = self.pv.list_target.clone()
            && let Some(lvim_fuzzy_ui::provided::Answer::Preview(p)) = self.pv.plug_answers.get(&(path, self.pv.plug_skip))
            && p.positions > 1
        {
            let step = match name {
                "preview-down" | "preview-page-down" | "preview-half-page-down" => Some(1i32),
                "preview-up" | "preview-page-up" | "preview-half-page-up" => Some(-1),
                _ => None,
            };
            if let Some(d) = step {
                let last = p.positions - 1;
                let to = (self.pv.plug_skip as i32 + d).clamp(0, last as i32) as u32;
                if to != self.pv.plug_skip {
                    self.pv.plug_skip = to;
                    self.pv.drawer.moved();
                    self.pv.force = true;
                }
                return true;
            }
        }
        if name == "rotate-preview" || name == "rotate-preview-back" {
            // right → left → up → hidden → right, as the Neovim picker (lvim-ui) turns it; back is the other way
            let (pos, visible) =
                rotate_side(self.pv.spec.pos, self.pv.visible, if name == "rotate-preview" { 1 } else { -1 });
            self.pv.spec.pos = pos;
            self.pv.spec.alt = None;
            self.pv.visible = visible;
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

#[cfg(test)]
mod rotate_tests {
    use super::*;

    #[test]
    fn the_cycle_is_right_left_up_down_hidden() {
        let mut at = (Pos::Right, true);
        let mut seen = Vec::new();
        for _ in 0..5 {
            at = rotate_side(at.0, at.1, 1);
            seen.push(at);
        }
        assert_eq!(seen, vec![(Pos::Left, true), (Pos::Up, true), (Pos::Down, true), (Pos::Down, false), (Pos::Right, true)]);
        // and the other way round
        let mut back = Vec::new();
        for _ in 0..5 {
            at = rotate_side(at.0, at.1, -1);
            back.push(at);
        }
        // (hidden keeps the side it was hidden on)
        assert_eq!(back, vec![(Pos::Right, false), (Pos::Down, true), (Pos::Up, true), (Pos::Left, true), (Pos::Right, true)]);
    }

    #[test]
    fn hidden_goes_on_from_its_place_in_the_cycle() {
        // forward from hidden: the right; back from hidden: the bottom
        assert_eq!(rotate_side(Pos::Left, false, 1), (Pos::Right, true));
        assert_eq!(rotate_side(Pos::Left, false, -1), (Pos::Down, true));
    }
}
