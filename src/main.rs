//! lvim-fuzzy — a fuzzy finder for the terminal, compatible with fzf.

mod ansi;
mod canvas;
mod config;
mod doctor;
mod engine;
mod fields;
mod filepreview;
mod help;
mod icons;
mod item;
mod keys;
mod listen;
mod lvim;
mod matcher;
mod modes;
mod nvim;
mod options;
mod placeholder;
mod pool;
mod popup;
mod preview;
mod previewers;
mod reader;
mod shell;
mod syntax;
mod termcolors;
mod theme;
mod tui;

use std::io::{IsTerminal, Write};
use std::sync::mpsc::channel;
use std::thread;

use engine::{MatchSettings, run_match};
use item::ItemBuilder;
use matcher::MatchConfig;
use options::Options;
use reader::{Event, Feed};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    tui::stage("main");
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let code = match real_main(&argv) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("lvim-fuzzy: {e}");
            2
        }
    };
    tui::stage("exit");
    std::process::exit(code);
}

pub fn shell_command(opts: &Options) -> Vec<String> {
    if let Some(ws) = &opts.with_shell {
        return options::shell_words(ws).unwrap_or_else(|_| vec!["sh".into(), "-c".into()]);
    }
    let sh = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "sh".into());
    vec![sh, "-c".into()]
}

pub fn match_settings(opts: &Options) -> MatchSettings {
    MatchSettings {
        cfg: MatchConfig {
            case: opts.case,
            extended: opts.extended,
            exact: opts.exact,
            normalize: !opts.literal,
            scheme: opts.scheme,
            typos: opts.typos,
            exact_window: opts.exact_window,
        },
        sort: opts.sort,
        tac: opts.tac,
        tiebreak: opts.tiebreaks(),
        scope: opts
            .scope
            .as_deref()
            .and_then(engine::Scope::parse)
            .unwrap_or(engine::Scope::All),
    }
}

/// Start the input source in a background thread.
pub fn start_source(opts: &Options, feed: Feed) {
    let sep = if opts.read0 { 0u8 } else { b'\n' };
    let stdin_tty = std::io::stdin().is_terminal();
    let shell = shell_command(opts);
    let default_cmd = std::env::var("LVIM_FUZZY_DEFAULT_COMMAND")
        .ok()
        .filter(|s| !s.is_empty());
    let walker = opts.walker.clone();
    let roots = opts.walker_root.clone();
    let skip = opts.walker_skip.clone();
    let mode_cmd = opts.command.clone();
    thread::spawn(move || {
        // a file on stdin, for the finder (which builds items): mapped, split and
        // built on every core, as --filter does, then handed over in big parts
        if mode_cmd.is_none()
            && !stdin_tty
            && let Some(b) = feed.build.clone()
            && let Ok(input) = Input::read_file_only(&std::io::stdin())
        {
            use rayon::prelude::*;
            tui::stage("reader: mapped");
            let data = input.bytes();
            let b = b.read().unwrap();
            let mut blocks = Vec::new();
            let mut from = 0;
            while from < data.len() {
                let want = (from + (1 << 18)).min(data.len());
                let end = if want == data.len() {
                    want
                } else {
                    memchr::memchr(sep, &data[want..]).map_or(data.len(), |p| want + p + 1)
                };
                blocks.push((from, end));
                from = end;
            }
            // every record's bounds (per block, on every core), so the count is
            // known and the finder can size its list once
            let bounds: Vec<(usize, usize)> = blocks
                .par_iter()
                .map(|&(a, z)| {
                    let mut v = Vec::with_capacity((z - a) / 32 + 1);
                    let mut s = a;
                    for p in memchr::memchr_iter(sep, &data[a..z]) {
                        v.push((s, a + p));
                        s = a + p + 1;
                    }
                    if s < z {
                        v.push((s, z));
                    }
                    v
                })
                .collect::<Vec<_>>()
                .concat();
            tui::stage("reader: bounds");
            // built in place on every core, numbered here: the finder takes the list
            // as it is (moved, not copied). Up to 2M records at once; beyond, in
            // parts of 2M so the first ones show while the rest is built.
            const PART: usize = 2_000_000;
            if bounds.len() > PART {
                let _ = feed.tx.send((feed.epoch, Event::Reserve(bounds.len())));
            }
            for (k, part) in bounds.chunks(PART).enumerate() {
                let built = build_part(&b, k * PART, part, data, sep);
                tui::stage("reader: part built");
                if feed.tx.send((feed.epoch, Event::Items(built))).is_err() {
                    break;
                }
            }
            std::mem::forget(input);
            let _ = feed.send(Event::Eof);
            return;
        }
        if let Some(cmd) = mode_cmd {
            reader::read_command(&shell, &cmd, sep, &feed, feed.child.clone());
        } else if !stdin_tty {
            reader::read_stream(std::io::stdin().lock(), sep, &feed);
        } else if let Some(cmd) = default_cmd {
            reader::read_command(&shell, &cmd, sep, &feed, feed.child.clone());
        } else {
            reader::walk(&roots, &walker, &skip, &feed);
        }
        let _ = feed.send(Event::Eof);
    });
}

/// Every mode: the built-in ones, then the config's.
pub static MODES: std::sync::OnceLock<Vec<modes::Mode>> = std::sync::OnceLock::new();
/// The user's own options (no subcommand, no query), re-applied when the
/// finder switches to another mode.
pub static PASSTHROUGH: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// User previewers from the config.
pub static PREVIEWERS: std::sync::OnceLock<Vec<previewers::Previewer>> = std::sync::OnceLock::new();

pub fn previewers() -> &'static [previewers::Previewer] {
    PREVIEWERS.get().map(Vec::as_slice).unwrap_or(&[])
}

pub fn all_modes() -> &'static [modes::Mode] {
    MODES.get().map(Vec::as_slice).unwrap_or(&[])
}

pub fn find_mode(name: &str) -> Option<&'static modes::Mode> {
    all_modes().iter().find(|m| m.name == name)
}

/// This program, for re-running itself (mode switches, the mode list).
pub fn self_exe() -> String {
    std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "lvim-fuzzy".into())
}

/// `--list-modes`: one `name<TAB>description` line per mode (for `lvim-fuzzy mode`).
fn list_modes() -> i32 {
    for m in all_modes() {
        let missing = m.missing();
        let note = if missing.is_empty() {
            String::new()
        } else {
            format!("  (needs {})", missing.join(", "))
        };
        let kind = if m.builtin { "" } else { "  [user]" };
        println!("{}\t{}{}{}", m.name, m.description, kind, note);
    }
    0
}

/// `--complete-list KIND [OPTION]`: candidates for the shell completion.
///   commands  the modes and subcommands, `name<TAB>description`
///   options   every long option (`--x=` when it takes a value)
///   values    the values of OPTION (themes and modes are read live)
fn complete_list(kind: &str, opt: Option<&str>) -> i32 {
    match kind {
        "commands" => {
            for m in all_modes() {
                println!("{}\t{}", m.name, m.description);
            }
            println!("mode\tthe list of every mode");
            println!("doctor\tevery problem: config, themes, modes, Neovim, terminal");
            println!("config\tcheck the config or print its path");
        }
        "options" => {
            for o in options::completion_options() {
                println!("{o}");
            }
        }
        "values" => match opt
            .unwrap_or("")
            .trim_start_matches('-')
            .trim_end_matches('=')
        {
            "theme" => {
                println!("terminal");
                let mut names = Vec::new();
                for d in lvim::theme_dirs() {
                    for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                        let p = e.path();
                        if p.extension().is_some_and(|x| x == "toml")
                            && let Some(s) = p.file_stem()
                        {
                            names.push(s.to_string_lossy().to_string());
                        }
                    }
                }
                names.sort();
                for n in names {
                    println!("{n}");
                }
            }
            "mode" => {
                for m in all_modes() {
                    println!("{}", m.name);
                }
            }
            o => {
                for v in options::completion_values(o) {
                    println!("{v}");
                }
            }
        },
        _ => return 2,
    }
    0
}

/// `--describe-mode NAME`: what a mode does (the preview of `lvim-fuzzy mode`).
fn describe_mode(name: &str) -> i32 {
    let Some(m) = find_mode(name) else {
        println!("no such mode: {name}");
        return 1;
    };
    println!("{}  —  {}", m.name, m.title);
    println!();
    println!("{}", m.description);
    println!();
    println!("start:    lvim-fuzzy {}", m.name);
    match &m.command {
        Some(c) => println!("source:   {c}"),
        None => println!("source:   the built-in directory walker"),
    }
    if m.live {
        println!("search:   the command re-runs with the query on every change");
    }
    if m.pattern {
        println!(
            "search:   the command runs once for the starting query; typing filters the results"
        );
    }
    if let Some(p) = &m.preview {
        println!("preview:  {p}");
    }
    if let Some(t) = &m.target {
        println!("target:   {t}");
    }
    if let Some(s) = &m.scope {
        println!("scope:    {s}");
    }
    let missing = m.missing();
    if !missing.is_empty() {
        println!();
        println!("needs (missing): {}", missing.join(", "));
    }
    if !m.builtin {
        println!();
        println!("(a user mode from the config)");
    }
    0
}

fn real_main(argv_in: &[String]) -> Result<i32, String> {
    let _ = ORIGINAL_ARGV.set(argv_in.to_vec());
    // the config file comes first: user modes join the built-in ones. `--config`
    // / `--no-config` may sit anywhere; they are taken out here (so a subcommand
    // can follow them) and kept for a mode switch.
    let explicit = config_flag(argv_in);
    let (argv, cfg_flags) = split_config_flags(argv_in);
    let argv = &argv[..];
    // doctor reports a broken config instead of stopping on it
    if argv.first().map(String::as_str) == Some("doctor") {
        quiet_on_sigpipe();
        return Ok(doctor::run(explicit.as_deref()));
    }
    let cfg = match config::load(explicit.as_deref()) {
        Ok(c) => c,
        Err(errs) => {
            for e in &errs {
                eprintln!("lvim-fuzzy: {e}");
            }
            eprintln!("lvim-fuzzy: {} problem(s) in the config", errs.len());
            return Ok(2);
        }
    };
    let mut all = modes::builtin();
    all.extend(cfg.modes.iter().cloned());
    let _ = MODES.set(all);
    let _ = PREVIEWERS.set(cfg.previewers.clone());

    if matches!(
        argv.first().map(String::as_str),
        Some("--list-modes" | "--describe-mode" | "--complete-list" | "config")
    ) {
        quiet_on_sigpipe();
    }
    match argv.first().map(String::as_str) {
        Some("--list-modes") => return Ok(list_modes()),
        Some("--complete-list") => {
            return Ok(complete_list(
                argv.get(1).map(String::as_str).unwrap_or(""),
                argv.get(2).map(String::as_str),
            ));
        }
        Some("--describe-mode") => {
            return Ok(describe_mode(argv.get(1).map(String::as_str).unwrap_or("")));
        }
        _ => {}
    }

    // a subcommand: a mode name, `mode`, or `config`
    let mut rest: Vec<String> = argv.to_vec();
    let mut mode: Option<&'static modes::Mode> = None;
    let mut query_words: Vec<String> = Vec::new();
    if let Some(first) = argv
        .first()
        .filter(|a| !a.starts_with('-') && !a.starts_with('+'))
    {
        rest.remove(0);
        match first.as_str() {
            "config" => {
                return Ok(match rest.first().map(String::as_str) {
                    Some("path") => {
                        match config::path(explicit.as_deref()) {
                            Some(p) => println!("{}", p.display()),
                            None => println!("(no config file)"),
                        }
                        0
                    }
                    Some("check") | None => {
                        match &cfg.path {
                            Some(p) => {
                                println!("{}: OK ({} user mode(s))", p.display(), cfg.modes.len())
                            }
                            None => println!(
                                "no config file ({})",
                                config::path(None)
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_default()
                            ),
                        }
                        0
                    }
                    Some(other) => {
                        return Err(format!("unknown config command: {other} (check, path)"));
                    }
                });
            }
            "pool" => {
                return match rest.first().map(String::as_str) {
                    // run the pool (in the foreground: a service or `&` puts it in the background)
                    None => {
                        let o = options::Options::parse(&cfg.args)?;
                        pool::serve(&o.nvim, o.nvim_appname.as_deref(), cfg.path.as_deref())
                    }
                    Some(cmd @ ("status" | "reload" | "stop")) => match pool::request(cmd) {
                        Some(out) => {
                            if cmd == "status" {
                                println!("{}", pool::dir().display());
                                if out.is_empty() {
                                    println!("  (no instances)");
                                }
                                for l in out.lines() {
                                    let f: Vec<&str> = l.split('\t').collect();
                                    println!("  #{} {}", f.first().unwrap_or(&""), f.get(1).unwrap_or(&""));
                                }
                            }
                            Ok(0)
                        }
                        None => {
                            eprintln!("lvim-fuzzy: no pool runs ({})", pool::dir().display());
                            Ok(1)
                        }
                    },
                    Some(other) => Err(format!("unknown pool command: {other} (status, reload, stop)")),
                };
            }
            "mode" | "modes" => {
                let exe = placeholder::quote(&self_exe());
                let pass: String = rest
                    .iter()
                    .map(|a| format!(" {}", placeholder::quote(a)))
                    .collect();
                let mut a = vec![
                    "--title=Modes".to_string(),
                    "--mode-name=mode".to_string(),
                    format!("--command={exe} --list-modes"),
                    "--delimiter=\t".to_string(),
                    "--no-sort".to_string(),
                    format!("--preview={exe} --describe-mode {{1}}"),
                    format!("--bind=enter:become({exe} {{1}}{pass})"),
                ];
                a.extend(rest.iter().cloned());
                let mut pass = rest.clone();
                pass.extend(cfg_flags.iter().cloned());
                return run_with(&cfg, a, pass);
            }
            name => match find_mode(name) {
                Some(m) => mode = Some(m),
                None => {
                    return Err(format!(
                        "unknown mode or subcommand: {name} (`lvim-fuzzy mode` lists the modes)"
                    ));
                }
            },
        }
        // words after the mode name, up to the first option, are the starting query
        while let Some(w) = rest.first().filter(|w| !w.starts_with('-')) {
            query_words.push(w.clone());
            rest.remove(0);
        }
    }
    // `--mode NAME` works too
    if mode.is_none()
        && let Some(i) = rest
            .iter()
            .position(|a| a == "--mode" || a.starts_with("--mode="))
    {
        let name = if let Some(v) = rest[i].strip_prefix("--mode=") {
            let v = v.to_string();
            rest.remove(i);
            v
        } else {
            rest.remove(i);
            if i < rest.len() {
                rest.remove(i)
            } else {
                String::new()
            }
        };
        mode = Some(find_mode(&name).ok_or_else(|| format!("unknown mode: {name}"))?);
    }
    let mut pre = Vec::new();
    if let Some(m) = mode {
        let missing = m.missing();
        if !missing.is_empty() {
            return Err(format!("mode `{}` needs: {}", m.name, missing.join(", ")));
        }
        pre = m.args(&query_words.join(" "));
    } else if !query_words.is_empty() {
        rest.insert(0, format!("--query={}", query_words.join(" ")));
    }
    let mut pass = rest.clone();
    pass.extend(cfg_flags.iter().cloned());
    pre.extend(rest);
    run_with(&cfg, pre, pass)
}

/// argv without `--config PATH` / `--config=PATH` / `--no-config`, and those flags.
fn split_config_flags(argv: &[String]) -> (Vec<String>, Vec<String>) {
    let mut rest = Vec::new();
    let mut flags = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        if a == "--no-config" || a.starts_with("--config=") {
            flags.push(a.clone());
        } else if a == "--config" {
            flags.push(a.clone());
            if let Some(v) = argv.get(i + 1) {
                flags.push(v.clone());
                i += 1;
            }
        } else {
            rest.push(a.clone());
        }
        i += 1;
    }
    (rest, flags)
}

/// For commands that only print: a closed pipe (`| head`) ends the process
/// quietly, as for any Unix tool, instead of a write error / panic. Not for the
/// finder itself, whose popup relies on seeing EPIPE as an error.
fn quiet_on_sigpipe() {
    #[cfg(unix)]
    // SAFETY: restoring the default disposition of one signal at start-up.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

/// `--config PATH` / `--no-config` from argv (read before anything else).
fn config_flag(argv: &[String]) -> Option<String> {
    let mut it = argv.iter();
    let mut out = None;
    while let Some(a) = it.next() {
        if a == "--no-config" {
            out = Some(String::new());
        } else if a == "--config" {
            out = it.next().cloned();
        } else if let Some(v) = a.strip_prefix("--config=") {
            out = Some(v.to_string());
        }
    }
    out
}

/// Parse config + environment + `args` (in that order) and run.
fn run_with(
    cfg: &config::Config,
    args: Vec<String>,
    passthrough: Vec<String>,
) -> Result<i32, String> {
    let _ = PASSTHROUGH.set(passthrough);
    // theme: --theme beats $LVIM_FUZZY_THEME beats the config's `theme`
    let mut all = cfg.args.clone();
    all.extend(Options::env_args()?);
    if let Ok(t) = std::env::var("LVIM_FUZZY_THEME")
        && !t.is_empty()
    {
        all.push(format!("--theme={t}"));
    }
    all.extend(args);
    let mut opts = Options::parse(&all)?;
    if std::env::var_os("LVIM_FUZZY_POPUP_CHILD").is_some() {
        // inside the popup pane the finder takes the whole pane
        opts.height = None;
    }
    if opts.version || opts.help || opts.man || opts.bind_help || opts.shell.is_some() {
        quiet_on_sigpipe();
    }
    if opts.version {
        println!("{VERSION} (lvim-fuzzy)");
        return Ok(0);
    }
    if opts.help {
        print!("{}", help::HELP);
        return Ok(0);
    }
    if opts.config_template {
        quiet_on_sigpipe();
        print!("{}", include_str!("config.template.toml"));
        return Ok(0);
    }
    if opts.bind_help {
        // the keys and actions --bind understands
        println!(
            "Keys: ctrl-[a-z], alt-[key], ctrl-alt-[a-z], f1..f12, enter, space, bspace, tab, shift-tab, esc,"
        );
        println!(
            "      del, insert, up, down, left, right, home, end, page-up, page-down, shift-/ctrl-/alt- arrows,"
        );
        println!(
            "      left-click, right-click, double-click, scroll-up, scroll-down, preview-scroll-up/down, any char"
        );
        println!(
            "Events: start, load, change, focus, result, resize, one, zero, multi, backward-eof, jump, jump-cancel"
        );
        println!();
        println!("Actions:");
        for a in keys::ACTIONS {
            match a.strip_suffix('(') {
                Some(n) => println!("  {n}(...)"),
                None => println!("  {a}"),
            }
        }
        return Ok(0);
    }
    if let Some(sh) = opts.shell {
        let script = match sh {
            options::Shell::Zsh => shell::zsh(&cfg.shell),
            options::Shell::Bash => shell::bash(&cfg.shell),
            options::Shell::Fish => shell::fish(&cfg.shell),
            options::Shell::Nushell => {
                return Err("nushell integration is not available yet (zsh, bash, fish)".into());
            }
        };
        print!("{script}");
        return Ok(0);
    }
    if opts.man {
        let page = help::man();
        // into a file or a pipe: the roff source (`lvim-fuzzy --man > lvim-fuzzy.1`)
        if !std::io::stdout().is_terminal() {
            print!("{page}");
            return Ok(0);
        }
        // on a terminal: through man(1) when it is there, else the page as text
        let shown = std::process::Command::new("man")
            .args(["-l", "-"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut c| {
                use std::io::Write as _;
                if let Some(mut i) = c.stdin.take() {
                    let _ = i.write_all(page.as_bytes());
                }
                c.wait()
            })
            .is_ok_and(|s| s.success());
        if !shown {
            print!("{}", help::HELP);
        }
        return Ok(0);
    }
    if opts.filter.is_some() {
        return filter(&opts);
    }
    // --popup: in Zellij / tmux, run again inside a floating pane (once: the
    // pane's run is marked); elsewhere just run here
    if let Some(p) = &opts.popup
        && std::env::var_os("LVIM_FUZZY_POPUP_CHILD").is_none()
        && popup::host().is_some()
    {
        let args = strip_popup(ORIGINAL_ARGV.get().map(Vec::as_slice).unwrap_or(&[]));
        return popup::run(p, &args);
    }
    tui::run(opts)
}

/// The user's argv as given (re-run inside a popup pane).
pub static ORIGINAL_ARGV: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// argv without `--popup[=…]` / `--tmux[=…]` (and a separate value after them).
fn strip_popup(argv: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip_value = false;
    for a in argv {
        if skip_value {
            skip_value = false;
            if popup::parse(a).is_ok() && !a.starts_with('-') {
                continue;
            }
        }
        if a == "--popup" || a == "--tmux" {
            skip_value = true;
            continue;
        }
        if a.starts_with("--popup=") || a.starts_with("--tmux=") {
            continue;
        }
        out.push(a.clone());
    }
    out
}

pub fn builder(opts: &Options) -> ItemBuilder {
    ItemBuilder {
        delimiter: opts.delimiter.clone(),
        with_nth: opts.with_nth.clone(),
        nth: opts.nth.clone(),
        ansi: opts.ansi,
    }
}

/// `--filter`: print the matches for the query and exit.
/// One record as an item (the finder trims a CR before the newline).
fn build_line(b: &item::ItemBuilder, index: u32, rec: &[u8], sep: u8) -> (item::Item, String) {
    let line = match std::str::from_utf8(strip_cr(rec, sep)) {
        Ok(t) => t.to_owned(),
        Err(_) => String::from_utf8_lossy(strip_cr(rec, sep)).into_owned(),
    };
    b.build(index, line)
}

/// A CR before the newline is not part of the record (the finder's view).
fn strip_cr(rec: &[u8], sep: u8) -> &[u8] {
    if sep == b'\n' && rec.last() == Some(&b'\r') {
        &rec[..rec.len() - 1]
    } else {
        rec
    }
}

/// Items and their store for records `bounds` of `data`, numbered from
/// `base`, on every core. Plain items (no --with-nth / --nth / --ansi) of
/// valid UTF-8 hold no text: the store takes the bytes straight from `data`.
fn build_part(
    b: &item::ItemBuilder,
    base: usize,
    bounds: &[(usize, usize)],
    data: &[u8],
    sep: u8,
) -> item::Part {
    use rayon::prelude::*;
    let mut part = item::Part::new();
    let span = bounds.first().map_or(0, |f| f.0)..bounds.last().map_or(0, |l| l.1);
    if b.plain() && std::str::from_utf8(&data[span]).is_ok() {
        // SAFETY: the whole span of these records is valid UTF-8 (checked
        // above), and a record boundary (a separator byte) is a char boundary.
        let texts: Vec<&str> = bounds
            .par_iter()
            .map(|&(a, z)| unsafe { std::str::from_utf8_unchecked(strip_cr(&data[a..z], sep)) })
            .collect();
        part.items = texts
            .par_iter()
            .enumerate()
            .map(|(i, t)| item::Item::plain((base + i) as u32, t))
            .collect();
        part.store.extend_par(&texts);
        return part;
    }
    let built: Vec<(item::Item, String)> = bounds
        .par_iter()
        .enumerate()
        .map(|(i, &(a, z))| build_line(b, (base + i) as u32, &data[a..z], sep))
        .collect();
    let texts: Vec<&str> = built.iter().map(|(_, t)| t.as_str()).collect();
    part.store.extend_par(&texts);
    drop(texts);
    part.items = built.into_iter().map(|(it, _)| it).collect();
    part
}

/// All of stdin: mapped when it is a regular file (no copy), read otherwise.
enum Input {
    Mapped(*const u8, usize),
    Owned(Vec<u8>),
}

impl Input {
    fn read(stdin: &std::io::Stdin) -> std::io::Result<Input> {
        use std::os::fd::AsRawFd;
        let fd = stdin.as_raw_fd();
        // SAFETY: fstat on our own stdin into a zeroed struct.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let is_file = unsafe { libc::fstat(fd, &mut st) } == 0
            && (st.st_mode & libc::S_IFMT) == libc::S_IFREG;
        if is_file {
            // from the current offset (stdin may have been read from already)
            // SAFETY: lseek on our own fd.
            let off = unsafe { libc::lseek(fd, 0, libc::SEEK_CUR) };
            let len = (st.st_size as i64 - off.max(0)) as usize;
            if off == 0 && len > 0 {
                // SAFETY: a private read-only mapping of the whole file; it is
                // never unmapped (the process ends after --filter).
                let p = unsafe {
                    libc::mmap(
                        std::ptr::null_mut(),
                        len,
                        libc::PROT_READ,
                        libc::MAP_PRIVATE,
                        fd,
                        0,
                    )
                };
                if p != libc::MAP_FAILED {
                    return Ok(Input::Mapped(p as *const u8, len));
                }
            }
        }
        let mut v = Vec::with_capacity(if is_file {
            st.st_size as usize + 1
        } else {
            1 << 22
        });
        std::io::Read::read_to_end(&mut stdin.lock(), &mut v)?;
        Ok(Input::Owned(v))
    }

    /// Only a regular file (mapped): a pipe is read the usual way, line by line.
    fn read_file_only(stdin: &std::io::Stdin) -> std::io::Result<Input> {
        use std::os::fd::AsRawFd;
        let fd = stdin.as_raw_fd();
        // SAFETY: fstat on our own stdin into a zeroed struct.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let is_file = unsafe { libc::fstat(fd, &mut st) } == 0
            && (st.st_mode & libc::S_IFMT) == libc::S_IFREG;
        if !is_file || st.st_size == 0 {
            return Err(std::io::Error::other("not a regular file"));
        }
        Input::read(stdin)
    }

    fn bytes(&self) -> &[u8] {
        match self {
            // SAFETY: the mapping is `len` bytes long and lives as long as self.
            Input::Mapped(p, len) => unsafe { std::slice::from_raw_parts(*p, *len) },
            Input::Owned(v) => v,
        }
    }
}

/// --filter without --with-nth, --nth and --ansi: the line is what is
/// matched and what is printed, so no items are needed.
fn plain_filter(opts: &Options) -> bool {
    opts.with_nth.is_none() && opts.nth.is_none() && !opts.ansi
}

/// --filter over plain lines: the search store built straight from them,
/// the matches printed (as `filter` does with items).
fn filter_lines(opts: &Options, texts: &[&str]) -> Result<i32, String> {
    // --tail N: only the last N lines take part (as fzf; not unsorted)
    let texts = match opts.tail {
        Some(n) if opts.sort && texts.len() > n => &texts[texts.len() - n..],
        _ => texts,
    };
    let mut store = lvim_fuzzy_core::Store::new();
    store.extend_par(texts);
    tui::stage("filter: store");
    let query = opts.filter.clone().unwrap_or_default();
    let hits = lvim_fuzzy_core::search(&store, &query, &match_settings(opts), None, None).unwrap();
    tui::stage("filter: searched");
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let end = if opts.print0 { b"\0" } else { b"\n" };
    if opts.print_query {
        out.write_all(query.as_bytes()).ok();
        out.write_all(end).ok();
    }
    for i in 0..hits.len() {
        let text = texts[hits.get(i).unwrap() as usize];
        let ok = match &opts.accept_nth {
            Some(r) => out.write_all(fields::extract(text, &opts.delimiter, r).as_bytes()),
            None => out.write_all(text.as_bytes()),
        };
        if ok.is_err() || out.write_all(end).is_err() {
            break;
        }
    }
    out.flush().ok();
    let code = if hits.len() == 0 { 1 } else { 0 };
    // the process ends here: freeing the store would only cost time
    std::mem::forget(hits);
    std::mem::forget(store);
    Ok(code)
}

fn filter(opts: &Options) -> Result<i32, String> {
    use rayon::prelude::*;
    let b = builder(opts);
    let sep = if opts.read0 { 0u8 } else { b'\n' };
    let mut header = opts.header_lines;
    let mut lines: Vec<String> = Vec::new();
    // stdin read here directly (else through the usual sources)
    let direct = opts.command.is_none() && !std::io::stdin().is_terminal();
    let mut built: Vec<(item::Item, String)> = if direct {
        // stdin: read it whole, find the records with memchr and build the
        // items on every core (no per-line reads or channel hops)
        let stdin = std::io::stdin();
        tui::stage("filter: start");
        let input = Input::read(&stdin).map_err(|e| e.to_string())?;
        tui::stage("filter: read");
        let data: &[u8] = input.bytes();
        // blocks of ~256 KiB cut at record ends, each split and built on its own core
        let mut blocks = Vec::new();
        let mut from = 0;
        while from < data.len() {
            let want = (from + (1 << 18)).min(data.len());
            let end = if want == data.len() {
                want
            } else {
                memchr::memchr(sep, &data[want..]).map_or(data.len(), |p| want + p + 1)
            };
            blocks.push((from, end));
            from = end;
        }
        // record bounds per block in parallel, then every item built in place
        let per_block: Vec<Vec<(usize, usize)>> = blocks
            .par_iter()
            .map(|&(a, z)| {
                let mut v = Vec::with_capacity((z - a) / 32 + 1);
                let mut s = a;
                for p in memchr::memchr_iter(sep, &data[a..z]) {
                    v.push((s, a + p));
                    s = a + p + 1;
                }
                if s < z {
                    v.push((s, z));
                }
                v
            })
            .collect();
        let bounds: Vec<(usize, usize)> = per_block.concat();
        tui::stage("filter: bounds");
        let skip = header.min(bounds.len());
        header = 0;
        // the usual case (the match target is the line itself, valid UTF-8):
        // no items at all, the lines go straight into the search store
        if plain_filter(opts)
            && blocks
                .par_iter()
                .all(|&(a, z)| std::str::from_utf8(&data[a..z]).is_ok())
        {
            // SAFETY: every block (so every record in it) was checked above.
            let texts: Vec<&str> = bounds[skip..]
                .par_iter()
                .map(|&(a, z)| unsafe { std::str::from_utf8_unchecked(&data[a..z]) })
                .collect();
            tui::stage("filter: utf-8 checked, texts");
            let code = filter_lines(opts, &texts);
            std::mem::forget(texts);
            std::mem::forget(input);
            return code;
        }
        let built: Vec<(item::Item, String)> = bounds[skip..]
            .par_iter()
            .enumerate()
            .map(|(i, &(a, z))| {
                // (fzf's --filter keeps a CR before the newline)
                let rec = &data[a..z];
                let line = match std::str::from_utf8(rec) {
                    Ok(t) => t.to_owned(),
                    Err(_) => String::from_utf8_lossy(rec).into_owned(),
                };
                b.build(i as u32, line)
            })
            .collect();
        std::mem::forget(input);
        built
    } else {
        Vec::new()
    };
    let (tx, rx) = channel();
    if !direct {
        start_source(
            opts,
            Feed {
                tx,
                epoch: 0,
                build: None,
                child: None,
            },
        );
    } else {
        drop(tx);
    }
    for (_, ev) in rx {
        match ev {
            Event::Lines(batch) => {
                for l in batch {
                    if header > 0 {
                        header -= 1;
                        continue;
                    }
                    lines.push(l);
                }
            }
            // (only sent when a Feed builds items; this one does not)
            Event::Items(_) | Event::Reserve(_) => {}
            Event::Eof => break,
        }
    }
    if !direct && plain_filter(opts) {
        let texts: Vec<&str> = lines[..].iter().map(String::as_str).collect();
        let code = filter_lines(opts, &texts);
        std::mem::forget(texts);
        std::mem::forget(lines);
        return code;
    }
    if !direct {
        // other sources: build what the reader sent, on every core
        built = lines
            .into_par_iter()
            .enumerate()
            .map(|(i, l)| b.build(i as u32, l))
            .collect();
    }
    // the items and their store (the match targets)
    let mut items = item::Items::default();
    items.nth = opts.nth.is_some();
    {
        let mut part = item::Part::new();
        let texts: Vec<&str> = built.iter().map(|(_, t)| t.as_str()).collect();
        part.store.extend_par(&texts);
        drop(texts);
        part.items = built.into_iter().map(|(it, _)| it).collect();
        items.extend(part);
    }
    // --tail N: only the last N items take part (as fzf)
    // (fzf's --filter streams unsorted results and ignores --tail then)
    if let Some(n) = opts.tail
        && opts.sort
    {
        items.keep_last(n);
    }
    let query = opts.filter.clone().unwrap_or_default();
    let hits = run_match(&items, &query, &match_settings(opts), None).unwrap();
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let end = if opts.print0 { b"\0" } else { b"\n" };
    if opts.print_query {
        out.write_all(query.as_bytes()).ok();
        out.write_all(end).ok();
    }
    for i in 0..hits.len() {
        let it = items.at(hits.get(i).unwrap() as usize);
        let line = match &opts.accept_nth {
            Some(r) => fields::extract(it.raw(), &opts.delimiter, r),
            None => it.raw().to_string(),
        };
        if out.write_all(line.as_bytes()).is_err() || out.write_all(end).is_err() {
            break;
        }
    }
    out.flush().ok();
    let code = if hits.len() == 0 { 1 } else { 0 };
    // the process ends here: freeing a million items would only cost time
    std::mem::forget(hits);
    std::mem::forget(items);
    Ok(code)
}
