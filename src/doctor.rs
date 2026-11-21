//! `lvim-fuzzy doctor`: every problem at once — the config, the themes, the
//! modes and previewers, Neovim, the programs the modes use, the terminal and
//! themer. Nothing stops at the first issue; the exit code is 1 when there is
//! an error, 0 otherwise (warnings alone do not fail it).

use std::process::{Command, Stdio};

use crate::{config, lvim, modes};

#[derive(Default)]
struct Report {
    errors: usize,
    warnings: usize,
}

impl Report {
    fn section(&self, name: &str) {
        println!();
        println!("{name}");
    }
    fn ok(&self, msg: impl AsRef<str>) {
        println!("  ✓ {}", msg.as_ref());
    }
    fn info(&self, msg: impl AsRef<str>) {
        println!("  · {}", msg.as_ref());
    }
    fn warn(&mut self, msg: impl AsRef<str>) {
        self.warnings += 1;
        println!("  ! {}", msg.as_ref());
    }
    fn error(&mut self, msg: impl AsRef<str>) {
        self.errors += 1;
        println!("  ✗ {}", msg.as_ref());
    }
}

/// Run `prog args` with a timeout-ish guard; Some(stdout) when it succeeded.
fn output(prog: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(prog)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The roles a theme file is expected to define.
fn expected_roles() -> Vec<String> {
    let mut v: Vec<String> = lvim::Theme::ansi().roles.keys().cloned().collect();
    v.sort();
    v
}

pub fn run(explicit: Option<&str>) -> i32 {
    let mut r = Report::default();
    println!(
        "lvim-fuzzy doctor — {} ({})",
        env!("CARGO_PKG_VERSION"),
        crate::self_exe()
    );

    // ── config ──────────────────────────────────────────────────────────────
    r.section("Config");
    let mut cfg_theme: Option<String> = None;
    let mut user_modes = Vec::new();
    let mut previewers = Vec::new();
    let mut shell_cfg = crate::shell::ShellCfg::default();
    match config::path(explicit) {
        None => r.info("no config file in use (--no-config)"),
        Some(p) if !p.is_file() => {
            r.warn(format!(
                "no config file at {} — the defaults apply",
                p.display()
            ));
        }
        Some(p) => match config::load(explicit) {
            Ok(c) => {
                r.ok(format!("{} is valid", p.display()));
                cfg_theme = c
                    .args
                    .iter()
                    .find_map(|a| a.strip_prefix("--theme=").map(str::to_string));
                user_modes = c.modes.clone();
                previewers = c.previewers.clone();
                shell_cfg = c.shell.clone();
                r.info(format!(
                    "{} user mode(s), {} previewer(s)",
                    user_modes.len(),
                    previewers.len()
                ));
            }
            Err(errs) => {
                for e in errs {
                    r.error(e);
                }
            }
        },
    }

    // ── themes ──────────────────────────────────────────────────────────────
    r.section("Themes");
    let dirs = lvim::theme_dirs();
    let dir = dirs.first().cloned();
    let mut found: Vec<String> = Vec::new();
    match &dir {
        Some(d) if d.is_dir() => {
            let expected = expected_roles();
            let mut entries: Vec<_> = std::fs::read_dir(d)
                .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).collect())
                .unwrap_or_default();
            entries.sort();
            let mut good = 0;
            for f in entries.iter().filter(|p| p.extension().is_some_and(|x| x == "toml")) {
                let stem = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                match std::fs::read_to_string(f).map_err(|e| e.to_string()).and_then(|t| lvim::Theme::parse(&t)) {
                    Ok(t) => {
                        let missing: Vec<&String> = expected.iter().filter(|k| !t.roles.contains_key(*k)).collect();
                        if missing.is_empty() {
                            good += 1;
                        } else {
                            r.warn(format!(
                                "{stem}: {} role(s) missing (they use the terminal's colours): {}",
                                missing.len(),
                                missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                            ));
                        }
                        found.push(stem);
                    }
                    Err(e) => r.error(format!("{}: {e}", f.display())),
                }
            }
            if found.is_empty() {
                r.warn(format!("{} holds no themes — run themer, or copy lvim-colorscheme's extras/lvim_fuzzy/*.toml there", d.display()));
            } else {
                r.ok(format!("{} theme(s) in {} ({good} complete)", found.len(), d.display()));
            }
        }
        Some(d) => r.warn(format!(
            "no themes directory ({}) — the terminal's colours are used; themer creates it on the next switch",
            d.display()
        )),
        None => r.warn("no home directory: themes cannot be found"),
    }
    // which theme a start would use, and from where
    let env_theme = std::env::var("LVIM_FUZZY_THEME")
        .ok()
        .filter(|s| !s.is_empty());
    let (name, source) = match (&env_theme, &cfg_theme) {
        (Some(e), _) => (Some(e.clone()), "$LVIM_FUZZY_THEME"),
        (None, Some(c)) => (Some(c.clone()), "the config"),
        _ => (None, ""),
    };
    match name {
        None => r.info("no theme set — the terminal's own colours are used"),
        Some(n) => match lvim::Theme::load(&n) {
            Ok(_) => r.ok(format!("theme \"{n}\" (from {source}) loads")),
            Err(e) => r.error(format!("theme \"{n}\" (from {source}): {e} — the finder falls back to the terminal's colours")),
        },
    }

    // ── themer ──────────────────────────────────────────────────────────────
    r.section("themer");
    let home = std::env::var("HOME").unwrap_or_default();
    let target = std::path::Path::new(&home).join(".config/themer/targets/lvim-fuzzy.toml");
    if !modes::on_path("themer") {
        r.info("themer is not installed — themes are managed by hand");
    } else if target.is_file() {
        r.ok(format!(
            "themer switches lvim-fuzzy's theme ({})",
            target.display()
        ));
    } else {
        r.warn(format!("themer is installed but has no lvim-fuzzy target ({}) — its theme switches skip lvim-fuzzy", target.display()));
    }

    // ── modes ───────────────────────────────────────────────────────────────
    r.section("Modes");
    let builtin = modes::builtin();
    let usable = builtin.iter().filter(|m| m.missing().is_empty()).count();
    r.ok(format!(
        "{usable} of {} built-in modes can run",
        builtin.len()
    ));
    for m in builtin.iter().filter(|m| !m.missing().is_empty()) {
        r.info(format!("{}: needs {}", m.name, m.missing().join(", ")));
    }
    for m in &user_modes {
        if m.missing().is_empty() {
            r.ok(format!("user mode {}", m.name));
        } else {
            r.warn(format!(
                "user mode {}: needs {}",
                m.name,
                m.missing().join(", ")
            ));
        }
    }

    // ── previews ────────────────────────────────────────────────────────────
    r.section("Previews");
    let nvim = output("nvim", &["--version"]).and_then(|v| v.lines().next().map(str::to_string));
    match &nvim {
        Some(v) => {
            r.ok(format!("Neovim: {v}"));
            // does the user's Neovim have the lvim-fuzzy plugin's preview module?
            let probe = output(
                "nvim",
                &[
                    "--headless",
                    "--cmd",
                    "let g:lvim_fuzzy_preview = 1",
                    "-c",
                    "lua io.stdout:write(pcall(require, 'lvim-fuzzy.preview') and 'yes' or 'no')",
                    "-c",
                    "qa!",
                ],
            );
            match probe.as_deref() {
                Some(s) if s.ends_with("yes") => r.ok("lvim-fuzzy.preview is available: the Neovim preview marks the query"),
                _ => r.warn("lvim-fuzzy.preview is not loaded in Neovim: the preview opens files but does not mark the query (install the lvim-fuzzy plugin)"),
            }
        }
        None => r.info("Neovim is not installed — the built-in file preview is used"),
    }
    if nvim.is_some() {
        match crate::pool::request("status") {
            Some(out) => {
                let free = out.lines().filter(|l| l.contains("\tfree\t")).count();
                let warming = out.lines().filter(|l| l.contains("\twarming\t")).count();
                let taken = out.lines().filter(|l| l.contains("\ttaken\t")).count();
                if free + warming > 0 {
                    r.ok(format!(
                        "pool: {free} free, {warming} warming, {taken} in use ({})",
                        crate::pool::dir().display()
                    ));
                } else {
                    r.warn("pool: runs but has no free instance (Neovim fails to warm up? see its log)");
                }
            }
            None => r.info("pool: not running — each finder starts its own Neovim (`lvim-fuzzy pool`, see the README)"),
        }
    }
    for p in &previewers {
        let missing: Vec<&String> = p.requires.iter().filter(|x| !modes::on_path(x)).collect();
        if missing.is_empty() {
            r.ok(format!("previewer {}", p.name));
        } else {
            r.warn(format!(
                "previewer {}: needs {} (skipped in the chain)",
                p.name,
                missing
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    // ── shell keys ──────────────────────────────────────────────────────────
    r.section("Shell integration");
    if shell_cfg.binds.is_empty() && shell_cfg.completion.is_none() {
        r.info("no [shell] keys set — `lvim-fuzzy --zsh` binds nothing");
    } else {
        for b in &shell_cfg.binds {
            let act = match b.act {
                crate::shell::Act::Insert => "insert",
                crate::shell::Act::Cd => "cd",
                crate::shell::Act::Replace => "replace",
            };
            r.ok(format!("{} → {} ({act})", b.key, b.mode));
        }
        if let Some(t) = &shell_cfg.completion {
            r.ok(format!("completion trigger {t}<Tab>"));
        }
    }

    // ── programs ────────────────────────────────────────────────────────────
    r.section("Programs");
    for (prog, what) in [
        ("rg", "grep, grep-filter"),
        ("fd", "faster file listing in custom modes"),
        ("git", "the git-* modes"),
        ("bat", "a common user previewer"),
    ] {
        if modes::on_path(prog) {
            r.ok(format!("{prog}"));
        } else {
            r.info(format!("{prog} not found ({what})"));
        }
    }

    // ── terminal ────────────────────────────────────────────────────────────
    r.section("Terminal");
    let term = std::env::var("TERM").unwrap_or_default();
    let colorterm = std::env::var("COLORTERM").unwrap_or_default();
    if term.is_empty() || term == "dumb" {
        r.warn(format!(
            "TERM is \"{term}\": the finder needs a real terminal"
        ));
    } else {
        r.ok(format!("TERM={term}"));
    }
    if colorterm == "truecolor" || colorterm == "24bit" {
        r.ok("truecolor (the themes' exact colours)");
    } else {
        r.warn(format!(
            "COLORTERM=\"{colorterm}\": themes are 24-bit; without truecolor they are approximated"
        ));
    }
    // what `theme = "terminal"` (and the popups' background) is built from
    match crate::lvim::terminal_colors() {
        None => r.warn(
            "the terminal did not answer the colour queries (OSC 10/11): the `terminal` theme uses plain ANSI colours",
        ),
        Some(tc) => {
            let answered = tc.palette.iter().filter(|c| c.is_some()).count();
            let hex = |c: crossterm::style::Color| match c {
                crossterm::style::Color::Rgb { r, g, b } => format!("#{r:02x}{g:02x}{b:02x}"),
                o => format!("{o:?}"),
            };
            r.ok(format!("terminal colours: fg {}, bg {}", hex(tc.fg), hex(tc.bg)));
            if crate::lvim::Theme::from_terminal(tc).is_some() {
                r.ok(format!("palette: {answered}/16 colours answered (OSC 4) — the `terminal` theme uses them"));
            } else {
                r.warn(format!(
                    "palette: {answered}/16 colours answered (OSC 4) — too few for the `terminal` theme, which falls back to plain ANSI colours"
                ));
            }
        }
    }
    r.info("file icons need a Nerd Font in the terminal");
    for (k, v) in std::env::vars().filter(|(k, _)| k.starts_with("LVIM_FUZZY_")) {
        r.info(format!("{k}={v}"));
    }

    println!();
    println!("{} error(s), {} warning(s)", r.errors, r.warnings);
    if r.errors > 0 { 1 } else { 0 }
}
