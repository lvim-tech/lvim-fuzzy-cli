//! `--bind` actions.

use super::*;

fn is_word(c: char, filepath: bool) -> bool {
    c.is_alphanumeric() && !(filepath && c == '/')
}

/// The command that opens `path` at `line` / `col` in the editor: `editor`
/// (--editor / `editor` in the config; `{file}`, `{line}`, `{col}` are filled
/// in), nvim by default. Without placeholders the line is passed the way the
/// editor takes it (`+N` for vi-likes, `--goto f:l:c` for VS Code).
/// The editor for several files: (n)vim with lines (grep results) gets them
/// as a quickfix list (`-q`: every hit at its place, :cnext between them);
/// else every file as an argument.
pub(super) fn editor_command_many(
    editor: Option<&str>,
    targets: &[(String, usize, usize)],
) -> String {
    if let [(p, l, c)] = targets {
        return editor_command(editor, p, *l, *c);
    }
    let ed = editor
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("nvim")
        .to_string();
    let files: Vec<String> = targets
        .iter()
        .map(|(p, _, _)| crate::placeholder::quote(p))
        .collect();
    if ed.contains("{file}") {
        let (_, l, c) = &targets[0];
        return ed
            .replace("{file}", &files.join(" "))
            .replace("{line}", &l.max(&1).to_string())
            .replace("{col}", &c.max(&1).to_string());
    }
    let prog = ed.split_whitespace().next().unwrap_or("");
    let base = std::path::Path::new(prog)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let vimlike = matches!(base.as_str(), "vi" | "vim" | "nvim" | "gvim");
    if vimlike && targets.iter().any(|(_, l, _)| *l > 0) {
        // a quickfix file that outlives this process (the editor reads it
        // after the finder is gone)
        let qf = std::env::temp_dir().join(format!("lvim-fuzzy-qf-{}.txt", std::process::id()));
        let text: String = targets
            .iter()
            .map(|(p, l, c)| format!("{}:{}:{}:\n", p, l.max(&1), c.max(&1)))
            .collect();
        if std::fs::write(&qf, text).is_ok() {
            return format!(
                "{ed} -q {}",
                crate::placeholder::quote(&qf.to_string_lossy())
            );
        }
    }
    format!("{ed} {}", files.join(" "))
}

pub(super) fn editor_command(editor: Option<&str>, path: &str, line: usize, col: usize) -> String {
    let ed = editor
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("nvim")
        .to_string();
    let file = crate::placeholder::quote(path);
    let (line, col) = (line.max(1), col.max(1));
    if ed.contains("{file}") {
        return ed
            .replace("{file}", &file)
            .replace("{line}", &line.to_string())
            .replace("{col}", &col.to_string());
    }
    let prog = ed.split_whitespace().next().unwrap_or("");
    let base = std::path::Path::new(prog)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    match base.as_str() {
        "vi" | "vim" | "nvim" | "gvim" | "nano" | "micro" | "kak" | "emacs" | "emacsclient"
        | "hx" | "helix" | "joe" | "ne" => format!("{ed} +{line} {file}"),
        "code" | "codium" | "code-insiders" | "cursor" => {
            format!("{ed} --goto {file}:{line}:{col}")
        }
        _ => format!("{ed} {file}"),
    }
}

/// Whether `change-X` / `transform-X` names a text target apply_text knows.
fn is_text_target(n: &str) -> bool {
    let t = n
        .trim_start_matches("change-")
        .trim_start_matches("transform-");
    matches!(
        t,
        "prompt" | "ghost" | "header" | "preview-label" | "query" | "search" | "nth" | "with-nth"
    )
}

impl App {
    /// A live source: every query change re-runs the command (`change:reload`).
    pub(super) fn is_live(&self) -> bool {
        self.keymap
            .get("change")
            .is_some_and(|a| a.iter().any(|x| x.name.starts_with("reload")))
    }

    /// Set a text property (the target of change-* / transform-* / bg-transform-*).
    pub(super) fn apply_text(&mut self, target: &str, text: String) {
        match target {
            "prompt" => self.prompt = text,
            "ghost" => self.ghost = text,
            "header" => self.header = text.lines().map(str::to_string).collect(),
            "preview-label" => self.pv.label = text,
            "query" => self.set_query(&text),
            "search" => {
                self.search_override = Some(text);
                self.cursor = 0;
                self.request();
            }
            "nth" | "with-nth" => {
                // re-derive every item's display / match text with the new fields
                let ranges = if text.trim().is_empty() {
                    None
                } else {
                    match crate::fields::parse_nth(&text) {
                        Ok(r) => Some(r),
                        Err(e) => {
                            self.toast(e, true);
                            return;
                        }
                    }
                };
                if target == "nth" {
                    self.builder.nth = ranges.clone();
                    self.shared_builder.write().unwrap().nth = ranges;
                } else {
                    self.builder.with_nth = ranges.clone();
                    self.shared_builder.write().unwrap().with_nth = ranges;
                }
                {
                    // bumped under the write lock: whoever reads the items
                    // sees the count that goes with them
                    let mut items = self.engine.items.write().unwrap();
                    self.engine
                        .resets
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    items.rebuild(&self.builder);
                }
                self.request();
            }
            _ => {}
        }
    }

    fn move_vert(&mut self, visual_up: bool, n: usize) {
        let len = self.hits.len();
        if len == 0 {
            return;
        }
        // In the default layout the list grows upwards, so "up" means the
        // next (worse) item.
        let forward = !visual_up;
        if forward {
            if self.cursor + n < len {
                self.cursor += n;
            } else if self.opts.cycle && n == 1 {
                self.cursor = 0;
            } else {
                self.cursor = len - 1;
            }
        } else if self.cursor >= n {
            self.cursor -= n;
        } else if self.opts.cycle && n == 1 {
            self.cursor = len - 1;
        } else {
            self.cursor = 0;
        }
    }

    fn toggle_current(&mut self) {
        if self.opts.multi.is_none() {
            return;
        }
        if let Some(i) = self.current_index() {
            self.toggle_idx(i);
        }
    }

    fn toggle_idx(&mut self, i: u32) {
        if self.selset.remove(&i) {
            self.selected.retain(|&x| x != i);
        } else {
            self.select_idx(i);
        }
    }

    fn select_idx(&mut self, i: u32) -> bool {
        let max = self.opts.multi.unwrap_or(0);
        if self.opts.multi.is_none() || self.selset.contains(&i) {
            return false;
        }
        if max > 0 && self.selected.len() >= max {
            return false;
        }
        self.selset.insert(i);
        self.selected.push(i);
        true
    }

    fn deselect_idx(&mut self, i: u32) {
        if self.selset.remove(&i) {
            self.selected.retain(|&x| x != i);
        }
    }

    fn word_left(&self) -> usize {
        let fp = self.opts.filepath_word;
        let mut i = self.qcur;
        while i > 0 && !is_word(self.query[i - 1], fp) {
            i -= 1;
        }
        while i > 0 && is_word(self.query[i - 1], fp) {
            i -= 1;
        }
        i
    }

    fn word_right(&self) -> usize {
        let fp = self.opts.filepath_word;
        let mut i = self.qcur;
        let n = self.query.len();
        while i < n && !is_word(self.query[i], fp) {
            i += 1;
        }
        while i < n && is_word(self.query[i], fp) {
            i += 1;
        }
        i
    }

    fn set_query(&mut self, q: &str) {
        self.query = q.chars().collect();
        self.qcur = self.query.len();
        self.query_changed();
    }

    pub(super) fn expand(&mut self, template: &str) -> String {
        self.expand_for(self.current_index(), template)
    }

    /// `expand` with item `current` as `{}` (another than the focused one).
    pub(super) fn expand_for(&mut self, current: Option<u32>, template: &str) -> String {
        let items = self.engine.items.read().unwrap();
        let cur = current
            .and_then(|i| items.get(i as usize))
            .map(|it| (it.raw(), it.index));
        let selected = self
            .selected
            .iter()
            .filter_map(|&i| items.get(i as usize))
            .map(|it| (it.raw(), it.index))
            .collect();
        let q = self.query_string();
        let ctx = placeholder::Context {
            current: cur,
            selected,
            query: &q,
            delimiter: &self.opts.delimiter,
        };
        let mut temps = Vec::new();
        let s = placeholder::expand(template, &ctx, &mut temps);
        drop(items);
        self.temps.extend(temps);
        s
    }

    fn command(&self, cmd: &str) -> Command {
        let (prog, args) = self.shell.split_first().expect("shell");
        let mut c = Command::new(prog);
        c.args(args).arg(cmd);
        c.env("LVIM_FUZZY_QUERY", self.query_string());
        c.env("LVIM_FUZZY_PROMPT", &self.prompt);
        c.env("LVIM_FUZZY_MATCH_COUNT", self.hits.len().to_string());
        c.env("LVIM_FUZZY_SELECT_COUNT", self.selected.len().to_string());
        c.env(
            "LVIM_FUZZY_TOTAL_COUNT",
            self.engine.items.read().unwrap().len().to_string(),
        );
        c.env("LVIM_FUZZY_POS", (self.cursor + 1).to_string());
        c.env("LVIM_FUZZY", "1");
        if let Some(a) = &self.listen_addr {
            c.env(
                if a.contains('/') {
                    "LVIM_FUZZY_SOCK"
                } else {
                    "LVIM_FUZZY_PORT"
                },
                a,
            );
        }
        c
    }

    /// Run `cmd` and return its stdout (trailing newline removed).
    pub(super) fn capture(&mut self, template: &str) -> String {
        let cmd = self.expand(template);
        let out = self
            .command(&cmd)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        match out {
            Ok(o) => {
                if !o.status.success() {
                    self.toast(format!("command failed ({}): {cmd}", o.status), true);
                }
                String::from_utf8_lossy(&o.stdout)
                    .trim_end_matches('\n')
                    .to_string()
            }
            Err(e) => {
                self.toast(format!("cannot run command: {e}"), true);
                String::new()
            }
        }
    }

    fn reload(&mut self, template: &str) {
        let cmd = self.expand(template);
        // the previous command, all of it (taken out of the lock first)
        let prev = self.reload_child.lock().unwrap().take();
        if let Some(mut c) = prev {
            crate::reader::stop(&mut c);
        }
        self.epoch += 1;
        // free the old items (a grep can leave 100 000s) off the UI thread
        // bumped under the write lock: whoever reads the items sees the count
        // that goes with them
        // the running search is for items about to go: cancelled first, so
        // it lets go of them at once and the keys do not wait for it
        self.engine
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let old = {
            let mut items = self.engine.items.write().unwrap();
            self.engine
                .resets
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            std::mem::take(&mut *items)
        };
        std::thread::spawn(move || drop(old));
        self.selected.clear();
        self.selset.clear();
        self.loading = true;
        self.load_fired = false;
        self.cursor = 0;
        self.offset = 0;
        let feed = Feed {
            tx: self.feed_tx.clone(),
            epoch: self.epoch,
            build: Some(self.shared_builder.clone()),
            child: Some(self.reload_child.clone()),
        };
        let shell = self.shell.clone();
        let sep = if self.opts.read0 { 0u8 } else { b'\n' };
        let slot = self.reload_child.clone();
        std::thread::spawn(move || {
            reader::read_command(&shell, &cmd, sep, &feed, Some(slot));
            let _ = feed.send(Event::Eof);
        });
        self.request();
    }

    pub(super) fn run_actions(
        &mut self,
        acts: &[Action],
        mut term: Option<&mut Term>,
    ) -> std::io::Result<Option<Outcome>> {
        for a in acts {
            let arg = a.arg.clone().unwrap_or_default();
            let len = self.hits.len();
            match a.name.as_str() {
                "abort" => return Ok(Some(Outcome::Abort)),
                "cancel" => {
                    if self.query.is_empty() {
                        return Ok(Some(Outcome::Abort));
                    }
                    self.set_query("");
                }
                "accept" => return Ok(Some(Outcome::Accept { key: None })),
                "accept-non-empty" => {
                    if len > 0 || !self.selected.is_empty() {
                        return Ok(Some(Outcome::Accept { key: None }));
                    }
                }
                "accept-or-print-query" => {
                    if len == 0 && self.selected.is_empty() {
                        self.prints.push(self.query_string());
                        return Ok(Some(Outcome::Accept { key: None }));
                    }
                    return Ok(Some(Outcome::Accept { key: None }));
                }
                "become" => {
                    let cmd = self.expand(&arg);
                    return Ok(Some(Outcome::Become(cmd)));
                }
                "open" => {
                    if self.current_index().is_none() {
                        return Ok(None);
                    }
                    // the mode's own meaning of open (--open): switch a session, show a commit, ...
                    if let Some(spec) = self.opts.open.clone()
                        && let Ok(b) = keys::parse_binds(&format!("x:{spec}"))
                        && let Some((_, acts)) = b.into_iter().next()
                        && !acts.iter().any(|x| x.name == "open")
                    {
                        if let Some(o) = self.run_actions(&acts, term.as_deref_mut())? {
                            return Ok(Some(o));
                        }
                        continue;
                    }
                    // selected items: all their files in the editor at once
                    let tpl = self.pv.file_template.clone();
                    if !self.selected.is_empty() {
                        let targets: Vec<(String, usize, usize)> = self
                            .selected
                            .clone()
                            .into_iter()
                            .map(|i| self.target_for(i, &tpl))
                            .filter(|(p, _, _)| !p.is_empty() && std::path::Path::new(p).exists())
                            .collect();
                        // each file the way the open plugin says (as lvc and the picker do): a picture or a PDF in its
                        // viewer, detached; what is text, all together in the editor
                        let mut for_editor = Vec::new();
                        for t in targets {
                            match self.open_decision(&t.0) {
                                lvim_fuzzy_ui::open::Open::Exec(line) => {
                                    if let Err(e) = lvim_fuzzy_ui::open::spawn_detached(&line) {
                                        self.toast(format!("cannot open {}: {e}", t.0), true);
                                    }
                                }
                                lvim_fuzzy_ui::open::Open::Editor => for_editor.push(t),
                                lvim_fuzzy_ui::open::Open::Missing(m) => self.toast(m.join("; "), true),
                                lvim_fuzzy_ui::open::Open::Pass => self.toast(format!("no open plugin has an answer for {}", t.0), true),
                            }
                        }
                        if !for_editor.is_empty() {
                            let cmd = editor_command_many(self.opts.editor.as_deref(), &for_editor);
                            return Ok(Some(Outcome::Become(cmd)));
                        }
                        continue;
                    }
                    // else the focused item's file in the editor, at its line (grep results)
                    let (path, line, col) = self.target(&tpl);
                    if path.is_empty() || !std::path::Path::new(&path).exists() {
                        // not a file (a process, a commit, ...): print it instead
                        return Ok(Some(Outcome::Accept { key: None }));
                    } else {
                        // the same rule as lvc and the Neovim picker: what the open plugin says — a program (detached),
                        // the editor — and the finder stays open after a viewer starts
                        match self.open_decision(&path) {
                            lvim_fuzzy_ui::open::Open::Exec(cmd) => {
                                match lvim_fuzzy_ui::open::spawn_detached(&cmd) {
                                    Ok(()) => self.toast(format!("opened {path}"), false),
                                    Err(e) => self.toast(format!("cannot open {path}: {e}"), true),
                                }
                                continue;
                            }
                            lvim_fuzzy_ui::open::Open::Editor => {
                                let cmd = editor_command(self.opts.editor.as_deref(), &path, line, col);
                                return Ok(Some(Outcome::Become(cmd)));
                            }
                            lvim_fuzzy_ui::open::Open::Missing(m) => {
                                self.toast(m.join("; "), true);
                                continue;
                            }
                            lvim_fuzzy_ui::open::Open::Pass => {
                                self.toast(format!("no open plugin has an answer for {path}"), true);
                                continue;
                            }
                        }
                    }
                }
                "up" => self.move_vert(true, 1),
                "down" => self.move_vert(false, 1),
                "first" => self.cursor = 0,
                "last" => self.cursor = len.saturating_sub(1),
                "pos" => {
                    let n: isize = arg.trim().parse().unwrap_or(1);
                    let p = if n > 0 { n - 1 } else { len as isize + n };
                    self.cursor = p.clamp(0, len.saturating_sub(1) as isize) as usize;
                }
                "page-up" => self.move_vert(true, self.list_rows.max(1)),
                "page-down" => self.move_vert(false, self.list_rows.max(1)),
                "half-page-up" => self.move_vert(true, (self.list_rows / 2).max(1)),
                "half-page-down" => self.move_vert(false, (self.list_rows / 2).max(1)),
                "offset-up" | "offset-down" => {
                    let up = a.name == "offset-up";
                    let forward = !up;
                    if forward {
                        if self.offset + self.list_rows < len {
                            self.offset += 1;
                            self.cursor = self.cursor.max(self.offset);
                        }
                    } else if self.offset > 0 {
                        self.offset -= 1;
                        self.cursor = self
                            .cursor
                            .min(self.offset + self.list_rows.saturating_sub(1));
                    }
                }
                "toggle" => self.toggle_current(),
                "toggle+down" => {
                    self.toggle_current();
                    self.move_vert(false, 1);
                }
                "toggle+up" => {
                    self.toggle_current();
                    self.move_vert(true, 1);
                }
                "toggle-in" | "toggle-out" => {
                    self.toggle_current();
                    let up = a.name == "toggle-in";
                    self.move_vert(up, 1);
                }
                "select" => {
                    if let Some(i) = self.current_index() {
                        self.select_idx(i);
                    }
                }
                "deselect" => {
                    if let Some(i) = self.current_index() {
                        self.deselect_idx(i);
                    }
                }
                "select-all" | "deselect-all" | "toggle-all" => {
                    for k in 0..len {
                        let i = self.hits.get(k).unwrap();
                        match a.name.as_str() {
                            "select-all" => {
                                self.select_idx(i);
                            }
                            "deselect-all" => self.deselect_idx(i),
                            _ => self.toggle_idx(i),
                        }
                    }
                }
                "clear-selection" | "clear-multi" => {
                    self.selected.clear();
                    self.selset.clear();
                }
                "clear-query" => self.set_query(""),
                "beginning-of-line" => self.qcur = 0,
                "end-of-line" => self.qcur = self.query.len(),
                "backward-char" => self.qcur = self.qcur.saturating_sub(1),
                "forward-char" => self.qcur = (self.qcur + 1).min(self.query.len()),
                "backward-word" | "backward-subword" => self.qcur = self.word_left(),
                "forward-word" | "forward-subword" => self.qcur = self.word_right(),
                "backward-delete-char" | "backward-delete-char/eof" => {
                    if self.query.is_empty() && a.name.ends_with("/eof") {
                        return Ok(Some(Outcome::Abort));
                    }
                    if self.qcur > 0 {
                        self.qcur -= 1;
                        self.query.remove(self.qcur);
                        self.query_changed();
                    } else if self.query.is_empty() {
                        if let Some(acts) = self.keymap.get("backward-eof").cloned() {
                            return self.run_actions(&acts, term);
                        }
                    }
                }
                "delete-char" | "delete-char/eof" => {
                    if self.query.is_empty() && a.name.ends_with("/eof") {
                        return Ok(Some(Outcome::Abort));
                    }
                    if self.qcur < self.query.len() {
                        self.query.remove(self.qcur);
                        self.query_changed();
                    }
                }
                "unix-line-discard" => {
                    if self.qcur > 0 {
                        self.yank = self.query.drain(..self.qcur).collect();
                        self.qcur = 0;
                        self.query_changed();
                    }
                }
                "kill-line" => {
                    if self.qcur < self.query.len() {
                        self.yank = self.query.drain(self.qcur..).collect();
                        self.query_changed();
                    }
                }
                "unix-word-rubout" => {
                    let mut i = self.qcur;
                    while i > 0 && self.query[i - 1].is_whitespace() {
                        i -= 1;
                    }
                    while i > 0 && !self.query[i - 1].is_whitespace() {
                        i -= 1;
                    }
                    if i < self.qcur {
                        self.yank = self.query.drain(i..self.qcur).collect();
                        self.qcur = i;
                        self.query_changed();
                    }
                }
                "backward-kill-word" | "backward-kill-subword" => {
                    let i = self.word_left();
                    if i < self.qcur {
                        self.yank = self.query.drain(i..self.qcur).collect();
                        self.qcur = i;
                        self.query_changed();
                    }
                }
                "kill-word" | "kill-subword" => {
                    let i = self.word_right();
                    if i > self.qcur {
                        self.yank = self.query.drain(self.qcur..i).collect();
                        self.query_changed();
                    }
                }
                "yank" => {
                    let y: Vec<char> = self.yank.chars().collect();
                    let n = y.len();
                    self.query.splice(self.qcur..self.qcur, y);
                    self.qcur += n;
                    if n > 0 {
                        self.query_changed();
                    }
                }
                "put" => {
                    let s = if a.arg.is_some() {
                        arg.clone()
                    } else {
                        String::new()
                    };
                    for c in s.chars() {
                        self.query.insert(self.qcur, c);
                        self.qcur += 1;
                    }
                    self.query_changed();
                }
                "change-query" => self.set_query(&arg),
                "transform-query" => {
                    let q = self.capture(&arg);
                    self.set_query(&q);
                }
                "change-prompt" => self.prompt = arg.clone(),
                "transform-prompt" => self.prompt = self.capture(&arg),
                "change-ghost" => self.ghost = arg.clone(),
                "transform-ghost" => self.ghost = self.capture(&arg),
                "change-header" => self.header = arg.lines().map(str::to_string).collect(),
                "transform-header" => {
                    self.header = self.capture(&arg).lines().map(str::to_string).collect()
                }
                "change-multi" => {
                    self.opts.multi = if a.arg.is_none() {
                        Some(0)
                    } else {
                        arg.trim().parse().ok().filter(|&n: &usize| n > 0).or(None)
                    };
                    if arg.trim() == "0" {
                        self.opts.multi = None;
                        self.selected.clear();
                        self.selset.clear();
                    }
                }
                "transform" => {
                    let out = self.capture(&arg);
                    match keys::parse_binds(&format!("x:{out}")) {
                        Ok(b) => {
                            if let Some((_, acts)) = b.into_iter().next()
                                && let Some(o) = self.run_actions(&acts, term.as_deref_mut())?
                            {
                                return Ok(Some(o));
                            }
                        }
                        Err(e) => self.toast(format!("transform: {e}"), true),
                    }
                }
                "print" => {
                    let s = self.expand(&arg);
                    self.prints.push(s);
                }
                "print-query" => {
                    self.prints.push(self.query_string());
                    return Ok(Some(Outcome::Accept { key: None }));
                }
                "toggle-sort" => {
                    self.sort = !self.sort;
                    self.request();
                }
                "toggle-search" => {
                    self.search = !self.search;
                    self.request();
                }
                "enable-search" => {
                    self.search = true;
                    self.request();
                }
                "disable-search" => {
                    self.search = false;
                    self.request();
                }
                "toggle-multi-line" | "toggle-hscroll" => {
                    self.opts.hscroll = !self.opts.hscroll;
                }
                "clear-screen" => {
                    if let Some(t) = term.as_deref_mut() {
                        t.prev = None;
                    }
                }
                "execute" => {
                    let cmd = self.expand(&arg);
                    if let Some(t) = term.as_deref_mut() {
                        t.suspend()?;
                        let tty_in = File::open("/dev/tty")
                            .map(Stdio::from)
                            .unwrap_or(Stdio::inherit());
                        let tty_out = OpenOptions::new()
                            .write(true)
                            .open("/dev/tty")
                            .map(Stdio::from)
                            .unwrap_or(Stdio::inherit());
                        let st = self.command(&cmd).stdin(tty_in).stdout(tty_out).status();
                        t.resume()?;
                        if let Err(e) = st {
                            self.toast(format!("cannot run command: {e}"), true);
                        }
                    }
                }
                "execute-silent" => {
                    let cmd = self.expand(&arg);
                    let st = self
                        .command(&cmd)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                    if let Err(e) = st {
                        self.toast(format!("cannot run command: {e}"), true);
                    }
                }
                "reload" | "reload-sync" => self.reload(&arg),
                "toggle-grep-filter" => {
                    if let Some(acts) = self.live_stash.take() {
                        // back to live grep: the source runs again for the grep query
                        self.keymap.insert("change".into(), acts);
                        self.search = false;
                        let q = std::mem::take(&mut self.live_query);
                        self.set_query(&q);
                        self.toast("grep: the query searches the files again".into(), false);
                    } else if self.is_live() {
                        // freeze the results; the query now filters them (and F4's scope applies)
                        self.live_stash = self.keymap.remove("change");
                        self.live_query = self.query_string();
                        self.search = true;
                        self.set_query("");
                        self.toast(
                            format!(
                                "filter: the query narrows these results ({}, F4 changes it)",
                                scope_label(self.scope)
                            ),
                            false,
                        );
                    } else {
                        self.toast(
                            "not a live source (grep ⇄ filter is for live modes)".into(),
                            false,
                        );
                    }
                }
                "rebind" | "unbind" | "toggle-bind" => {
                    // keys are re-read from the original map on rebind
                    for k in arg.split(',').filter(|s| !s.is_empty()) {
                        if let Ok(k) = keys::canonical_key(k) {
                            if a.name == "unbind" {
                                self.keymap.remove(&k);
                            } else if a.name == "rebind" {
                                if let Some(v) = self.base_keymap.get(&k).cloned() {
                                    self.keymap.insert(k, v);
                                }
                            } else if self.keymap.remove(&k).is_none()
                                && let Some(v) = self.base_keymap.get(&k).cloned()
                            {
                                self.keymap.insert(k, v);
                            }
                        }
                    }
                }
                "trigger" => {
                    for k in arg.split(',') {
                        if let Ok(k) = keys::canonical_key(k)
                            && let Some(acts) = self.keymap.get(&k).cloned()
                            && let Some(o) = self.run_actions(&acts, term.as_deref_mut())?
                        {
                            return Ok(Some(o));
                        }
                    }
                }
                "mode-menu" => self.open_menu(),
                "toggle-raw" | "enable-raw" | "disable-raw" => {
                    self.opts.raw = match a.name.as_str() {
                        "toggle-raw" => !self.opts.raw,
                        "enable-raw" => true,
                        _ => false,
                    };
                    // keep the focused item across the switch
                    self.tracked = self.current_index();
                    self.tracked_key = self.id_key(self.tracked);
                    self.refocus = true;
                    self.request();
                }
                "up-match" | "down-match" => {
                    // raw mode: the previous / next MATCHING item (visual direction)
                    if let Some(set) = &self.raw_set {
                        let fwd = a.name != "up-match";
                        let n = self.hits.len();
                        let mut i = self.cursor;
                        for _ in 0..n {
                            i = if fwd {
                                (i + 1) % n.max(1)
                            } else {
                                (i + n - 1) % n.max(1)
                            };
                            if self.hits.get(i).is_some_and(|x| set.contains(&x)) {
                                self.cursor = i;
                                break;
                            }
                        }
                    } else {
                        self.move_vert(a.name == "up-match", 1);
                    }
                }
                "toggle-wrap" | "toggle-wrap-word" => self.opts.wrap = !self.opts.wrap,
                "track" | "track-current" => {
                    self.track = true;
                    self.tracked = self.current_index();
                    self.tracked_key = self.id_key(self.tracked);
                }
                "untrack-current" => {
                    self.tracked = None;
                    self.tracked_key = None;
                    self.track = self.opts.track;
                }
                "toggle-track" | "toggle-track-current" => {
                    self.track = !self.track;
                    self.tracked = if self.track {
                        self.current_index()
                    } else {
                        None
                    };
                }
                "prev-history" | "next-history" => {
                    if self.history.is_empty() {
                        continue;
                    }
                    let last = self.history.len() - 1;
                    let pos = match (a.name.as_str(), self.hist_pos) {
                        ("prev-history", None) => {
                            self.hist_draft = self.query_string();
                            Some(last)
                        }
                        ("prev-history", Some(p)) => Some(p.saturating_sub(1)),
                        ("next-history", Some(p)) if p < last => Some(p + 1),
                        ("next-history", Some(_)) => None,
                        _ => continue,
                    };
                    self.hist_pos = pos;
                    let q = match pos {
                        Some(p) => self.history[p].clone(),
                        None => self.hist_draft.clone(),
                    };
                    self.set_query(&q);
                }
                "jump" | "jump-accept" => self.jump = Some(a.name == "jump-accept"),
                "theme-menu" => self.open_theme_menu(),
                "switch-mode" => {
                    let name = arg.trim().to_string();
                    match crate::find_mode(&name) {
                        Some(m) if m.missing().is_empty() => {
                            return Ok(Some(Outcome::Switch(name, self.query_string())));
                        }
                        Some(m) => self.toast(
                            format!("mode `{name}` needs: {}", m.missing().join(", ")),
                            true,
                        ),
                        None => self.toast(format!("no such mode: {name}"), true),
                    }
                }
                "toggle-scope" => {
                    if self.grep_input || self.opts.scope.is_some() {
                        self.scope = self.scope.next();
                        self.cursor = 0;
                        self.offset = 0;
                        self.request();
                        self.toast(format!("filter: {}", scope_label(self.scope)), false);
                    } else {
                        self.toast(
                            "the filter scope needs grep results (path:line:text)".into(),
                            false,
                        );
                    }
                }
                "help" => {
                    self.help = Some(Help {
                        query: String::new(),
                        offset: 0,
                        max_off: std::cell::Cell::new(0),
                    })
                }
                "ignore" | "bell" => {}
                "replace-query" => {
                    let t = {
                        let items = self.engine.items.read().unwrap();
                        self.current_index()
                            .and_then(|i| items.get(i as usize))
                            .map(|it| it.display().to_string())
                    };
                    if let Some(t) = t {
                        self.set_query(&t);
                    }
                }
                "exclude" | "exclude-multi" => {
                    if a.name == "exclude-multi" && !self.selected.is_empty() {
                        self.excluded.extend(self.selected.drain(..));
                        self.selset.clear();
                    } else if let Some(i) = self.current_index() {
                        self.excluded.insert(i);
                    }
                    self.request();
                }
                "hide-header" => self.header_hidden = true,
                "show-header" => self.header_hidden = false,
                "toggle-header" => self.header_hidden = !self.header_hidden,
                "hide-input" => self.opts.no_input = true,
                "show-input" => self.opts.no_input = false,
                "toggle-input" => self.opts.no_input = !self.opts.no_input,
                "close" => {
                    if self.preview_enabled() {
                        self.pv.visible = false;
                    } else {
                        return Ok(Some(Outcome::Abort));
                    }
                }
                "offset-middle" => {
                    self.offset = self.cursor.saturating_sub(self.list_rows / 2);
                }
                "next-selected" | "prev-selected" | "up-selected" | "down-selected" => {
                    let fwd = match a.name.as_str() {
                        "next-selected" => true,
                        "prev-selected" => false,
                        n => n != "up-selected",
                    };
                    let n = self.hits.len();
                    let mut i = self.cursor;
                    for _ in 0..n {
                        i = if fwd {
                            (i + 1) % n.max(1)
                        } else {
                            (i + n - 1) % n.max(1)
                        };
                        if self.hits.get(i).is_some_and(|x| self.selset.contains(&x)) {
                            self.cursor = i;
                            break;
                        }
                    }
                }
                "bg-cancel" => {
                    self.bg_gen += 1;
                    crate::reader::stop_slot(&self.bg_child);
                }
                n if n.starts_with("bg-transform") => {
                    // run the command off the UI thread; apply its output when it is done
                    let target = n
                        .strip_prefix("bg-transform")
                        .unwrap()
                        .trim_start_matches('-')
                        .to_string();
                    let cmd = self.expand(&arg);
                    let c = self.command(&cmd);
                    let tx = self.bg_tx.clone();
                    let g = self.bg_gen;
                    let slot = self.bg_child.clone();
                    std::thread::spawn(move || {
                        let out = crate::reader::capture(c, &slot)
                            .map(|o| {
                                String::from_utf8_lossy(&o)
                                    .trim_end_matches('\n')
                                    .to_string()
                            })
                            .unwrap_or_default();
                        let _ = tx.send((g, target, out));
                    });
                }
                n if (n.starts_with("change-") || n.starts_with("transform-"))
                    && is_text_target(n) =>
                {
                    let (target, text) = match n.strip_prefix("change-") {
                        Some(t) => (t.to_string(), arg.clone()),
                        None => (
                            n.trim_start_matches("transform-").to_string(),
                            self.capture(&arg),
                        ),
                    };
                    self.apply_text(&target, text);
                }
                other => {
                    if !self.preview_action(other, &arg, a.arg.is_some()) {
                        self.toast(format!("action not implemented yet: {other}"), true);
                    }
                }
            }
        }
        Ok(None)
    }
}

pub(super) fn become_cmd(shell: &[String], cmd: &str) -> Result<i32, String> {
    let (prog, args) = shell.split_first().ok_or("no shell")?;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = Command::new(prog).args(args).arg(cmd).exec();
        Err(format!("become: {err}"))
    }
    #[cfg(not(unix))]
    {
        let st = Command::new(prog)
            .args(args)
            .arg(cmd)
            .status()
            .map_err(|e| e.to_string())?;
        Ok(st.code().unwrap_or(1))
    }
}

/// How a scope reads on the footer, the title and the toast.
pub(super) fn scope_label(s: crate::engine::Scope) -> &'static str {
    s.name()
}

#[cfg(test)]
mod editor_tests {
    use super::{editor_command, editor_command_many};

    #[test]
    fn editor_commands_for_several() {
        let t = |p: &str, l| (p.to_string(), l, 1usize);
        assert_eq!(
            editor_command_many(None, &[t("a.rs", 0), t("b.rs", 0)]),
            "nvim 'a.rs' 'b.rs'"
        );
        assert_eq!(editor_command_many(None, &[t("a.rs", 3)]), "nvim +3 'a.rs'");
        let q = editor_command_many(None, &[t("a.rs", 3), t("b.rs", 7)]);
        assert!(q.starts_with("nvim -q "), "{q}");
        let path = q.trim_start_matches("nvim -q ").trim_matches('\'');
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "a.rs:3:1:\nb.rs:7:1:\n"
        );
        assert_eq!(
            editor_command_many(Some("code"), &[t("a.rs", 3), t("b.rs", 7)]),
            "code 'a.rs' 'b.rs'"
        );
    }

    #[test]
    fn editor_commands() {
        assert_eq!(editor_command(None, "a.rs", 12, 3), "nvim +12 'a.rs'");
        assert_eq!(
            editor_command(Some("code"), "a.rs", 12, 3),
            "code --goto 'a.rs':12:3"
        );
        assert_eq!(
            editor_command(Some("hx {file}:{line}:{col}"), "a.rs", 2, 0),
            "hx 'a.rs':2:1"
        );
        assert_eq!(editor_command(Some("gedit"), "a b", 0, 0), "gedit 'a b'");
    }
}
