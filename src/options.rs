//! fzf-compatible command line parsing.
//!
//! Every fzf 0.75 option is known with its arity, so scripts written for fzf
//! parse the same way. Options that are not implemented yet are accepted and
//! recorded in `Options::unsupported`.

use crate::fields::{self, Delimiter, Range};
use crate::matcher::{CaseMode, Scheme, Tiebreak};

/// The parts of the screen --hide / --show / [show] name.
pub const PARTS: [&str; 7] = [
    "title",
    "counter",
    "search-label",
    "footer",
    "icons",
    "preview-title",
    "separator",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderStyle {
    None,
    Rounded,
    Sharp,
    Bold,
    Block,
    ThinBlock,
    Double,
    Dashed,
    Horizontal,
    Vertical,
    Top,
    Bottom,
    Left,
    Right,
    Line,
}

impl BorderStyle {
    pub fn parse(s: &str) -> Option<BorderStyle> {
        Some(match s {
            "none" => BorderStyle::None,
            "rounded" => BorderStyle::Rounded,
            "sharp" => BorderStyle::Sharp,
            "bold" => BorderStyle::Bold,
            "block" => BorderStyle::Block,
            "thinblock" => BorderStyle::ThinBlock,
            "double" => BorderStyle::Double,
            "dashed" => BorderStyle::Dashed,
            "horizontal" => BorderStyle::Horizontal,
            "vertical" => BorderStyle::Vertical,
            "top" => BorderStyle::Top,
            "bottom" => BorderStyle::Bottom,
            "left" => BorderStyle::Left,
            "right" => BorderStyle::Right,
            "line" => BorderStyle::Line,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Height {
    pub value: u16,
    pub percent: bool,
    pub negative: bool,
    pub adaptive: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Nushell,
}

#[derive(Clone, Debug)]
pub struct Walker {
    pub file: bool,
    pub dir: bool,
    pub follow: bool,
    pub hidden: bool,
    /// Walk a directory again under every path that reaches it (as fzf does);
    /// by default each directory is walked once.
    pub dup: bool,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub extended: bool,
    pub exact: bool,
    pub case: CaseMode,
    pub scheme: Scheme,
    pub nth: Option<Vec<Range>>,
    pub with_nth: Option<Vec<Range>>,
    pub accept_nth: Option<Vec<Range>>,
    pub delimiter: Delimiter,
    pub sort: bool,
    pub literal: bool,
    pub tail: Option<usize>,
    pub disabled: bool,
    pub tiebreak: Option<Vec<Tiebreak>>,
    pub read0: bool,
    pub print0: bool,
    pub ansi: bool,
    pub sync: bool,
    pub colors: Vec<String>,
    pub no_color: bool,
    pub bold: bool,
    pub height: Option<Height>,
    pub min_height: u16,
    pub multi: Option<usize>,
    /// --multi / --no-multi was given (else the look decides).
    pub multi_set: bool,
    pub cycle: bool,
    pub tac: bool,
    pub scroll_off: usize,
    pub hscroll: bool,
    pub hscroll_off: usize,
    pub marker: Option<String>,
    pub tabstop: usize,
    pub scrollbar: Option<String>,
    pub no_input: bool,
    pub prompt: String,
    pub ghost: String,
    /// The lvim look's texts: no matches / still loading.
    pub empty_text: String,
    /// The pattern a `pattern` mode (grep ➤ filter) ran its command with:
    /// marked in the rows and the preview.
    pub grep_pattern: String,
    pub loading_text: String,
    /// File icons in the lvim look (--no-icons), and the user's own
    /// (--icon=NAME=ICON[,COLOR]: NAME an extension, a file name or `dir`).
    pub icons: bool,
    /// --icon-mode: "auto" (Material images where the terminal takes them), "image", "font".
    pub icon_mode: String,
    /// --zellij-images: this zellij accepts kitty placeholders (a build with them).
    pub zellij_images: bool,
    pub icon_map: Vec<(String, String, Option<String>)>,
    pub filepath_word: bool,
    pub preview: Option<String>,
    pub preview_window: Vec<String>,
    pub preview_nvim: Option<String>,
    pub preview_order: Option<Vec<String>>,
    /// Filter scope for grep results: line / path / file / all.
    pub scope: Option<String>,
    pub track: bool,
    /// --wrap[=char|word]: long items continue on the next lines.
    pub wrap: bool,
    /// --raw: show every item, the non-matching ones dimmed.
    pub raw: bool,
    pub multi_line: bool,
    pub id_nth: Option<Vec<Range>>,
    pub freeze_left: usize,
    pub bind_help: bool,
    /// --config-template: print every config setting, commented out.
    pub config_template: bool,
    /// The popup border: 8 strings, Neovim's order (--popup-border).
    pub popup_border: [String; 8],
    /// --listen[=ADDR] / --listen-unsafe.
    pub listen: Option<String>,
    pub listen_unsafe: bool,
    /// --margin: [top, right, bottom, left] as (value, is_percent).
    pub margin: [(u16, bool); 4],
    pub wrap_sign: String,
    pub preview_wrap_sign: String,
    pub jump_labels: String,
    /// `--popup[=OPTS]` / `--tmux[=OPTS]`: run in a floating pane of Zellij / tmux.
    pub popup: Option<String>,
    /// The source command (a mode's), used instead of stdin / the walker.
    pub command: Option<String>,
    /// `file[:line[:col]]` template for the Neovim and built-in file preview.
    pub target: Option<String>,
    /// The mode this finder runs (set by the mode machinery).
    pub mode_name: Option<String>,
    /// Config file to read (`--config`); `--no-config` skips it.
    pub config: Option<String>,
    /// Parts of the screen not shown (--hide / [show]): title, counter,
    /// search-label, footer, icons, preview-title, separator.
    pub hidden: std::collections::BTreeSet<String>,
    pub theme: Option<String>,
    pub title: Option<String>,
    pub title_pos: String,
    /// Where the button bar sits: "bottom" (default) or "top".
    pub bar: String,
    pub nvim: String,
    pub nvim_appname: Option<String>,
    /// Take the preview's Neovim from a running `lvim-fuzzy pool` (--no-pool: always start one).
    pub pool: bool,
    /// The editor of the `open` action (--editor / `editor` in the config).
    pub editor: Option<String>,
    /// What the `open` action does (--open; a mode's `open`).
    pub open: Option<String>,
    /// --typos=N|auto (0 = none, as fzf).
    pub typos: u8,
    /// --exact-window=N: the widest match window scored exactly (wider ones
    /// are matched greedily); 0 = always exactly.
    pub exact_window: usize,
    pub header: Vec<String>,
    pub header_lines: usize,
    pub query: String,
    pub select_1: bool,
    pub exit_0: bool,
    pub filter: Option<String>,
    pub print_query: bool,
    pub expect: Vec<String>,
    pub binds: Vec<String>,
    pub with_shell: Option<String>,
    pub walker: Walker,
    pub walker_root: Vec<String>,
    pub walker_skip: Vec<String>,
    pub history: Option<String>,
    pub history_size: usize,
    pub shell: Option<Shell>,
    pub version: bool,
    pub help: bool,
    pub man: bool,
    pub unicode: bool,
    pub mouse: bool,
    pub unsupported: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            extended: true,
            exact: false,
            case: CaseMode::Smart,
            scheme: Scheme::Default,
            nth: None,
            with_nth: None,
            accept_nth: None,
            delimiter: Delimiter::Awk,
            sort: true,
            literal: false,
            tail: None,
            disabled: false,
            tiebreak: None,
            read0: false,
            print0: false,
            ansi: false,
            sync: false,
            colors: Vec::new(),
            no_color: false,
            bold: true,
            height: None,
            min_height: 10,
            multi: None,
            multi_set: false,
            cycle: false,
            tac: false,
            scroll_off: 3,
            hscroll: true,
            hscroll_off: 10,
            marker: None,
            tabstop: 8,
            scrollbar: None,
            no_input: false,
            prompt: "> ".into(),
            ghost: String::new(),
            empty_text: "[no matches]".into(),
            grep_pattern: String::new(),
            loading_text: "loading…".into(),
            icons: true,
            icon_mode: "auto".into(),
            zellij_images: true,
            icon_map: Vec::new(),
            filepath_word: false,
            preview: None,
            preview_window: Vec::new(),
            preview_nvim: None,
            preview_order: None,
            scope: None,
            track: false,
            wrap: false,
            raw: false,
            multi_line: true,
            id_nth: None,
            freeze_left: 0,
            bind_help: false,
            config_template: false,
            popup_border: std::array::from_fn(|_| " ".to_string()),
            listen: None,
            listen_unsafe: false,
            margin: [(0, false); 4],
            wrap_sign: "↳ ".into(),
            preview_wrap_sign: "↳ ".into(),
            jump_labels: "asdfghjklqwertyuiopzxcvbnm1234567890ASDFGHJKLQWERTYUIOPZXCVBNM`~;:,<.>/?'\"!@#$%^&*()[{]}-_=+".into(),
            popup: None,
            command: None,
            target: None,
            mode_name: None,
            config: None,
            hidden: Default::default(),
            theme: None,
            title: None,
            title_pos: "left".into(),
            bar: "bottom".into(),
            nvim: "nvim".into(),
            nvim_appname: None,
            pool: true,
            editor: None,
            open: None,
            typos: 0,
            exact_window: lvim_fuzzy_core::EXACT_WINDOW,
            header: Vec::new(),
            header_lines: 0,
            query: String::new(),
            select_1: false,
            exit_0: false,
            filter: None,
            print_query: false,
            expect: Vec::new(),
            binds: Vec::new(),
            with_shell: None,
            walker: Walker {
                file: true,
                dir: false,
                follow: true,
                hidden: true,
                dup: false,
            },
            walker_root: vec![".".into()],
            walker_skip: vec![".git".into(), "node_modules".into()],
            history: None,
            history_size: 1000,
            shell: None,
            version: false,
            help: false,
            man: false,
            unicode: true,
            mouse: true,
            unsupported: Vec::new(),
        }
    }
}

#[derive(Clone, Copy)]
enum Arity {
    Flag,
    Required,
    /// Value only via `=`, or the next argument when it passes the check.
    Optional(fn(&str) -> bool),
    /// One or more following arguments that do not start with `-`.
    Many,
}

struct Spec {
    long: &'static str,
    short: Option<char>,
    arity: Arity,
}

fn is_num(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}
fn is_scrollbar(s: &str) -> bool {
    !s.starts_with('-') && s.chars().count() <= 2 && !s.is_empty()
}
fn is_wrap(s: &str) -> bool {
    s == "char" || s == "word"
}
fn is_popup(s: &str) -> bool {
    let first = s.split(',').next().unwrap_or("");
    matches!(first, "center" | "top" | "bottom" | "left" | "right")
        || first.ends_with('%')
        || is_num(first)
}
fn is_listen(s: &str) -> bool {
    is_num(s) || s.contains(':') || s.ends_with(".sock")
}
fn never(_: &str) -> bool {
    false
}

macro_rules! spec {
    ($l:expr) => {
        Spec {
            long: $l,
            short: None,
            arity: Arity::Flag,
        }
    };
    ($l:expr, $s:expr) => {
        Spec {
            long: $l,
            short: Some($s),
            arity: Arity::Flag,
        }
    };
}
macro_rules! req {
    ($l:expr) => {
        Spec {
            long: $l,
            short: None,
            arity: Arity::Required,
        }
    };
    ($l:expr, $s:expr) => {
        Spec {
            long: $l,
            short: Some($s),
            arity: Arity::Required,
        }
    };
}
macro_rules! opt {
    ($l:expr, $f:expr) => {
        Spec {
            long: $l,
            short: None,
            arity: Arity::Optional($f),
        }
    };
}

/// `--popup-border`: a style name (rounded, sharp, bold, double, none, ...) or
/// eight comma-separated strings in Neovim's order (top-left, top, top-right,
/// right, bottom-right, bottom, bottom-left, left); `""` leaves a side out.
pub fn parse_popup_border(v: &str) -> Result<[String; 8], String> {
    let named: Option<[&str; 8]> = match v {
        "none" => Some([""; 8]),
        "space" | "padding" => Some([" "; 8]),
        "rounded" => Some(["╭", "─", "╮", "│", "╯", "─", "╰", "│"]),
        "sharp" | "single" => Some(["┌", "─", "┐", "│", "┘", "─", "└", "│"]),
        "bold" => Some(["┏", "━", "┓", "┃", "┛", "━", "┗", "┃"]),
        "double" => Some(["╔", "═", "╗", "║", "╝", "═", "╚", "║"]),
        "block" => Some(["▛", "▀", "▜", "▐", "▟", "▄", "▙", "▌"]),
        "thinblock" => Some(["🭽", "▔", "🭾", "▕", "🭿", "▁", "🭼", "▏"]),
        _ => None,
    };
    if let Some(n) = named {
        return Ok(n.map(String::from));
    }
    let parts: Vec<String> = v
        .split(',')
        .map(|p| p.trim_matches('"').trim_matches('\'').to_string())
        .collect();
    if parts.len() != 8 {
        return Err(format!(
            "invalid --popup-border: {v} (a style: rounded, sharp, bold, double, block, thinblock, space, none; or 8 comma-separated strings)"
        ));
    }
    if let Some(p) = parts.iter().find(|p| crate::canvas::str_width(p) > 1) {
        return Err(format!(
            "invalid --popup-border: `{p}` is wider than one cell"
        ));
    }
    Ok(std::array::from_fn(|i| parts[i].clone()))
}

/// Every long option for shell completion: `--name` for flags, `--name=` when
/// it takes a value.
pub fn completion_options() -> Vec<String> {
    let mut v: Vec<String> = SPECS
        .iter()
        .map(|s| match s.arity {
            Arity::Flag => format!("--{}", s.long),
            _ => format!("--{}=", s.long),
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

/// The fixed values of an option, for completion (empty = free text).
pub fn completion_values(opt: &str) -> &'static [&'static str] {
    match opt.trim_start_matches('-').trim_end_matches('=') {
        "scheme" => &["default", "path", "history"],
        "scope" => &["all", "line", "path", "file"],
        "bar" => &["top", "bottom"],
        "title-pos" => &["left", "center", "right"],
        "tiebreak" => &["length", "chunk", "pathname", "begin", "end", "index"],
        "preview-order" => &["nvim", "file", "command"],
        "wrap" => &["char", "word"],
        "popup" | "tmux" => &["center", "top", "bottom", "left", "right"],
        "typos" => &["0", "1", "2", "auto"],
        "exact-window" => &["4096", "0"],
        "popup-border" => &[
            "space",
            "none",
            "rounded",
            "sharp",
            "bold",
            "double",
            "block",
            "thinblock",
        ],
        _ => &[],
    }
}

#[rustfmt::skip]
const SPECS: &[Spec] = &[
    // search
    spec!("exact", 'e'), spec!("extended", 'x'), spec!("ignore-case", 'i'), spec!("smart-case"),
    req!("scheme"), req!("nth", 'n'), req!("with-nth"), req!("accept-nth"), req!("delimiter", 'd'),
    spec!("sort", 's'), spec!("literal"), req!("tail"), spec!("disabled"), spec!("phony"), spec!("enabled"),
    req!("tiebreak"), req!("algo"), spec!("track"), req!("id-nth"),
    // input/output
    spec!("read0"), spec!("print0"), spec!("ansi"), spec!("sync"),
    // style
    req!("color"), spec!("no-color"), spec!("bold"), spec!("black"),
    spec!("unicode"), spec!("ambidouble"), spec!("mouse"), spec!("clear"), spec!("no-winpty"),
    // display
    req!("height"), req!("min-height"), opt!("popup", is_popup), opt!("tmux", is_popup),
    // layout
    req!("margin"),
    // list
    Spec { long: "multi", short: Some('m'), arity: Arity::Optional(is_num) },
    spec!("cycle"), opt!("wrap", is_wrap), req!("wrap-sign"), spec!("multi-line"),
    spec!("raw"), spec!("tac"), req!("freeze-left"),
    req!("scroll-off"), spec!("hscroll"), req!("hscroll-off"),
    req!("jump-labels"), req!("marker"), req!("tabstop"), opt!("scrollbar", is_scrollbar),
    // input section
    spec!("input"), req!("prompt"), req!("ghost"), req!("popup-border"), req!("editor"), req!("open"),
    req!("typos"), req!("exact-window"), spec!("filepath-word"), req!("empty-text"), req!("loading-text"),
    req!("hide"), req!("show"), req!("grep-pattern"), spec!("icons"), req!("icon"), req!("icon-mode"), spec!("zellij-images"),
    // preview
    req!("preview"), req!("preview-window"), req!("preview-wrap-sign"),
    // lvim-fuzzy: a real Neovim as the preview
    opt!("preview-nvim", never), req!("nvim"), req!("nvim-appname"), spec!("pool"),
    // lvim-fuzzy: the lvim look
    req!("preview-order"), req!("scope"), req!("command"), req!("target"), req!("mode-name"), req!("config"), req!("theme"), req!("title"), req!("title-pos"), req!("bar"),
    // header / footer
    req!("header"), req!("header-lines"),
    // scripting
    req!("query", 'q'), spec!("select-1", '1'), spec!("exit-0", '0'), req!("filter", 'f'),
    spec!("print-query"), req!("expect"), req!("bind"), req!("toggle-sort"),
    // advanced
    req!("with-shell"), opt!("listen", is_listen), opt!("listen-unsafe", is_listen),
    // walker
    req!("walker"), Spec { long: "walker-root", short: None, arity: Arity::Many }, req!("walker-skip"),
    // history
    req!("history"), req!("history-size"),
    // shell integration / help
    spec!("bash"), spec!("zsh"), spec!("fish"), spec!("nushell"), spec!("version"), spec!("help", 'h'),
    spec!("man"), spec!("bind-help"), spec!("config-template"),
    // legacy / hidden
    spec!("no-256"), spec!("256"), spec!("2"),
];

fn find_long(name: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.long == name)
}
fn find_short(c: char) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.short == Some(c))
}

pub use lvim_fuzzy_ui::util::shell_words;

/// One parsed argument: option name, its value, and whether it was negated.
struct Arg {
    name: &'static str,
    value: Option<String>,
    negated: bool,
}

fn tokenize(args: &[String]) -> Result<Vec<Arg>, String> {
    let mut out = Vec::new();
    let mut i = 0;
    let take_value = |i: &mut usize, name: &str| -> Result<String, String> {
        *i += 1;
        args.get(*i)
            .cloned()
            .ok_or_else(|| format!("argument required: --{name}"))
    };
    while i < args.len() {
        let a = &args[i];
        if let Some(body) = a.strip_prefix("--") {
            let (name, inline) = match body.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (body, None),
            };
            let (spec, negated) = match find_long(name) {
                Some(s) => (s, false),
                None => match name.strip_prefix("no-").and_then(find_long) {
                    Some(s) => (s, true),
                    None => return Err(format!("unknown option: {a}")),
                },
            };
            let value = if negated {
                None
            } else {
                match spec.arity {
                    Arity::Flag => {
                        if inline.is_some() {
                            return Err(format!("option does not take a value: --{name}"));
                        }
                        None
                    }
                    Arity::Required => match inline {
                        Some(v) => Some(v),
                        None => Some(take_value(&mut i, name)?),
                    },
                    Arity::Optional(check) => match inline {
                        Some(v) => Some(v),
                        None => match args.get(i + 1) {
                            Some(n) if check(n) => {
                                i += 1;
                                Some(n.clone())
                            }
                            _ => None,
                        },
                    },
                    Arity::Many => {
                        let mut vals = Vec::new();
                        if let Some(v) = inline {
                            vals.push(v);
                        }
                        while let Some(n) = args.get(i + 1) {
                            if n.starts_with('-') && n != "-" {
                                break;
                            }
                            vals.push(n.clone());
                            i += 1;
                        }
                        if vals.is_empty() {
                            return Err(format!("argument required: --{name}"));
                        }
                        Some(vals.join("\u{0}"))
                    }
                }
            };
            out.push(Arg {
                name: spec.long,
                value,
                negated,
            });
        } else if let Some(body) = a.strip_prefix('+').filter(|b| !b.is_empty()) {
            for c in body.chars() {
                let spec = find_short(c).ok_or_else(|| format!("unknown option: +{c}"))?;
                out.push(Arg {
                    name: spec.long,
                    value: None,
                    negated: true,
                });
            }
        } else if let Some(body) = a.strip_prefix('-').filter(|b| !b.is_empty()) {
            let mut chars = body.char_indices();
            while let Some((p, c)) = chars.next() {
                let spec = find_short(c).ok_or_else(|| format!("unknown option: -{c}"))?;
                match spec.arity {
                    Arity::Flag => out.push(Arg {
                        name: spec.long,
                        value: None,
                        negated: false,
                    }),
                    Arity::Optional(check) => {
                        let rest = &body[p + c.len_utf8()..];
                        if !rest.is_empty() && check(rest) {
                            out.push(Arg {
                                name: spec.long,
                                value: Some(rest.into()),
                                negated: false,
                            });
                            break;
                        }
                        if rest.is_empty()
                            && let Some(n) = args.get(i + 1).filter(|n| check(n))
                        {
                            i += 1;
                            out.push(Arg {
                                name: spec.long,
                                value: Some(n.clone()),
                                negated: false,
                            });
                            break;
                        }
                        out.push(Arg {
                            name: spec.long,
                            value: None,
                            negated: false,
                        });
                    }
                    _ => {
                        let rest = &body[p + c.len_utf8()..];
                        let v = if rest.is_empty() {
                            take_value(&mut i, spec.long)?
                        } else {
                            rest.to_string()
                        };
                        out.push(Arg {
                            name: spec.long,
                            value: Some(v),
                            negated: false,
                        });
                        break;
                    }
                }
            }
        } else {
            return Err(format!("unknown option: {a}"));
        }
        i += 1;
    }
    Ok(out)
}

fn parse_height(s: &str) -> Result<Height, String> {
    let mut t = s;
    let adaptive = t.starts_with('~');
    if adaptive {
        t = &t[1..];
    }
    let negative = t.starts_with('-');
    if negative {
        t = &t[1..];
    }
    let percent = t.ends_with('%');
    if percent {
        t = &t[..t.len() - 1];
    }
    let value: u16 = t.parse().map_err(|_| format!("invalid height: {s}"))?;
    if percent && value > 100 {
        return Err(format!("invalid height: {s}"));
    }
    Ok(Height {
        value,
        percent,
        negative,
        adaptive,
    })
}

/// `TRBL | TB,RL | T,RL,B | T,R,B,L`, each N or N%.
fn parse_trbl(s: &str) -> Result<[(u16, bool); 4], String> {
    let one = |p: &str| -> Result<(u16, bool), String> {
        let p = p.trim();
        let (n, pct) = match p.strip_suffix('%') {
            Some(n) => (n, true),
            None => (p, false),
        };
        n.parse::<u16>()
            .map(|v| (v, pct))
            .map_err(|_| format!("invalid margin/padding: {s}"))
    };
    let v: Vec<(u16, bool)> = s.split(',').map(one).collect::<Result<_, _>>()?;
    Ok(match v.as_slice() {
        [a] => [*a, *a, *a, *a],
        [tb, rl] => [*tb, *rl, *tb, *rl],
        [t, rl, b] => [*t, *rl, *b, *rl],
        [t, r, b, l] => [*t, *r, *b, *l],
        _ => return Err(format!("invalid margin/padding: {s}")),
    })
}

fn parse_tiebreak(s: &str) -> Result<Vec<Tiebreak>, String> {
    let mut out = Vec::new();
    for p in s.split(',') {
        let t = match p {
            "length" => Tiebreak::Length,
            "chunk" => Tiebreak::Chunk,
            "pathname" => Tiebreak::Pathname,
            "begin" => Tiebreak::Begin,
            "end" => Tiebreak::End,
            "index" => Tiebreak::Index,
            _ => return Err(format!("invalid sort criterion: {p}")),
        };
        if out.contains(&t) {
            return Err(format!("duplicate sort criteria: {p}"));
        }
        if t == Tiebreak::Index {
            out.push(t);
            break;
        }
        out.push(t);
    }
    Ok(out)
}

fn num<T: std::str::FromStr>(name: &str, v: &str) -> Result<T, String> {
    v.parse()
        .map_err(|_| format!("invalid value for --{name}: {v}"))
}

impl Options {
    /// Parse `LVIM_FUZZY_DEFAULT_OPTS_FILE`, `LVIM_FUZZY_DEFAULT_OPTS` and argv, in that order.
    /// The options from `LVIM_FUZZY_DEFAULT_OPTS_FILE` and `LVIM_FUZZY_DEFAULT_OPTS`.
    pub fn env_args() -> Result<Vec<String>, String> {
        let mut all: Vec<String> = Vec::new();
        if let Ok(path) = std::env::var("LVIM_FUZZY_DEFAULT_OPTS_FILE")
            && !path.is_empty()
        {
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
            all.extend(shell_words(&text)?);
        }
        if let Ok(v) = std::env::var("LVIM_FUZZY_DEFAULT_OPTS") {
            all.extend(shell_words(&v)?);
        }
        Ok(all)
    }

    #[allow(dead_code)]
    pub fn from_env_and_args(argv: &[String]) -> Result<Options, String> {
        let mut all: Vec<String> = Vec::new();
        if let Ok(path) = std::env::var("LVIM_FUZZY_DEFAULT_OPTS_FILE")
            && !path.is_empty()
        {
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
            all.extend(shell_words(&text)?);
        }
        for var in ["LVIM_FUZZY_DEFAULT_OPTS"] {
            if let Ok(v) = std::env::var(var) {
                all.extend(shell_words(&v)?);
            }
        }
        all.extend(argv.iter().cloned());
        Options::parse(&all)
    }

    pub fn parse(args: &[String]) -> Result<Options, String> {
        let mut o = Options::default();
        for a in tokenize(args)? {
            o.apply(a)?;
        }
        // Tab selects out of the box; --multi / --no-multi win
        if !o.multi_set {
            o.multi = Some(0);
        }
        Ok(o)
    }

    fn apply(&mut self, a: Arg) -> Result<(), String> {
        let d = Options::default();
        let n = a.name;
        let v = a.value.as_deref();
        let neg = a.negated;
        let val = || v.unwrap_or_default();
        match n {
            "exact" => self.exact = !neg,
            "extended" => self.extended = !neg,
            "ignore-case" => {
                self.case = if neg {
                    CaseMode::Respect
                } else {
                    CaseMode::Ignore
                }
            }
            "smart-case" => self.case = CaseMode::Smart,
            "scheme" => {
                self.scheme = match val() {
                    "default" => Scheme::Default,
                    "path" => Scheme::Path,
                    "history" => Scheme::History,
                    s => return Err(format!("invalid scoring scheme: {s}")),
                }
            }
            "nth" => {
                self.nth = if neg {
                    None
                } else {
                    Some(fields::parse_nth(val())?)
                }
            }
            "with-nth" => {
                self.with_nth = if neg {
                    None
                } else {
                    Some(fields::parse_nth(val())?)
                }
            }
            "accept-nth" => {
                self.accept_nth = if neg {
                    None
                } else {
                    Some(fields::parse_nth(val())?)
                }
            }
            "delimiter" => {
                self.delimiter = if neg {
                    Delimiter::Awk
                } else {
                    Delimiter::parse(val())
                }
            }
            "sort" => self.sort = !neg,
            "toggle-sort" => self.binds.push(format!("{}:toggle-sort", val())),
            "literal" => self.literal = !neg,
            "tail" => self.tail = if neg { None } else { Some(num(n, val())?) },
            "disabled" | "phony" => self.disabled = !neg,
            "enabled" => self.disabled = neg,
            "tiebreak" => self.tiebreak = Some(parse_tiebreak(val())?),
            "read0" => self.read0 = !neg,
            "print0" => self.print0 = !neg,
            "ansi" => self.ansi = !neg,
            "sync" => self.sync = !neg,
            "color" => {
                if neg {
                    self.no_color = true;
                } else {
                    self.colors.push(val().to_string());
                }
            }
            "no-color" => self.no_color = true,
            "bold" => self.bold = !neg,
            "unicode" => self.unicode = !neg,
            "mouse" => self.mouse = !neg,
            "height" => {
                self.height = if neg {
                    None
                } else {
                    Some(parse_height(val())?)
                }
            }
            "min-height" => {
                if !neg {
                    self.min_height = num(n, val().trim_end_matches('+'))?;
                }
            }
            "multi" => {
                self.multi_set = true;
                self.multi = if neg {
                    None
                } else {
                    match v {
                        None => Some(0),
                        Some(s) => Some(num(n, s)?),
                    }
                }
            }
            "cycle" => self.cycle = !neg,
            "tac" => self.tac = !neg,
            "scroll-off" => self.scroll_off = if neg { 0 } else { num(n, val())? },
            "hscroll" => self.hscroll = !neg,
            "hscroll-off" => self.hscroll_off = if neg { d.hscroll_off } else { num(n, val())? },
            "marker" => {
                self.marker = if neg {
                    Some(String::new())
                } else {
                    Some(val().to_string())
                }
            }
            "tabstop" => {
                let t: usize = num(n, val())?;
                if t < 1 {
                    return Err("tab stop must be a positive integer".into());
                }
                self.tabstop = t;
            }
            "scrollbar" => {
                self.scrollbar = if neg {
                    Some(String::new())
                } else {
                    v.map(str::to_string)
                }
            }
            "input" => self.no_input = neg,
            "prompt" => {
                self.prompt = if neg {
                    String::new()
                } else {
                    val().to_string()
                }
            }
            "grep-pattern" => {
                self.grep_pattern = if neg {
                    String::new()
                } else {
                    val().to_string()
                }
            }
            "empty-text" => {
                self.empty_text = if neg {
                    String::new()
                } else {
                    val().to_string()
                }
            }
            "loading-text" => {
                self.loading_text = if neg {
                    String::new()
                } else {
                    val().to_string()
                }
            }
            "icons" => self.icons = !neg,
            "icon-mode" => {
                let v = val();
                if lvim_fuzzy_ui::iconimg::Mode::parse(v).is_none() {
                    return Err(format!("invalid --icon-mode: {v} (auto, image, font)"));
                }
                self.icon_mode = v.to_string();
            }
            "zellij-images" => self.zellij_images = !neg,
            "icon" => {
                let v = val();
                let (key, rest) = v
                    .split_once('=')
                    .filter(|(k, i)| !k.is_empty() && !i.is_empty())
                    .ok_or_else(|| format!("invalid --icon: {v} (NAME=ICON[,COLOR])"))?;
                let (glyph, color) = match rest.split_once(',') {
                    Some((g, c)) => (g.to_string(), Some(c.to_string())),
                    None => (rest.to_string(), None),
                };
                let key = key.to_ascii_lowercase();
                self.icon_map.retain(|(k, _, _)| *k != key);
                self.icon_map.push((key, glyph, color));
            }
            "ghost" => {
                self.ghost = if neg {
                    String::new()
                } else {
                    val().to_string()
                }
            }
            "filepath-word" => self.filepath_word = !neg,
            "preview" => self.preview = if neg { None } else { Some(val().to_string()) },
            "preview-window" => self.preview_window.push(val().to_string()),
            "preview-nvim" => {
                self.preview_nvim = if neg {
                    None
                } else {
                    Some(v.unwrap_or("{}").to_string())
                }
            }
            "nvim" => self.nvim = val().to_string(),
            "nvim-appname" => self.nvim_appname = if neg { None } else { Some(val().to_string()) },
            "pool" => self.pool = !neg,
            "editor" => self.editor = if neg { None } else { Some(val().to_string()) },
            "exact-window" => {
                let v = val();
                self.exact_window = v.parse().map_err(|_| {
                    format!(
                        "invalid --exact-window: {v} (a number of characters; 0 = always exact)"
                    )
                })?;
            }
            "typos" => {
                self.typos = match val() {
                    "auto" => crate::matcher::TYPOS_AUTO,
                    v => v
                        .parse::<u8>()
                        .ok()
                        .filter(|n| *n <= 2)
                        .ok_or_else(|| format!("invalid --typos: {v} (0, 1, 2 or auto)"))?,
                };
            }
            "open" => {
                // an action, or a plain command (= become(command))
                let e = crate::modes::open_action(val());
                crate::keys::parse_binds(&format!("x:{e}"))
                    .map_err(|err| format!("--open: {err}"))?;
                self.open = Some(e);
            }
            "preview-order" => {
                let list: Vec<String> = val()
                    .split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect();
                for x in &list {
                    // built-in names or user previewers (checked when the finder starts)
                    if !x.chars().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'
                    }) {
                        return Err(format!("invalid preview provider name: {x}"));
                    }
                }
                self.preview_order = if neg { None } else { Some(list) };
            }
            "hide" | "show" => {
                for part in val().split(',').map(str::trim).filter(|p| !p.is_empty()) {
                    if !PARTS.contains(&part) {
                        return Err(format!("invalid --{n} part: {part} ({})", PARTS.join(", ")));
                    }
                    if (n == "hide") != neg {
                        self.hidden.insert(part.to_string());
                    } else {
                        self.hidden.remove(part);
                    }
                }
            }
            "scope" => {
                if crate::engine::Scope::parse(val()).is_none() {
                    return Err(format!("invalid scope: {} (line, path, file, all)", val()));
                }
                self.scope = if neg { None } else { Some(val().to_string()) };
            }
            "command" => self.command = if neg { None } else { Some(val().to_string()) },
            "target" => self.target = if neg { None } else { Some(val().to_string()) },
            "mode-name" => self.mode_name = if neg { None } else { Some(val().to_string()) },
            "config" => {
                self.config = if neg {
                    Some(String::new())
                } else {
                    Some(val().to_string())
                }
            }
            "track" => self.track = !neg,
            "margin" => {
                self.margin = if neg {
                    [(0, false); 4]
                } else {
                    parse_trbl(val())?
                }
            }
            "wrap" => self.wrap = !neg,
            "wrap-sign" => self.wrap_sign = val().to_string(),
            "raw" => self.raw = !neg,
            "multi-line" => self.multi_line = !neg,
            "id-nth" => {
                self.id_nth = if neg {
                    None
                } else {
                    Some(fields::parse_nth(val())?)
                }
            }
            "freeze-left" => self.freeze_left = num(n, val())?,
            "bind-help" => self.bind_help = true,
            "config-template" => self.config_template = true,
            "popup-border" => self.popup_border = parse_popup_border(val())?,
            "listen" | "listen-unsafe" => {
                self.listen = if neg {
                    None
                } else {
                    Some(v.unwrap_or("").to_string())
                };
                self.listen_unsafe = n == "listen-unsafe" && !neg;
            }
            "preview-wrap-sign" => self.preview_wrap_sign = val().to_string(),
            "jump-labels" => {
                if !neg {
                    if val().is_empty() {
                        return Err("jump labels must not be empty".into());
                    }
                    self.jump_labels = val().to_string();
                }
            }
            "theme" => self.theme = if neg { None } else { Some(val().to_string()) },
            "title" => self.title = if neg { None } else { Some(val().to_string()) },
            "title-pos" => {
                if !matches!(val(), "left" | "center" | "right") {
                    return Err(format!(
                        "invalid title position: {} (left, center, right)",
                        val()
                    ));
                }
                self.title_pos = val().to_string();
            }
            "bar" => {
                if !matches!(val(), "top" | "bottom") {
                    return Err(format!("invalid bar position: {} (top, bottom)", val()));
                }
                self.bar = val().to_string();
            }
            "popup" | "tmux" => {
                self.popup = if neg {
                    None
                } else {
                    Some(v.unwrap_or("").to_string())
                };
                if let Some(o) = &self.popup {
                    crate::popup::parse(o)?;
                }
            }
            "header" => {
                self.header = if neg {
                    Vec::new()
                } else {
                    val().lines().map(str::to_string).collect()
                }
            }
            "header-lines" => self.header_lines = if neg { 0 } else { num(n, val())? },
            "query" => self.query = val().to_string(),
            "select-1" => self.select_1 = !neg,
            "exit-0" => self.exit_0 = !neg,
            "filter" => self.filter = if neg { None } else { Some(val().to_string()) },
            "print-query" => self.print_query = !neg,
            "expect" => {
                if neg {
                    self.expect.clear();
                } else {
                    self.expect.extend(
                        val()
                            .split(',')
                            .filter(|s| !s.is_empty())
                            .map(str::to_string),
                    );
                }
            }
            "bind" => self.binds.push(val().to_string()),
            "with-shell" => self.with_shell = if neg { None } else { Some(val().to_string()) },
            "walker" => {
                let mut w = Walker {
                    file: false,
                    dir: false,
                    follow: false,
                    hidden: false,
                    dup: false,
                };
                for p in val().split(',') {
                    match p {
                        "file" => w.file = true,
                        "dir" => w.dir = true,
                        "follow" => w.follow = true,
                        "hidden" => w.hidden = true,
                        "dup" => w.dup = true,
                        _ => return Err(format!("invalid walker option: {p}")),
                    }
                }
                if !w.file && !w.dir {
                    return Err("walker requires file or dir".into());
                }
                self.walker = w;
            }
            "walker-root" => self.walker_root = val().split('\u{0}').map(str::to_string).collect(),
            "walker-skip" => self.walker_skip = val().split(',').map(str::to_string).collect(),
            "history" => self.history = if neg { None } else { Some(val().to_string()) },
            "history-size" => self.history_size = num(n, val())?,
            "bash" => self.shell = Some(Shell::Bash),
            "zsh" => self.shell = Some(Shell::Zsh),
            "fish" => self.shell = Some(Shell::Fish),
            "nushell" => self.shell = Some(Shell::Nushell),
            "version" => self.version = true,
            "help" => self.help = true,
            "man" => self.man = true,
            "black" | "clear" | "no-winpty" | "256" | "2" | "no-256" | "ambidouble" | "algo" => {}
            _ => {
                if !neg && !self.unsupported.iter().any(|u| u == n) {
                    self.unsupported.push(n.to_string());
                }
            }
        }
        Ok(())
    }

    pub fn tiebreaks(&self) -> Vec<Tiebreak> {
        if let Some(t) = &self.tiebreak {
            return t.clone();
        }
        match self.scheme {
            Scheme::Default => vec![Tiebreak::Length],
            Scheme::Path => vec![Tiebreak::Pathname, Tiebreak::Length],
            Scheme::History => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Options {
        Options::parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn forms() {
        let o = p(&["-q", "foo", "-m3", "--height=40%", "+s", "-e"]);
        assert_eq!(o.query, "foo");
        assert_eq!(o.multi, Some(3));
        assert_eq!(o.height.unwrap().value, 40);
        assert!(!o.sort && o.exact);
        let o = p(&["-m", "--no-sort", "-d:", "-n2", "-1", "-0"]);
        assert_eq!(o.multi, Some(0));
        assert!(!o.sort && o.select_1 && o.exit_0);
        let o = p(&["--query=x", "-m", "--multi", "--no-multi"]);
        assert!(Options::parse(&["--layout=reverse".to_string()]).is_err());
        assert_eq!(o.multi, None);
        assert!(Options::parse(&["--nope".to_string()]).is_err());
    }

    #[test]
    fn lvim_options() {
        let o = p(&[
            "--preview-nvim={1}:{2}",
            "--nvim",
            "nvim -u NONE",
            "--nvim-appname=x",
            "--preview-order=file,command",
            "--theme",
            "LvimNord_dark",
            "--title=Grep",
            "--title-pos=center",
            "--bar=top",
        ]);
        assert_eq!(o.preview_nvim.as_deref(), Some("{1}:{2}"));
        assert_eq!(o.nvim, "nvim -u NONE");
        assert_eq!(
            o.preview_order,
            Some(vec!["file".to_string(), "command".to_string()])
        );
        assert_eq!(
            (o.title.as_deref(), o.title_pos.as_str(), o.bar.as_str()),
            (Some("Grep"), "center", "top")
        );
        assert!(o.unsupported.is_empty(), "{:?}", o.unsupported);
        assert!(Options::parse(&["--bar=left".to_string()]).is_err());
    }

    #[test]
    fn words() {
        assert_eq!(
            shell_words(r#"--bind 'ctrl-a:select-all' --prompt="a b" x\ y"#).unwrap(),
            vec!["--bind", "ctrl-a:select-all", "--prompt=a b", "x y"]
        );
    }
}
