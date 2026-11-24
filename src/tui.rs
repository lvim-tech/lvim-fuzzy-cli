//! The interactive finder.

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as TermEvent, KeyEventKind,
};
use crossterm::style::Color;
use crossterm::{cursor, execute, queue, terminal};

use crate::canvas::{self, Canvas, Positioner, Rect, str_width, truncate};
use crate::preview::Pos;
use crate::engine::{Engine, Hits};
use crate::item::ItemBuilder;
use crate::keys::{self, Action, KeyMap};
use crate::matcher::Pattern;
use crate::options::{BorderStyle, Options};
use crate::placeholder;
use crate::reader::{self, Event, Feed};
use crate::theme::{Slot, Style, Theme};

mod actions;
mod frame;
mod menu;
mod pview;
mod themes;
mod view;
mod view_lvim;

use actions::become_cmd;

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

enum Outcome {
    Accept {
        key: Option<String>,
    },
    Abort,
    Become(String),
    /// Re-run in another mode (name, query).
    Switch(String, String),
}

struct Toast {
    text: String,
    error: bool,
    until: Instant,
}

struct Help {
    query: String,
    offset: usize,
    /// The largest useful offset, as of the last draw (scrolling stops there).
    max_off: std::cell::Cell<usize>,
}

struct App {
    opts: Options,
    theme: Theme,
    keymap: KeyMap,
    base_keymap: KeyMap,
    engine: Engine,
    builder: ItemBuilder,
    /// The same, shared with the reading threads (built items arrive ready).
    shared_builder: Arc<std::sync::RwLock<ItemBuilder>>,
    shell: Vec<String>,
    // input
    feed_tx: std::sync::mpsc::Sender<(u64, Event)>,
    epoch: u64,
    loading: bool,
    header_left: usize,
    header_items: Vec<String>,
    reload_child: Arc<Mutex<Option<Child>>>,
    // query
    query: Vec<char>,
    qcur: usize,
    yank: String,
    prompt: String,
    ghost: String,
    header: Vec<String>,
    // results
    hits: Hits,
    requested: u64,
    received: u64,
    /// The query behind each pending request (generation, query), and the
    /// one behind the shown result: highlights use that one, so they always
    /// match the rows on screen (not what is typed meanwhile).
    asked: Vec<(u64, String)>,
    shown_query: String,
    /// When a result was last put on the screen.
    last_shown: Instant,
    dirty_items: bool,
    last_request: Instant,
    sort: bool,
    search: bool,
    /// --track: keep the focused item focused when the results change.
    track: bool,
    tracked: Option<u32>,
    /// --history: earlier queries, the position while browsing them, and the
    /// query typed before browsing began.
    history: Vec<String>,
    hist_pos: Option<usize>,
    hist_draft: String,
    /// Raw mode: every item is listed; this is the set that matches the query.
    raw_set: Option<std::collections::HashSet<u32>>,
    /// Put the focus back on `tracked` with the next result (one shot).
    refocus: bool,
    /// start / change events: run at the top of the event loop, where their
    /// actions can end the finder (start:accept, change:abort, ...).
    pending_events: Vec<&'static str>,
    /// Live grep frozen into a filter (toggle-grep-filter): the stashed
    /// `change` binding that re-runs the source, and the grep query.
    live_stash: Option<Vec<keys::Action>>,
    live_query: String,
    /// --id-nth: the tracked item's identity (its id fields), so tracking
    /// survives a reload that renumbers the items.
    tracked_key: Option<String>,
    header_hidden: bool,
    /// search(...): what the matcher reads instead of the typed query.
    search_override: Option<String>,
    /// exclude / exclude-multi: items taken out of every result.
    excluded: HashSet<u32>,
    /// bg-transform*: results of background commands (generation, target, output).
    bg_tx: std::sync::mpsc::Sender<(u64, String, String)>,
    bg_rx: Receiver<(u64, String, String)>,
    bg_gen: u64,
    /// The running bg-transform command (stopped by bg-cancel, the next one,
    /// or the exit).
    bg_child: Arc<Mutex<Option<std::process::Child>>>,
    /// --listen: requests from the HTTP endpoint, and its port / socket.
    listen_rx: Option<Receiver<crate::listen::Req>>,
    listen_addr: Option<String>,
    /// Jump mode: Some(accept) while the row labels are shown.
    jump: Option<bool>,
    /// Filter scope for grep results, and whether the input is grep results.
    scope: crate::engine::Scope,
    grep_input: bool,
    scope_checked: bool,
    // list state
    cursor: usize,
    offset: usize,
    selected: Vec<u32>,
    selset: HashSet<u32>,
    last_focus: Option<u32>,
    row_map: Vec<(u16, usize)>,
    list_rows: usize,
    // misc
    prints: Vec<String>,
    toast: Option<Toast>,
    /// Every popup's background: the terminal's, a step lighter / darker.
    popup_bg: Option<crossterm::style::Color>,
    help: Option<Help>,
    menu: Option<menu::Menu>,
    theme_menu: Option<themes::ThemeMenu>,
    /// Set by Shift-Enter: print the folder of the choice (for `cd "$(lfz)"`) instead of the choice.
    print_folder: bool,
    /// The lvim theme in use (its file stem).
    theme_name: String,
    spinner: usize,
    load_fired: bool,
    temps: Vec<std::path::PathBuf>,
    last_click: Option<Instant>,
    /// `row-tip`: the whole text of the row under the cursor is shown over the list
    tip: bool,
    tip_was: bool,
    /// the row under the cursor is cut at the end by the width of the list (the footer then offers `row-tip`)
    cursor_cut: bool,
    /// the button bar: the first button shown (it scrolls when they do not all fit), its row, and whether all fit
    footer_off: usize,
    footer_y: u16,
    footer_fits: bool,
    // preview
    pv: Preview,
    term_w: u16,
    term_h: u16,
    nvim_cursor: Option<(u16, u16)>,
    look: Option<crate::lvim::Look>,
    default_title: String,
    buttons: Vec<view_lvim::Button>,
}

pub(super) struct Preview {
    template: Option<String>,
    spec: crate::preview::Spec,
    /// `change-preview-window` alternatives and the current index.
    cycle: Vec<String>,
    cycle_idx: usize,
    visible: bool,
    runner: Option<crate::preview::Runner>,
    version: u64,
    last_cmd: Option<String>,
    /// What the preview shows: the item (index and text: F3 / F4 and reloads
    /// put other items at the same index), the query, the selection.
    key: Option<(Option<u32>, String, String, usize)>,
    offset: usize,
    label: String,
    rect: Option<Rect>,
    /// the area the preview was split off (its size is a share of it), and whether its edge is being dragged
    area: Option<Rect>,
    drag: bool,
    /// the size moved with the keys and not yet written, and when the file was last written (see `divider-*`)
    size_dirty: bool,
    size_saved: Option<Instant>,
    /// Inner size of the last drawn preview (cols, lines).
    size: (u16, u16),
    force: bool,
    oneoff: Option<String>,
    // embedded Neovim
    nvim_template: Option<String>,
    nvim: Option<crate::nvim::Nvim>,
    nvim_failed: bool,
    focus: bool,
    wake: std::sync::mpsc::Sender<()>,
    /// Inner rect of the last drawn preview.
    inner: Option<Rect>,
    /// The provider chain, tried in order for each item.
    order: Vec<pview::Provider>,
    active: Option<pview::Provider>,
    /// Template that turns an item into `file[:line[:col]]` for the file preview.
    file_template: String,
    file_target: Option<(String, usize, usize)>,
    loader: crate::filepreview::Loader,
    /// Draws the plugins' answers (and keeps the picture on the terminal): the component lvc uses too.
    drawer: lvim_fuzzy_ui::provided::Drawer,
    /// The position in a multi-page file (a PDF page, a video step) the plugin is asked for; 0 = the first.
    plug_skip: u32,
    /// What the plugin provider shows, and how many rows it has (for scrolling).
    list_target: Option<std::path::PathBuf>,
    rows_total: usize,
    /// Set as the preview is drawn: in the lvim look (the look is passed, `self.look` is taken while the frame is drawn) the panel's background, which the
    /// preview's scrollbar stands on in the blank column outside the preview; None in the plain look.
    bar_panel_bg: Option<Option<crossterm::style::Color>>,
    /// The preview plugins (None = none installed), the request in flight (ticket, file) and the answers so far.
    plug: Option<lvim_fuzzy_plugin::Previewer>,
    /// The plugin host the open plugin is asked through (Enter on a file); made at the first Enter.
    opener: Option<lvim_fuzzy_plugin::Previewer>,
    plug_wait: Option<(u64, std::path::PathBuf, u32)>,
    plug_answers: std::collections::HashMap<(std::path::PathBuf, u32), lvim_fuzzy_ui::provided::Answer>,
}

struct Term {
    out: BufWriter<File>,
    fullscreen: bool,
    height: u16,
    width: u16,
    prev: Option<Canvas>,
    pos: Positioner,
    mouse: bool,
    /// The lvim caret colour (a thin bar), restored on exit.
    caret: Option<Color>,
    /// The height an inline finder may grow to when it needs room (the preview opened in the minimal look): what `--height` allows.
    max_height: u16,
    /// `--exit-cursor=hidden`.
    keep_cursor_hidden: bool,
}

impl Term {
    /// Make the inline finder taller: the lines it needs are made below it (the terminal scrolls when it is at the bottom).
    fn grow_to(&mut self, h: u16) -> std::io::Result<()> {
        if self.fullscreen || h <= self.height {
            return Ok(());
        }
        self.pos.goto(&mut self.out, 0, self.height.saturating_sub(1))?;
        for _ in self.height..h {
            self.out.write_all(b"\n")?;
        }
        self.pos.row = h - 1;
        self.height = h;
        self.prev = None;
        self.out.flush()
    }

    fn open(opts: &Options, needed: Option<u16>, caret: Option<Color>) -> std::io::Result<Term> {
        let tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
        let mut out = BufWriter::with_capacity(1 << 16, tty);
        // a terminal that reports no size (0x0: a fresh pty) gets the usual 80x24
        let (cols, rows) = match terminal::size()? {
            (0, _) | (_, 0) => (80, 24),
            s => s,
        };
        let fullscreen = opts.height.is_none();
        let mut max_height = rows;
        let height = match opts.height {
            None => rows,
            Some(h) => {
                let mut v = if h.percent {
                    let v = (rows as u32 * h.value as u32 / 100) as u16;
                    v.max(opts.min_height.min(rows))
                } else {
                    h.value
                };
                if h.negative {
                    v = rows.saturating_sub(h.value);
                }
                max_height = v.clamp(1, rows);
                if h.adaptive
                    && let Some(n) = needed
                {
                    v = v.min(n);
                }
                v.clamp(1, rows)
            }
        };
        terminal::enable_raw_mode()?;
        if fullscreen {
            if keeps_screen() {
                execute!(out, cursor::Hide)?;
            } else {
                execute!(out, terminal::EnterAlternateScreen, cursor::Hide)?;
            }
        } else {
            // make room below the cursor: print newlines, then go back up
            queue!(out, cursor::Hide, cursor::MoveToColumn(0))?;
            for _ in 1..height {
                out.write_all(b"\n")?;
            }
            if height > 1 {
                queue!(out, cursor::MoveUp(height - 1))?;
            }
            out.flush()?;
        }
        if opts.mouse {
            execute!(out, EnableMouseCapture)?;
        }
        // Shift-Enter (cd) and the other modified keys reach lfz as themselves, in a terminal that can
        lvim_fuzzy_ui::keyboard::push(&mut out);
        out.flush()?;
        if let Some(c) = caret {
            set_caret(&mut out, c)?;
        }
        Ok(Term {
            out,
            fullscreen,
            height,
            width: cols,
            prev: None,
            pos: Positioner {
                inline: !fullscreen,
                row: 0,
            },
            mouse: opts.mouse,
            caret,
            max_height,
            keep_cursor_hidden: opts.exit_cursor_hidden,
        })
    }

    fn suspend(&mut self) -> std::io::Result<()> {
        // the cursor is out of sight while the finder is taken down (it would be seen on the rows it clears)
        queue!(self.out, cursor::Hide)?;
        lvim_fuzzy_ui::keyboard::pop(&mut self.out);
        if self.mouse {
            queue!(self.out, DisableMouseCapture)?;
        }
        if self.caret.is_some() {
            // back to the terminal's own cursor colour and shape
            self.out.write_all(b"\x1b]112\x07")?;
            queue!(self.out, cursor::SetCursorStyle::DefaultUserShape)?;
        }
        self.clear_region()?;
        if self.fullscreen && !keeps_screen() {
            queue!(self.out, terminal::LeaveAlternateScreen)?;
        }
        if !self.keep_cursor_hidden {
            queue!(self.out, cursor::Show)?;
        }
        self.out.flush()?;
        terminal::disable_raw_mode()?;
        Ok(())
    }

    fn resume(&mut self) -> std::io::Result<()> {
        terminal::enable_raw_mode()?;
        if self.fullscreen {
            if !keeps_screen() {
                queue!(self.out, terminal::EnterAlternateScreen)?;
            }
        } else {
            queue!(self.out, cursor::MoveToColumn(0))?;
            for _ in 1..self.height {
                self.out.write_all(b"\n")?;
            }
            if self.height > 1 {
                queue!(self.out, cursor::MoveUp(self.height - 1))?;
            }
            self.pos.row = 0;
        }
        if self.mouse {
            queue!(self.out, EnableMouseCapture)?;
        }
        queue!(self.out, cursor::Hide)?;
        lvim_fuzzy_ui::keyboard::push(&mut self.out);
        if let Some(c) = self.caret {
            set_caret(&mut self.out, c)?;
        }
        self.out.flush()?;
        // a program that ran full screen in between: kitty dropped an alternate screen's images, send them again
        lvim_fuzzy_ui::iconimg::invalidate();
        self.prev = None;
        Ok(())
    }

    fn clear_region(&mut self) -> std::io::Result<()> {
        if self.fullscreen {
            return Ok(());
        }
        queue!(
            self.out,
            crossterm::style::SetAttribute(crossterm::style::Attribute::Reset)
        )?;
        for y in 0..self.height {
            self.pos.goto(&mut self.out, 0, y)?;
            queue!(self.out, terminal::Clear(terminal::ClearType::UntilNewLine))?;
        }
        self.pos.goto(&mut self.out, 0, 0)?;
        Ok(())
    }

    fn close(mut self) -> std::io::Result<()> {
        // the icon images go with the finder
        let gone = lvim_fuzzy_ui::iconimg::cleanup();
        if !gone.is_empty() {
            self.out.write_all(&gone)?;
        }
        self.suspend()
    }
}

/// A thin bar caret in `c` (OSC 12 sets the cursor colour).
fn set_caret<W: Write>(out: &mut W, c: Color) -> std::io::Result<()> {
    if let Color::Rgb { r, g, b } = c {
        write!(out, "\x1b]12;#{r:02x}{g:02x}{b:02x}\x07")?;
    }
    queue!(out, cursor::SetCursorStyle::SteadyBar)?;
    out.flush()
}

fn border_chars(b: BorderStyle, unicode: bool) -> [&'static str; 8] {
    if !unicode {
        return canvas::BORDER_ASCII;
    }
    match b {
        BorderStyle::Sharp => canvas::BORDER_SHARP,
        BorderStyle::Bold => canvas::BORDER_BOLD,
        BorderStyle::Double => canvas::BORDER_DOUBLE,
        BorderStyle::Dashed => canvas::BORDER_DASHED,
        BorderStyle::Block => canvas::BORDER_BLOCK,
        BorderStyle::ThinBlock => canvas::BORDER_THINBLOCK,
        _ => canvas::BORDER_ROUNDED,
    }
}

/// [top, right, bottom, left]
fn border_sides(b: BorderStyle) -> [bool; 4] {
    match b {
        BorderStyle::None => [false; 4],
        BorderStyle::Horizontal => [true, false, true, false],
        BorderStyle::Vertical => [false, true, false, true],
        BorderStyle::Top => [true, false, false, false],
        BorderStyle::Bottom => [false, false, true, false],
        BorderStyle::Left => [false, false, false, true],
        BorderStyle::Right => [false, true, false, false],
        BorderStyle::Line => [true, false, false, false],
        _ => [true; 4],
    }
}

/// The lvim look's colours mapped onto the fzf colour slots, so the shared
/// pieces (help popup, toasts, the fzf-style list) wear the theme too.
fn lvim_slots(t: &mut Theme, l: &crate::lvim::Look) {
    let fg_only = |s: Style| Style { bg: None, ..s };
    t.set(Slot::Fg, l.panel);
    t.set(Slot::Bg, l.panel);
    t.set(Slot::Hl, fg_only(l.matched));
    t.set(Slot::CurFg, fg_only(l.sel[0]));
    t.set(Slot::CurBg, l.sel[0]);
    t.set(Slot::CurHl, fg_only(l.matched));
    t.set(
        Slot::Gutter,
        Style {
            bg: l.panel.bg,
            ..Style::default()
        },
    );
    t.set(Slot::Query, fg_only(l.input));
    t.set(Slot::Prompt, l.prompt);
    t.set(Slot::Pointer, fg_only(l.marker));
    t.set(Slot::Marker, fg_only(l.marker));
    t.set(Slot::Info, fg_only(l.count));
    t.set(Slot::Spinner, fg_only(l.counter));
    t.set(Slot::Border, fg_only(l.border));
    t.set(Slot::Separator, fg_only(l.border));
    t.set(Slot::Scrollbar, fg_only(l.scrollbar));
    t.set(Slot::PreviewScrollbar, fg_only(l.preview_scrollbar));
    t.set(Slot::Label, fg_only(l.title));
    t.set(Slot::Header, fg_only(l.title));
    t.set(Slot::Ghost, fg_only(l.count));
    t.set(Slot::PreviewFg, l.panel);
    t.set(Slot::PreviewBg, l.panel);
}

/// The keys: the defaults, then what the options bind.
pub(crate) fn build_keymap(
    opts: &Options,
) -> Result<std::collections::HashMap<String, Vec<keys::Action>>, String> {
    let mut keymap = keys::default_keymap();
    if opts.history.is_some() {
        // as fzf: with --history, ctrl-p / ctrl-n walk the earlier queries
        for (k, a) in [("ctrl-p", "prev-history"), ("ctrl-n", "next-history")] {
            keymap.insert(
                k.into(),
                vec![keys::Action {
                    name: a.into(),
                    arg: None,
                }],
            );
        }
    }
    // standalone at a terminal: enter opens the item in the editor and ctrl-y
    // prints it; when the output is read by a program (shell keys, `$(lfz)`),
    // enter prints, as in fzf
    {
        use std::io::IsTerminal;
        let act = |n: &str| {
            vec![keys::Action {
                name: n.into(),
                arg: None,
            }]
        };
        if std::io::stdout().is_terminal() && opts.expect.is_empty() {
            keymap.insert("enter".into(), act("open"));
        }
        keymap.insert("ctrl-y".into(), act("accept"));
        keymap.insert("shift-enter".into(), act("cd"));
    }
    for b in &opts.binds {
        for (k, acts) in keys::parse_binds(b)? {
            keymap.insert(k, acts);
        }
    }
    Ok(keymap)
}

/// The preview window: no border (the panels are divided by a line), then
/// --preview-window.
pub(crate) fn preview_spec(opts: &Options) -> Result<crate::preview::Spec, String> {
    let mut spec = crate::preview::Spec::default();
    spec.border = BorderStyle::None;
    for w in &opts.preview_window {
        let first = w.split('|').next().unwrap_or("");
        spec.apply(first)?;
    }
    Ok(spec)
}

pub fn build_theme(opts: &Options, look: Option<&crate::lvim::Look>) -> Result<Theme, String> {
    let mut t = Theme::base("dark").unwrap();
    // the colours of the lvim theme; --color still wins
    match look {
        Some(l) => lvim_slots(&mut t, l),
        None => {
            let (th, _) = crate::lvim::resolve_theme(opts.theme.as_deref());
            lvim_slots(&mut t, &crate::lvim::Look::new(th));
        }
    }
    for c in &opts.colors {
        t.apply(c)?;
    }
    if opts.no_color {
        t.mono = true;
    }
    Ok(t)
}

/// The preview provider chain: `--preview-order`, or nvim (if asked for),
/// then the command (if any), then the built-in file preview (lvim look).
/// How an item names the file to preview: `--target` (modes set it), the
/// `--preview-nvim` template, or the item itself.
fn preview_target(o: &Options) -> String {
    o.target
        .clone()
        .or_else(|| o.preview_nvim.clone())
        .unwrap_or_else(|| "{}".into())
}

/// The preview plugins of `~/.config/lvim-plugins` (or `$LVIM_PLUGIN_ROOTS`, `:`-separated), set up with lfz's own
/// `[plugins.<name>]` overrides. None when no plugin previews anything.
/// `LVIM_FUZZY_KEEP_SCREEN` set (by lvim-fuzzy-commander, which runs the finder from its own full-screen view): draw in the
/// screen the program was started in — do not enter the alternate screen and do not leave it. A program that is
/// already in the alternate screen would otherwise show the shell for a moment on the way in and out.
fn keeps_screen() -> bool {
    std::env::var_os("LVIM_FUZZY_KEEP_SCREEN").is_some_and(|v| !v.is_empty())
}

fn preview_plugins(wake: std::sync::mpsc::Sender<()>) -> Option<lvim_fuzzy_plugin::Previewer> {
    let own = crate::PLUGINS.get().cloned().unwrap_or_default();
    let p = crate::plugin_host("lvim-fuzzy-cli", &own, wake);
    (!p.is_empty()).then_some(p)
}

fn preview_order(o: &Options) -> Result<Vec<pview::Provider>, String> {
    if let Some(v) = &o.preview_order {
        return v.iter().map(|s| pview::Provider::parse(s)).collect();
    }
    // 1. the preview plugins (everything they have an engine for: pictures, tables, directories ...), 2. Neovim (files, when it is
    // installed or asked for), 3. the built-in file preview (lvim look), 4. the --preview command
    // (everything that is not a file)
    // (a directory, a table, an archive, a binary file and text come from the preview plugin, first in the chain)
    let mut v = vec![
        pview::Provider::Plugin,
    ];
    if o.preview_nvim.is_some()
        || crate::modes::on_path(o.nvim.split_whitespace().next().unwrap_or("nvim"))
    {
        v.push(pview::Provider::Nvim);
    }
    v.push(pview::Provider::File);
    if o.preview.is_some() {
        v.push(pview::Provider::Command);
    }
    Ok(v)
}

/// "Files" when the finder walks the directory itself, "Search" otherwise.
fn default_title(o: &Options) -> String {
    use std::io::IsTerminal;
    let cmd = std::env::var("LVIM_FUZZY_DEFAULT_COMMAND")
        .ok()
        .filter(|s| !s.is_empty());
    if std::io::stdin().is_terminal() && cmd.is_none() {
        if o.walker.dir && !o.walker.file {
            "Directories".into()
        } else {
            "Files".into()
        }
    } else {
        "Search".into()
    }
}

pub fn stage(name: &str) {
    if let Ok(p) = std::env::var("LVIM_FUZZY_STAGES")
        && let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
    {
        use std::io::Write as _;
        static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        let t0 = *T0.get_or_init(Instant::now);
        let _ = writeln!(f, "{:>8.1} ms  {name}", t0.elapsed().as_secs_f64() * 1000.0);
    }
}

pub fn run(opts: Options) -> Result<i32, String> {
    stage("run");
    let (bg_tx, bg_rx) = channel::<(u64, String, String)>();
    let mut theme_note: Option<String> = None;
    let look = {
        let (t, note) = crate::lvim::resolve_theme(opts.theme.as_deref());
        if let Some(n) = note {
            theme_note = Some(n);
        }
        Some(crate::lvim::Look::new(t))
    };
    stage("look");
    let theme = build_theme(&opts, look.as_ref())?;
    // asked now, before the keyboard is read (the terminal answers on stdin)
    let popup_bg = {
        use std::io::IsTerminal;
        let term_bg = if std::io::stderr().is_terminal() || std::io::stdout().is_terminal() {
            crate::lvim::terminal_colors().map(|t| t.bg)
        } else {
            None
        };
        term_bg
            .or(look.as_ref().and_then(|l| l.panel.bg))
            .or(theme.get(crate::theme::Slot::Bg).bg)
            .map(crate::lvim::shade)
    };
    let keymap = build_keymap(&opts)?;
    let mut expect = Vec::new();
    for k in &opts.expect {
        expect.push(keys::canonical_key(k)?);
    }
    let (wake_tx, wake_rx) = channel::<()>();
    let engine = Engine::new(crate::match_settings(&opts), wake_tx.clone());
    let mut spec = preview_spec(&opts)?;
    // what this look left last time: its preview shown or not, and on which side (a side the options name wins)
    let saved = crate::look_state::get(opts.minimal);
    if opts.preview_window.is_empty()
        && let Some(side) = saved.side
    {
        spec.pos = side;
    }
    if opts.preview_window.is_empty()
        && let Some(n) = saved.size
    {
        spec.percent = true;
        spec.size = n.clamp(10, 90);
    }
    let pv = Preview {
        runner: Some(crate::preview::Runner::new(
            crate::shell_command(&opts),
            wake_tx.clone(),
        )),
        template: opts.preview.clone(),
        // (no word yet: the minimal look without the preview, the full one with it), unless the options hide it
        visible: !spec.hidden && saved.visible.unwrap_or(!opts.minimal),
        spec,
        cycle: Vec::new(),
        cycle_idx: 0,
        version: 0,
        last_cmd: None,
        key: None,
        offset: 0,
        label: String::new(),
        rect: None,
        area: None,
        drag: false,
        size_dirty: false,
        size_saved: None,
        size: (0, 0),
        force: false,
        oneoff: None,
        nvim_template: Some(preview_target(&opts)),
        nvim: None,
        nvim_failed: false,
        focus: false,
        wake: wake_tx.clone(),
        inner: None,
        order: preview_order(&opts)?,
        active: None,
        file_template: preview_target(&opts),
        file_target: None,
        loader: crate::filepreview::Loader::new(wake_tx.clone()),
        drawer: lvim_fuzzy_ui::provided::Drawer::new(true),
        plug_skip: 0,
        list_target: None,
        rows_total: 0,
        bar_panel_bg: None,
        plug: preview_plugins(wake_tx.clone()),
        opener: None,
        plug_wait: None,
        plug_answers: Default::default(),
    };
    stage("before source");
    let (feed_tx, feed_rx) = channel();
    // the source's process (a command): stopped by a reload or on exit
    let reload_child: Arc<Mutex<Option<std::process::Child>>> = Arc::new(Mutex::new(None));
    let shared_builder = Arc::new(std::sync::RwLock::new(crate::builder(&opts)));
    crate::start_source(
        &opts,
        Feed {
            tx: feed_tx.clone(),
            epoch: 0,
            // --header-lines are kept as the raw text: built here, not there
            build: (opts.header_lines == 0).then(|| shared_builder.clone()),
            child: Some(reload_child.clone()),
        },
    );
    let builder = crate::builder(&opts);
    let shell = crate::shell_command(&opts);
    let query: Vec<char> = opts.query.chars().collect();
    let mut app = App {
        prompt: opts.prompt.clone(),
        ghost: opts.ghost.clone(),
        header: opts.header.clone(),
        header_left: opts.header_lines,
        sort: opts.sort,
        track: opts.track,
        tracked: None,
        history: opts
            .history
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| {
                t.lines()
                    .filter(|l| !l.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        hist_pos: None,
        hist_draft: String::new(),
        jump: None,
        raw_set: None,
        refocus: false,
        pending_events: Vec::new(),
        live_stash: None,
        live_query: String::new(),
        tracked_key: None,
        header_hidden: false,
        search_override: None,
        excluded: HashSet::new(),
        bg_tx: bg_tx.clone(),
        bg_rx,
        bg_gen: 0,
        bg_child: Arc::new(Mutex::new(None)),
        listen_rx: None,
        listen_addr: None,
        scope: crate::match_settings(&opts).scope,
        grep_input: false,
        // an explicit --scope, or the fzf look, leaves the scope alone
        scope_checked: opts.scope.is_some(),
        search: !opts.disabled,
        qcur: query.len(),
        query,
        theme,
        look,
        default_title: default_title(&opts),
        buttons: Vec::new(),
        base_keymap: keymap.clone(),
        keymap,
        engine,
        builder,
        shared_builder,
        shell,
        feed_tx,
        epoch: 0,
        loading: true,
        header_items: Vec::new(),
        reload_child,
        yank: String::new(),
        hits: Hits::All {
            count: 0,
            tac: opts.tac,
        },
        requested: 0,
        received: 0,
        asked: Vec::new(),
        shown_query: String::new(),
        last_shown: Instant::now(),
        dirty_items: false,
        last_request: Instant::now(),
        cursor: 0,
        offset: 0,
        selected: Vec::new(),
        selset: HashSet::new(),
        last_focus: None,
        row_map: Vec::new(),
        list_rows: 0,
        prints: Vec::new(),
        toast: None,
        popup_bg,
        help: None,
        menu: None,
        theme_menu: None,
        print_folder: false,
        theme_name: opts
            .theme
            .clone()
            .unwrap_or_else(|| crate::lvim::TERMINAL.to_string()),
        spinner: 0,
        load_fired: false,
        temps: Vec::new(),
        last_click: None,
        tip: false,
        tip_was: false,
        cursor_cut: false,
        footer_off: 0,
        footer_y: 0,
        footer_fits: true,
        pv,
        term_w: 0,
        term_h: 0,
        nvim_cursor: None,
        opts,
    };
    if let Some(spec) = app.opts.listen.clone() {
        let (ltx, lrx) = channel();
        let addr = crate::listen::start(&spec, ltx)?;
        app.listen_rx = Some(lrx);
        app.listen_addr = Some(addr);
    }
    if let Some(n) = theme_note {
        app.toast(n, false);
    }
    if !app.opts.unsupported.is_empty() {
        let list = app
            .opts
            .unsupported
            .iter()
            .map(|s| format!("--{s}"))
            .collect::<Vec<_>>()
            .join(", ");
        app.toast(format!("not implemented yet: {list}"), true);
    }

    // --select-1 / --exit-0 / --sync / adaptive height: wait for the input first
    let wait_all = app.opts.select_1 || app.opts.exit_0 || app.opts.sync;
    let adaptive = app.opts.height.is_some_and(|h| h.adaptive);
    if wait_all || adaptive {
        let deadline = if wait_all {
            None
        } else {
            Some(Instant::now() + Duration::from_millis(200))
        };
        while app.loading {
            let ev = match deadline {
                None => feed_rx.recv().ok(),
                Some(d) => feed_rx
                    .recv_timeout(d.saturating_duration_since(Instant::now()))
                    .ok(),
            };
            match ev {
                Some((ep, e)) => app.ingest(ep, e),
                None => break,
            }
        }
        stage("loaded");
        if wait_all {
            let items = app.engine.items.read().unwrap();
            let q: String = app.query_str_for_search();
            let hits =
                crate::engine::run_match(&items, &q, &crate::match_settings(&app.opts), None)
                    .unwrap();
            if hits.len() == 1 && app.opts.select_1 {
                let line = app.output_line(items.at(hits.get(0).unwrap() as usize));
                drop(items);
                return app.finish(Outcome::Accept { key: None }, vec![line]);
            }
            if hits.len() == 0 && app.opts.exit_0 {
                drop(items);
                return app.finish(Outcome::Accept { key: None }, vec![]);
            }
            // --sync: the start event sees the finished result
            drop(items);
            app.shown_query = app.query_string();
            app.apply_hits(hits);
            stage("matched");
        }
    }
    if let Ok(path) = std::env::var("LVIM_FUZZY_DUMP") {
        return app.dump(&path, &feed_rx);
    }
    let needed = adaptive.then(|| app.needed_height());
    let mut term = Term::open(
        &app.opts,
        needed,
        app.look.as_ref().and_then(|l| l.caret.bg),
    )
    .map_err(|e| format!("cannot open the terminal: {e}"))?;
    // file icons as Material images where the terminal takes them: in raw mode, before input is read
    if app.opts.icons
        && let Some(m) = lvim_fuzzy_ui::iconimg::Mode::parse(&app.opts.icon_mode)
    {
        lvim_fuzzy_ui::iconimg::init(m, app.opts.zellij_images);
    }
    app.request();
    stage("terminal open");
    app.pending_events.push("start");
    let outcome = app.event_loop(&mut term, &feed_rx, &wake_rx, &expect);
    let outcome = match outcome {
        Ok(o) => o,
        Err(e) => {
            let _ = term.close();
            return Err(e.to_string());
        }
    };
    let _ = term.close();
    stage("loop done");
    if let Outcome::Switch(mode, query) = &outcome {
        app.stop_commands();
        app.quit_nvim();
        for t in app.temps.drain(..) {
            let _ = std::fs::remove_file(t);
        }
        return menu::switch_mode(mode, query);
    }
    let lines = match &outcome {
        Outcome::Accept { .. } => app.accepted_lines(),
        _ => Vec::new(),
    };
    app.finish(outcome, lines)
}

impl App {
    /// Take a new result: raw mode lists everything (remembering what
    /// matches), --track / a one-shot refocus keep the focused item.
    fn apply_hits(&mut self, hits: Hits) {
        self.hits = hits;
        if !self.excluded.is_empty() {
            // exclude: take the excluded items out of the result
            let kept: Vec<crate::engine::Hit> = (0..self.hits.len())
                .filter_map(|i| self.hits.get(i))
                .filter(|i| !self.excluded.contains(i))
                .map(|idx| crate::engine::Hit { idx, score: 0 })
                .collect();
            self.hits = Hits::List(Arc::new(kept));
        }
        if self.opts.raw {
            // raw: list everything, remember what matches
            let set: std::collections::HashSet<u32> = (0..self.hits.len())
                .filter_map(|i| self.hits.get(i))
                .collect();
            let count = self.engine.items.read().unwrap().len();
            self.raw_set = (!self.query.is_empty() && self.search).then_some(set);
            self.hits = Hits::All {
                count,
                tac: self.opts.tac,
            };
        } else {
            self.raw_set = None;
        }
        let refocus = std::mem::take(&mut self.refocus);
        if self.track || refocus {
            if let Some(key) = self.tracked_key.clone() {
                // --id-nth: find the item by its identity, wherever it is now
                let items = self.engine.items.read().unwrap();
                let found = (0..self.hits.len()).find(|&p| {
                    self.hits
                        .get(p)
                        .and_then(|i| items.get(i as usize))
                        .is_some_and(|it| {
                            self.opts.id_nth.as_ref().is_some_and(|r| {
                                crate::fields::extract(it.raw(), &self.opts.delimiter, r) == key
                            })
                        })
                });
                if let Some(p) = found {
                    self.cursor = p;
                }
            } else if let Some(t) = self.tracked
                && let Some(p) = self.hits.position(t)
            {
                self.cursor = p;
            }
        }
    }

    /// Handle what came in on --listen: run actions, answer state queries.
    fn serve_listen(&mut self, term: &mut Term) -> std::io::Result<Option<Outcome>> {
        let reqs: Vec<crate::listen::Req> = match &self.listen_rx {
            Some(rx) => rx.try_iter().collect(),
            None => return Ok(None),
        };
        for req in reqs {
            match req {
                crate::listen::Req::Actions(body, reply) => {
                    match keys::parse_binds(&format!("x:{body}")) {
                        Ok(b) => {
                            let acts = b.into_iter().next().map(|(_, a)| a).unwrap_or_default();
                            if !self.opts.listen_unsafe
                                && let Some(a) =
                                    acts.iter().find(|a| crate::listen::is_unsafe(&a.name))
                            {
                                let _ =
                                    reply.send(Some(format!("{} needs --listen-unsafe", a.name)));
                                continue;
                            }
                            let _ = reply.send(None);
                            if let Some(o) = self.run_actions(&acts, Some(term))? {
                                return Ok(Some(o));
                            }
                        }
                        Err(e) => {
                            let _ = reply.send(Some(e));
                        }
                    }
                }
                crate::listen::Req::State(limit, offset, reply) => {
                    let _ = reply.send(self.state_json(limit, offset));
                }
            }
        }
        Ok(None)
    }

    /// The state as JSON (the shape fzf's --listen GET returns).
    fn state_json(&self, limit: usize, offset: usize) -> String {
        use crate::listen::json_str;
        let items = self.engine.items.read().unwrap();
        let item = |i: u32| -> String {
            match items.get(i as usize) {
                Some(it) => format!("{{\"index\":{},\"text\":{}}}", it.index, json_str(it.raw())),
                None => "null".into(),
            }
        };
        let matches: Vec<String> = (offset..offset.saturating_add(limit).min(self.hits.len()))
            .filter_map(|i| self.hits.get(i))
            .map(item)
            .collect();
        let selected: Vec<String> = self.selected.iter().map(|&i| item(i)).collect();
        format!(
            "{{\"reading\":{},\"progress\":100,\"query\":{},\"position\":{},\"sort\":{},\"totalCount\":{},\"matchCount\":{},\"current\":{},\"matches\":[{}],\"selected\":[{}]}}",
            self.loading,
            json_str(&self.query_string()),
            self.cursor,
            self.sort,
            items.len(),
            self.matched_count(),
            self.current_index()
                .map(item)
                .unwrap_or_else(|| "null".into()),
            matches.join(","),
            selected.join(","),
        )
    }

    /// The --id-nth identity of item `idx` (None without --id-nth).
    pub(super) fn id_key(&self, idx: Option<u32>) -> Option<String> {
        let r = self.opts.id_nth.as_ref()?;
        let items = self.engine.items.read().unwrap();
        let it = items.get(idx? as usize)?;
        Some(crate::fields::extract(it.raw(), &self.opts.delimiter, r))
    }

    /// Add the query to --history (unless empty or a repeat), keep the last
    /// --history-size entries.
    fn save_history(&mut self) {
        let Some(path) = self.opts.history.clone() else {
            return;
        };
        let q = self.query_string();
        if q.is_empty() || self.history.last() == Some(&q) {
            return;
        }
        self.history.push(q);
        let size = self.opts.history_size.max(1);
        if self.history.len() > size {
            let drop = self.history.len() - size;
            self.history.drain(..drop);
        }
        let _ = std::fs::write(&path, self.history.join("\n") + "\n");
    }

    /// Development aid: render frames into a file instead of the terminal
    /// (`LVIM_FUZZY_DUMP=file`, size from `LVIM_FUZZY_DUMP_SIZE=WxH`).
    fn dump(mut self, path: &str, feed_rx: &Receiver<(u64, Event)>) -> Result<i32, String> {
        let size = std::env::var("LVIM_FUZZY_DUMP_SIZE").unwrap_or_else(|_| "120x30".into());
        let (w, h) = size
            .split_once('x')
            .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
            .unwrap_or((120, 30));
        let out = File::create(path).map_err(|e| e.to_string())?;
        let mut term = Term {
            out: BufWriter::new(out),
            fullscreen: true,
            height: h,
            width: w,
            prev: None,
            pos: Positioner {
                inline: false,
                row: 0,
            },
            mouse: false,
            caret: None,
            max_height: h,
            keep_cursor_hidden: false,
        };
        while self.loading {
            match feed_rx.recv() {
                Ok((ep, e)) => self.ingest(ep, e),
                Err(_) => break,
            }
        }
        self.request();
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.received < self.requested && Instant::now() < deadline {
            let res = self.engine.result.lock().unwrap().take();
            if let Some((g, hits, _)) = res {
                self.received = g;
                self.shown(g);
                self.apply_hits(hits);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // actions to run before drawing (e.g. `mode-menu`), fzf --bind syntax
        if let Ok(acts) = std::env::var("LVIM_FUZZY_DUMP_ACTIONS")
            && let Ok(b) = keys::parse_binds(&format!("x:{acts}"))
            && let Some((_, a)) = b.into_iter().next()
        {
            let _ = self.run_actions(&a, None);
        }
        // a few frames so async previews can land
        for _ in 0..6 {
            self.draw(&mut term).map_err(|e| e.to_string())?;
            self.update_preview();
            std::thread::sleep(Duration::from_millis(150));
            term.prev = None;
        }
        self.draw(&mut term).map_err(|e| e.to_string())?;
        self.quit_nvim();
        Ok(0)
    }

    /// What the open plugin says to do with the file `path` (asked through a host of its own, made on first use).
    fn open_decision(&mut self, path: &str) -> lvim_fuzzy_ui::open::Open {
        let opener = self.pv.opener.get_or_insert_with(|| {
            let (wake, _rx) = std::sync::mpsc::channel();
            crate::plugin_host("lvim-fuzzy-cli", &crate::PLUGINS.get().cloned().unwrap_or_default(), wake)
        });
        crate::open_decision(opener, "lvim-fuzzy-cli", path)
    }

    fn toast(&mut self, text: String, error: bool) {
        self.toast = Some(Toast {
            text,
            error,
            until: Instant::now() + Duration::from_secs(4),
        });
    }

    fn query_string(&self) -> String {
        self.query.iter().collect()
    }

    /// What the rows and the preview mark: the query of the shown result and,
    /// filtering grep results (F3), the grep pattern too — as an exact term,
    /// since that is what found these lines.
    pub(crate) fn highlight_query(&self) -> String {
        let q = self.shown_query.trim();
        // F3 from live grep: the grep query kept aside; a grep ➤ filter mode:
        // the pattern it started with
        let g = if self.live_stash.is_some() {
            self.live_query.trim()
        } else {
            self.opts.grep_pattern.trim()
        };
        if g.is_empty() {
            return q.to_string();
        }
        let grep = format!("'{}", g.replace(' ', "\\ "));
        if q.is_empty() {
            grep
        } else {
            format!("{grep} {q}")
        }
    }

    /// The result of request `g` is shown: its query is what highlights.
    fn shown(&mut self, g: u64) {
        if let Some(p) = self.asked.iter().position(|(r, _)| *r == g) {
            self.shown_query = self.asked[p].1.clone();
            self.asked.drain(..=p);
        }
    }

    fn query_str_for_search(&self) -> String {
        if let Some(q) = &self.search_override {
            return q.clone();
        }
        if self.search {
            self.query_string()
        } else {
            String::new()
        }
    }

    fn needed_height(&self) -> u16 {
        let items = self.engine.items.read().unwrap().len();
        // the prompt row, and above and below it the blank ring (the minimal look has none)
        let chrome = if self.opts.minimal { 1 } else { 2 } + self.header.len() + self.header_items.len();
        (items + chrome).min(u16::MAX as usize) as u16
    }

    fn ingest(&mut self, epoch: u64, e: Event) {
        if epoch != self.epoch {
            return;
        }
        match e {
            Event::Reserve(n) => {
                self.engine.items.write().unwrap().reserve(n);
            }
            Event::Items(mut built) => {
                stage("ingest: got part");
                let mut items = self.engine.items.write().unwrap();
                let base = items.len() as u32;
                // numbered by the sender when it knew where they go; else here
                if built.items.first().is_some_and(|it| it.index != base) {
                    for (i, it) in built.items.iter_mut().enumerate() {
                        it.index = base + i as u32;
                    }
                }
                if !self.scope_checked
                    && let Some(it) = built.get(0)
                {
                    self.scope_checked = true;
                    if let Some((pe, _)) = it.grep
                        && std::path::Path::new(&it.display()[..pe as usize]).is_file()
                    {
                        self.grep_input = true;
                        self.scope = crate::engine::Scope::Line;
                    }
                }
                // (the first part becomes the list: moved, not copied)
                items.nth = self.builder.nth.is_some();
                items.extend(built);
                stage("ingest: appended");
                drop(items);
                self.after_ingest();
                stage("ingest: after");
            }
            Event::Lines(lines) => {
                let mut items = self.engine.items.write().unwrap();
                for l in lines {
                    if self.header_left > 0 {
                        self.header_left -= 1;
                        self.header_items.push(l);
                        continue;
                    }
                    let idx = items.len() as u32;
                    items.nth = self.builder.nth.is_some();
                    items.push(self.builder.build(idx, l));
                    if !self.scope_checked {
                        // grep results (`path:line:text` naming a real file): filter the
                        // line's text by default, not the path
                        self.scope_checked = true;
                        let it = items.at(idx as usize);
                        if let Some((pe, _)) = it.grep
                            && std::path::Path::new(&it.display()[..pe as usize]).is_file()
                        {
                            self.grep_input = true;
                            self.scope = crate::engine::Scope::Line;
                        }
                    }
                }
                drop(items);
                self.after_ingest();
            }
            Event::Eof => {
                stage("ingest: eof");
                self.loading = false;
                self.dirty_items = true;
            }
        }
    }

    /// After new items: --tail trimming, and a new match due.
    fn after_ingest(&mut self) {
        {
            let mut items = self.engine.items.write().unwrap();
            // --tail N: keep the last N items (dropped in chunks; indices renumbered)
            if let Some(n) = self.opts.tail
                && items.len() > n + (n / 10).max(1)
            {
                // renumbered: earlier matches no longer point at the same items
                self.engine
                    .resets
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                items.keep_last(n);
                self.selected.clear();
                self.selset.clear();
                self.tracked = None;
                self.hits = Hits::All {
                    count: 0,
                    tac: self.opts.tac,
                };
                self.cursor = 0;
            }
        }
        self.dirty_items = true;
    }

    fn request(&mut self) {
        self.requested = self
            .engine
            .request(&self.query_str_for_search(), self.sort, self.scope);
        self.asked.push((self.requested, self.query_string()));
        if self.asked.len() > 64 {
            self.asked.drain(..32);
        }
        self.last_request = Instant::now();
        self.dirty_items = false;
    }

    fn query_changed(&mut self) {
        self.search_override = None;
        if self.track {
            // keep the focused item through the new result
            self.tracked = self.current_index().or(self.tracked);
            self.tracked_key = self.id_key(self.tracked);
        }
        self.cursor = 0;
        self.offset = 0;
        self.request();
        if !self.pending_events.contains(&"change") {
            self.pending_events.push("change");
        }
    }

    fn current_index(&self) -> Option<u32> {
        self.hits.get(self.cursor)
    }

    fn output_line(&self, it: crate::item::ItemView) -> String {
        match &self.opts.accept_nth {
            Some(r) => crate::fields::extract(it.raw(), &self.opts.delimiter, r),
            None => it.raw().to_string(),
        }
    }

    fn accepted_lines(&self) -> Vec<String> {
        let lines = self.chosen_lines();
        // Shift-Enter: the folder of each (of a file, the one it is in)
        if self.print_folder {
            let base = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
            return lines.iter().map(|l| lvim_fuzzy_ui::printas::folder(l, &base)).collect();
        }
        lines
    }

    /// The choice as the finder has it (the marked items, or the one under the cursor), before Shift-Enter makes folders of them.
    fn chosen_lines(&self) -> Vec<String> {
        let items = self.engine.items.read().unwrap();
        if !self.selected.is_empty() {
            return self
                .selected
                .iter()
                .filter_map(|&i| items.get(i as usize))
                .map(|it| self.output_line(it))
                .collect();
        }
        match self.current_index().and_then(|i| items.get(i as usize)) {
            Some(it) => vec![self.output_line(it)],
            None => Vec::new(),
        }
    }

    /// Whether a part of the screen is hidden (--hide / [show]).
    pub(crate) fn hidden(&self, part: &str) -> bool {
        self.opts.hidden.contains(part)
    }

    /// Stop every command the finder runs in the background (the source,
    /// the preview, bg-transform): none outlives it, also
    /// not across `become` or a mode switch (exec).
    fn stop_commands(&self) {
        crate::reader::stop_slot(&self.reload_child);
        if let Some(r) = &self.pv.runner {
            r.stop();
        }
        crate::reader::stop_slot(&self.bg_child);
    }

    fn finish(mut self, outcome: Outcome, lines: Vec<String>) -> Result<i32, String> {
        // a size moved with the keys that has not been written yet
        if self.pv.size_dirty {
            crate::look_state::save(self.opts.minimal, self.pv.visible, self.pv.spec.pos, Some(self.pv.spec.size));
        }
        self.stop_commands();
        self.quit_nvim();
        for t in self.temps.drain(..) {
            let _ = std::fs::remove_file(t);
        }
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        let end: &[u8] = if self.opts.print0 { b"\0" } else { b"\n" };
        for p in &self.prints {
            let _ = out.write_all(p.as_bytes());
            let _ = out.write_all(end);
        }
        match outcome {
            Outcome::Abort => {
                let _ = out.flush();
                Ok(130)
            }
            Outcome::Become(cmd) => {
                let _ = out.flush();
                become_cmd(&self.shell, &cmd)
            }
            Outcome::Switch(mode, query) => {
                let _ = out.flush();
                menu::switch_mode(&mode, &query)
            }
            Outcome::Accept { key } => {
                self.save_history();
                if self.opts.print_query {
                    let _ = out.write_all(self.query_string().as_bytes());
                    let _ = out.write_all(end);
                }
                if !self.opts.expect.is_empty() {
                    let _ = out.write_all(key.unwrap_or_default().as_bytes());
                    let _ = out.write_all(end);
                }
                for l in &lines {
                    let _ = out.write_all(l.as_bytes());
                    let _ = out.write_all(end);
                }
                let _ = out.flush();
                Ok(if lines.is_empty() && self.prints.is_empty() {
                    1
                } else {
                    0
                })
            }
        }
    }

    fn event_loop(
        &mut self,
        term: &mut Term,
        feed_rx: &Receiver<(u64, Event)>,
        wake_rx: &Receiver<()>,
        expect: &[String],
    ) -> std::io::Result<Outcome> {
        let mut last_spin = Instant::now();
        let trace = std::env::var("LVIM_FUZZY_TRACE").ok();
        loop {
            let t0 = Instant::now();
            let mut marks: Vec<(&str, Duration)> = Vec::new();
            for ev in std::mem::take(&mut self.pending_events) {
                if let Some(o) = self.run_event(ev, term)? {
                    return Ok(o);
                }
            }
            // --listen requests
            if let Some(o) = self.serve_listen(term)? {
                return Ok(o);
            }
            // bg-transform results
            while let Ok((g, target, out)) = self.bg_rx.try_recv() {
                if g != self.bg_gen {
                    continue; // bg-cancel came after it was started
                }
                if target.is_empty() {
                    if let Ok(b) = keys::parse_binds(&format!("x:{out}"))
                        && let Some((_, acts)) = b.into_iter().next()
                        && let Some(o) = self.run_actions(&acts, Some(term))?
                    {
                        return Ok(o);
                    }
                } else {
                    self.apply_text(&target, out);
                }
            }
            // input
            marks.push(("pre", t0.elapsed()));
            let mut got_input = false;
            let mut batches = 0;
            // at most ~8 ms of new input per frame: a source that floods (a grep
            // for one letter) must not hold the keys and the screen
            let budget = Instant::now() + Duration::from_millis(8);
            // while a search holds the items (to read them), new input waits
            // for the next frame: taking the lock here would stop the keys
            let free = self.engine.items.try_write().is_ok();
            while free && let Ok((ep, e)) = feed_rx.try_recv() {
                self.ingest(ep, e);
                got_input = true;
                batches += 1;
                if Instant::now() >= budget {
                    break;
                }
            }
            marks.push(("ingest", t0.elapsed()));
            if self.dirty_items
                && (self.last_request.elapsed() > Duration::from_millis(60) || !self.loading)
            {
                self.request();
            }
            // results
            while wake_rx.try_recv().is_ok() {}
            self.poll_plugins();
            let result = self.engine.result.lock().unwrap().take();
            if let Some((g, hits, _n)) = result {
                // a result of a query already typed past: the rows stay until
                // the latest one (no screen in between), unless the latest is
                // slow to come — then the newest so far is shown
                let stale = g < self.requested
                    && (hits.len() == 0 || self.last_shown.elapsed() < Duration::from_millis(150));
                if stale && self.hits.len() > 0 {
                    self.received = self.received.max(g);
                } else if g >= self.received {
                    self.last_shown = Instant::now();
                    self.received = g;
                    self.shown(g);
                    self.apply_hits(hits);
                    let len = self.hits.len();
                    if self.cursor >= len {
                        self.cursor = len.saturating_sub(1);
                    }
                    if g == self.requested {
                        if !self.loading && !self.load_fired {
                            self.load_fired = true;
                            if let Some(o) = self.run_event("load", term)? {
                                return Ok(o);
                            }
                        }
                        if let Some(o) = self.run_event("result", term)? {
                            return Ok(o);
                        }
                        let ev = match len {
                            0 => Some("zero"),
                            1 => Some("one"),
                            _ => None,
                        };
                        if let Some(ev) = ev
                            && let Some(o) = self.run_event(ev, term)?
                        {
                            return Ok(o);
                        }
                    }
                }
            }
            if self.current_index() != self.last_focus {
                self.last_focus = self.current_index();
                if let Some(o) = self.run_event("focus", term)? {
                    return Ok(o);
                }
            }
            if self.loading && last_spin.elapsed() > Duration::from_millis(100) {
                self.spinner = (self.spinner + 1) % SPINNER.len();
                last_spin = Instant::now();
            }
            if self
                .toast
                .as_ref()
                .is_some_and(|t| Instant::now() > t.until)
            {
                self.toast = None;
            }
            marks.push(("results", t0.elapsed()));
            self.draw(term)?;
            marks.push(("draw", t0.elapsed()));
            self.update_preview();
            marks.push(("preview", t0.elapsed()));
            let _ = got_input;
            // terminal events
            if event::poll(Duration::from_millis(16))? {
                let ev = event::read()?;
                if let Some(o) = self.handle_event(ev, term, expect)? {
                    return Ok(o);
                }
                // drain queued events before the next frame (fast typing / paste)
                while event::poll(Duration::from_millis(0))? {
                    let ev = event::read()?;
                    if let Some(o) = self.handle_event(ev, term, expect)? {
                        return Ok(o);
                    }
                }
                marks.push(("keys", t0.elapsed()));
            }
            if let Some(path) = &trace {
                // the whole frame, the keys included (a key can wait on a lock)
                let busy = t0.elapsed();
                if busy > Duration::from_millis(16)
                    && let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                {
                    use std::io::Write as _;
                    let items = self.engine.items.read().unwrap().len();
                    let _ = writeln!(f, "{:?} items={items} batches={batches} {marks:?}", busy);
                }
            }
        }
    }

    fn run_event(&mut self, name: &str, term: &mut Term) -> std::io::Result<Option<Outcome>> {
        match self.keymap.get(name).cloned() {
            Some(acts) => self.run_actions(&acts, Some(term)),
            None => Ok(None),
        }
    }

    fn handle_event(
        &mut self,
        ev: TermEvent,
        term: &mut Term,
        expect: &[String],
    ) -> std::io::Result<Option<Outcome>> {
        match ev {
            TermEvent::Key(k) => {
                // LVIM_FUZZY_KEYLOG=FILE: every key as the terminal sent it and the
                // name it gets (for finding out why a binding does not fire)
                if let Ok(path) = std::env::var("LVIM_FUZZY_KEYLOG")
                    && let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                {
                    use std::io::Write as _;
                    let _ = writeln!(f, "{:?} -> {:?}", k, keys::key_name(&k));
                }
                if k.kind == KeyEventKind::Release {
                    return Ok(None);
                }
                // a key hides the tooltip (the key that shows it toggles from what it was)
                self.tip_was = std::mem::take(&mut self.tip);
                let Some(name) = keys::key_name(&k) else {
                    return Ok(None);
                };
                if let Some(accept) = self.jump.take() {
                    // a label picks its row; any other key cancels
                    let labels: Vec<char> = self.opts.jump_labels.chars().collect();
                    let pick = (name.chars().count() == 1)
                        .then(|| name.chars().next())
                        .flatten()
                        .and_then(|c| labels.iter().position(|l| *l == c))
                        .filter(|&i| i < self.list_rows && self.offset + i < self.hits.len());
                    match pick {
                        Some(i) => {
                            self.cursor = self.offset + i;
                            if accept {
                                return Ok(Some(Outcome::Accept { key: None }));
                            }
                            return self.run_event("jump", term);
                        }
                        None => return self.run_event("jump-cancel", term),
                    }
                }
                if self.help.is_some() {
                    self.help_key(&name);
                    return Ok(None);
                }
                if self.theme_menu.is_some() {
                    self.theme_key(&name);
                    return Ok(None);
                }
                if self.menu.is_some() {
                    if let Some(mode) = self.menu_key(&name) {
                        return Ok(Some(Outcome::Switch(mode, self.query_string())));
                    }
                    return Ok(None);
                }
                if self.pv.focus && self.pv.nvim.is_some() {
                    // only the focus toggle key itself is ours while Neovim has focus
                    let is_toggle = self
                        .keymap
                        .get(&name)
                        .is_some_and(|a| a.iter().any(|x| x.name == "toggle-preview-focus"));
                    if !is_toggle {
                        self.nvim_key(&k);
                        return Ok(None);
                    }
                }
                if expect.iter().any(|e| *e == name) {
                    return Ok(Some(Outcome::Accept { key: Some(name) }));
                }
                // while the preview is shown, ctrl-d / ctrl-u scroll it by half a page and ctrl-f / ctrl-b by a page (vim's keys): the keys
                // that edit the query otherwise (delete, clear, forward, back), unless they are bound to something else
                if self.preview_enabled() {
                    let (editing, scroll) = match name.as_str() {
                        "ctrl-d" => ("delete-char/eof", "preview-half-page-down"),
                        "ctrl-u" => ("unix-line-discard", "preview-half-page-up"),
                        "ctrl-f" => ("forward-char", "preview-page-down"),
                        "ctrl-b" => ("backward-char", "preview-page-up"),
                        _ => ("", ""),
                    };
                    if !scroll.is_empty()
                        && self.keymap.get(&name).is_some_and(|a| a.len() == 1 && a[0].name == editing)
                    {
                        let acts = vec![keys::Action {
                            name: scroll.into(),
                            arg: None,
                        }];
                        return self.run_actions(&acts, Some(term));
                    }
                }
                if let Some(acts) = self.keymap.get(&name).cloned() {
                    return self.run_actions(&acts, Some(term));
                }
                let ch = if name == "space" {
                    Some(' ')
                } else if name.chars().count() == 1 {
                    name.chars().next()
                } else {
                    None
                };
                if let Some(c) = ch {
                    self.query.insert(self.qcur, c);
                    self.qcur += 1;
                    self.query_changed();
                }
                Ok(None)
            }
            TermEvent::Paste(s) => {
                for c in s.chars().filter(|c| !c.is_control()) {
                    self.query.insert(self.qcur, c);
                    self.qcur += 1;
                }
                self.query_changed();
                Ok(None)
            }
            TermEvent::Mouse(m) => {
                if self.help.is_some() || self.menu.is_some() || self.theme_menu.is_some() {
                    // the wheel scrolls the open popup
                    let k = match m.kind {
                        crossterm::event::MouseEventKind::ScrollUp => Some("scroll-up"),
                        crossterm::event::MouseEventKind::ScrollDown => Some("scroll-down"),
                        _ => None,
                    };
                    if let Some(k) = k {
                        if self.help.is_some() {
                            self.help_key(k);
                        } else if self.theme_menu.is_some() {
                            self.theme_key(k);
                        } else {
                            self.menu_key(k);
                        }
                    }
                    return Ok(None);
                }
                // the edge between the list and the preview is dragged with the left button; where it is left is kept
                if let Some(a) = self.pv.area
                    && let Some(r) = self.pv.rect
                {
                    use crossterm::event::{MouseButton, MouseEventKind as K};
                    let pos = self.pv.spec.pos;
                    let near = |v: u16, edge: u16| v + 1 >= edge && v <= edge;
                    let on_edge = match pos {
                        Pos::Right => near(m.column, r.x) && m.row >= r.y && m.row < r.y + r.h,
                        Pos::Left => near(m.column, r.x + r.w) && m.row >= r.y && m.row < r.y + r.h,
                        Pos::Down => near(m.row, r.y) && m.column >= r.x && m.column < r.x + r.w,
                        Pos::Up => near(m.row, r.y + r.h) && m.column >= r.x && m.column < r.x + r.w,
                    };
                    match m.kind {
                        K::Down(MouseButton::Left) if on_edge => {
                            self.pv.drag = true;
                            return Ok(None);
                        }
                        K::Drag(MouseButton::Left) if self.pv.drag => {
                            let (total, from_far) = match pos {
                                Pos::Right => (a.w, (a.x + a.w).saturating_sub(m.column)),
                                Pos::Left => (a.w, m.column.saturating_sub(a.x)),
                                Pos::Down => (a.h, (a.y + a.h).saturating_sub(m.row)),
                                Pos::Up => (a.h, m.row.saturating_sub(a.y)),
                            };
                            let pct = (from_far as u32 * 100 / total.max(1) as u32).clamp(15, 85) as u16;
                            self.pv.spec.percent = true;
                            self.pv.spec.size = pct;
                            self.pv.spec.alt = None;
                            return Ok(None);
                        }
                        K::Up(MouseButton::Left) if self.pv.drag => {
                            self.pv.drag = false;
                            crate::look_state::save(self.opts.minimal, self.pv.visible, pos, Some(self.pv.spec.size));
                            return Ok(None);
                        }
                        _ => {}
                    }
                }
                // wheel over the preview scrolls the preview
                if let Some(r) = self.pv.rect
                    && m.column >= r.x
                    && m.column < r.x + r.w
                    && m.row >= r.y
                    && m.row < r.y + r.h
                {
                    if self.nvim_mode() && self.pv.nvim.is_some() {
                        match m.kind {
                            crossterm::event::MouseEventKind::ScrollUp => {
                                self.nvim_wheel(true, m.row, m.column);
                                return Ok(None);
                            }
                            crossterm::event::MouseEventKind::ScrollDown => {
                                self.nvim_wheel(false, m.row, m.column);
                                return Ok(None);
                            }
                            crossterm::event::MouseEventKind::Down(_) => {
                                self.pv.focus = true;
                                return Ok(None);
                            }
                            _ => {}
                        }
                    }
                    let key = match m.kind {
                        crossterm::event::MouseEventKind::ScrollUp => Some("preview-scroll-up"),
                        crossterm::event::MouseEventKind::ScrollDown => Some("preview-scroll-down"),
                        _ => None,
                    };
                    if let Some(k) = key {
                        if let Some(acts) = self.keymap.get(k).cloned() {
                            return self.run_actions(&acts, Some(term));
                        }
                        return Ok(None);
                    }
                }
                // the wheel over the button bar scrolls it
                if m.row == self.footer_y && !self.footer_fits {
                    use crossterm::event::MouseEventKind as K;
                    match m.kind {
                        K::ScrollDown | K::ScrollRight => {
                            self.footer_off += 1;
                            return Ok(None);
                        }
                        K::ScrollUp | K::ScrollLeft => {
                            self.footer_off = self.footer_off.saturating_sub(1);
                            return Ok(None);
                        }
                        _ => {}
                    }
                }
                if let crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) =
                    m.kind
                    && let Some(acts) = self.click_footer(m.column, m.row)
                {
                    return self.run_actions(&acts, Some(term));
                }
                if let crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) =
                    m.kind
                {
                    let row = m.row.saturating_sub(if term.fullscreen { 0 } else { 0 });
                    if let Some(&(_, hit)) = self.row_map.iter().find(|(y, _)| *y == row) {
                        let again = hit == self.cursor;
                        self.cursor = hit;
                        if again
                            && self
                                .last_click
                                .is_some_and(|t| t.elapsed() < Duration::from_millis(400))
                        {
                            if let Some(acts) = self.keymap.get("double-click").cloned() {
                                return self.run_actions(&acts, Some(term));
                            }
                        }
                        self.last_click = Some(Instant::now());
                    }
                    return Ok(None);
                }
                if let Some(name) = keys::mouse_name(&m)
                    && let Some(acts) = self.keymap.get(name).cloned()
                {
                    return self.run_actions(&acts, Some(term));
                }
                Ok(None)
            }
            TermEvent::Resize(w, h) => {
                lvim_fuzzy_ui::iconimg::resized();
                if term.fullscreen {
                    term.height = h;
                }
                term.width = w;
                term.prev = None;
                if let Some(acts) = self.keymap.get("resize").cloned() {
                    return self.run_actions(&acts, Some(term));
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}
