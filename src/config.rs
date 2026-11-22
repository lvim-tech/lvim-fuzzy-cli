//! The config file: `$XDG_CONFIG_HOME/lvim-fuzzy/config.toml` (or
//! `$LVIM_FUZZY_CONFIG`, or `--config PATH`; `--no-config` skips it).
//!
//! ```toml
//! theme = "LvimNord_dark"
//! style = "lvim"            # lvim | default (fzf)
//! bar = "bottom"            # the button bar: top | bottom
//! title_pos = "left"        # left | center | right
//! options = ["--cycle"]     # any lvim-fuzzy / fzf options
//!
//! [text]                    # the texts
//! search = "Search"
//! no_matches = "[no matches]"
//! loading = "loading…"
//! marker = "➤"
//!
//! [icons]                   # file icons: by extension, file name or `dir`
//! enabled = true
//! rs = "" # or ["", "orange"]: an icon and a theme colour
//!
//! [preview]
//! order = ["nvim", "file", "command"]
//! nvim = "nvim"
//! nvim_appname = "nvim-preview"
//!
//! [pool]                    # warm Neovim instances from `lvim-fuzzy pool`
//! enabled = true
//!
//! [keys]
//! "ctrl-o" = "execute(less {})"
//!
//! [modes.todo]              # a user mode: menu entry + `lvim-fuzzy todo`
//! description = "TODO and FIXME comments"
//! title = "TODO"
//! command = "rg -n --no-heading 'TODO|FIXME'"
//! target = "{1}:{2}"
//! scope = "line"
//! ```
//!
//! The whole file is validated before anything starts, and every problem is
//! reported at once. A user mode may not take a built-in mode's name.

use std::path::PathBuf;

use crate::modes::{self, Mode};

#[derive(Debug)]
pub struct Config {
    pub path: Option<PathBuf>,
    /// Options derived from the file, applied before everything else.
    pub args: Vec<String>,
    pub modes: Vec<Mode>,
    pub previewers: Vec<crate::previewers::Previewer>,
    /// `[plugins.<name>]`: lfz's own overrides of the plugins' settings (a table per plugin).
    pub plugins: toml::Table,
    /// `[shell]`: the keys `lvim-fuzzy --zsh/--bash/--fish` binds (none by default).
    pub shell: crate::shell::ShellCfg,
}

/// `[text]` (global or a look's): the look's texts as options.
fn look_text(
    v: &toml::Value,
    what: &str,
    args: &mut Vec<String>,
    errs: &mut Vec<String>,
    at: &dyn Fn(&str) -> String,
) {
    let Some(t) = v.as_table() else {
        errs.push(format!("{}: [{what}] must be a table", at(what)));
        return;
    };
    for (tk, tv) in t {
        let opt = match tk.as_str() {
            "search" => "prompt",
            "no_matches" => "empty-text",
            "loading" => "loading-text",
            "marker" => "marker",
            other => {
                errs.push(format!(
                    "{}: unknown key `{what}.{other}` (search, no_matches, loading, marker)",
                    at(other)
                ));
                continue;
            }
        };
        match tv.as_str() {
            Some(s) => args.push(format!("--{opt}={s}")),
            None => errs.push(format!("{}: {what}.{tk} must be a string", at(tk))),
        }
    }
}

/// `[icons]`: `enabled`, and NAME = "icon" or ["icon", "color"].
fn look_icons(
    v: &toml::Value,
    what: &str,
    args: &mut Vec<String>,
    errs: &mut Vec<String>,
    at: &dyn Fn(&str) -> String,
) {
    let Some(t) = v.as_table() else {
        errs.push(format!("{}: [{what}] must be a table", at(what)));
        return;
    };
    for (ik, iv) in t {
        if ik == "mode" {
            match iv.as_str() {
                Some(m) if lvim_fuzzy_ui::iconimg::Mode::parse(m).is_some() => args.push(format!("--icon-mode={m}")),
                _ => errs.push(format!("{}: {what}.mode must be \"auto\", \"image\" or \"font\"", at(ik))),
            }
            continue;
        }
        if ik == "zellij_images" {
            match iv.as_bool() {
                Some(true) => args.push("--zellij-images".into()),
                Some(false) => args.push("--no-zellij-images".into()),
                None => errs.push(format!("{}: {what}.zellij_images must be true or false", at(ik))),
            }
            continue;
        }
        if ik == "enabled" {
            match iv.as_bool() {
                Some(true) => args.push("--icons".into()),
                Some(false) => args.push("--no-icons".into()),
                None => errs.push(format!("{}: {what}.enabled must be true or false", at(ik))),
            }
            continue;
        }
        let spec = match iv {
            toml::Value::String(g) if !g.is_empty() => Some(g.clone()),
            toml::Value::Array(a) => match a.iter().map(|x| x.as_str()).collect::<Option<Vec<_>>>()
            {
                Some(p) if p.len() == 2 && !p[0].is_empty() => Some(format!("{},{}", p[0], p[1])),
                _ => None,
            },
            _ => None,
        };
        match spec {
            Some(s) => args.push(format!("--icon={ik}={s}")),
            None => errs.push(format!(
                "{}: {what}.{ik} must be \"icon\" or [\"icon\", \"color\"]",
                at(ik)
            )),
        }
    }
}

/// `[show]`: PART = true / false (title, counter, search-label, footer,
/// icons, preview-title, separator, scrollbar).
fn look_show(
    v: &toml::Value,
    what: &str,
    args: &mut Vec<String>,
    errs: &mut Vec<String>,
    at: &dyn Fn(&str) -> String,
) {
    let Some(t) = v.as_table() else {
        errs.push(format!("{}: [{what}] must be a table", at(what)));
        return;
    };
    for (part, pv) in t {
        let key = part.replace('_', "-");
        if !crate::options::PARTS.contains(&key.as_str()) {
            errs.push(format!(
                "{}: unknown part `{what}.{part}` ({})",
                at(part),
                crate::options::PARTS.join(", ")
            ));
            continue;
        }
        match pv.as_bool() {
            Some(true) => args.push(format!("--show={key}")),
            Some(false) => args.push(format!("--hide={key}")),
            None => errs.push(format!("{}: {what}.{part} must be true or false", at(part))),
        }
    }
}

/// Where the config is read from (None = no config file in use).
pub fn path(explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return if p.is_empty() {
            None
        } else {
            Some(PathBuf::from(p))
        };
    }
    if let Ok(p) = std::env::var("LVIM_FUZZY_CONFIG")
        && !p.is_empty()
    {
        return Some(PathBuf::from(p));
    }
    crate::lvim::theme_dirs()
        .first()
        .and_then(|d| d.parent().map(|c| c.join("config.toml")))
}

/// The 1-based line where `needle` first appears (for error messages).
fn line_of(text: &str, needle: &str) -> Option<usize> {
    text.lines().position(|l| l.contains(needle)).map(|i| i + 1)
}

/// Load and validate. A missing default file is fine; a missing explicit one
/// is an error. Every problem is collected.
pub fn load(explicit: Option<&str>) -> Result<Config, Vec<String>> {
    let Some(path) = path(explicit) else {
        return Ok(Config {
            path: None,
            args: Vec::new(),
            modes: Vec::new(),
            previewers: Vec::new(),
            plugins: toml::Table::new(),
            shell: Default::default(),
        });
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && explicit.is_none() => {
            return Ok(Config {
                path: None,
                args: Vec::new(),
                modes: Vec::new(),
                previewers: Vec::new(),
                plugins: toml::Table::new(),
                shell: Default::default(),
            });
        }
        Err(e) => return Err(vec![format!("{}: {e}", path.display())]),
    };
    let file = path.display().to_string();
    let doc: toml::Table = text
        .parse()
        .map_err(|e: toml::de::Error| vec![format!("{file}: {}", e.to_string().trim())])?;
    let mut errs: Vec<String> = Vec::new();
    let mut pre_errs: Vec<String> = Vec::new();
    let mut args: Vec<String> = Vec::new();
    let mut modes_out: Vec<Mode> = Vec::new();
    let mut shell_cfg = crate::shell::ShellCfg::default();
    // previewers first: `[preview] order` may name them
    let mut previewers: Vec<crate::previewers::Previewer> = Vec::new();
    if let Some(v) = doc.get("previewers") {
        match v.as_table() {
            None => pre_errs.push(format!(
                "{file}: [previewers] must be a table of previewers"
            )),
            Some(t) => {
                for (name, pv) in t {
                    let where_ = match line_of(&text, &format!("previewers.{name}")) {
                        Some(n) => format!("{file}:{n}"),
                        None => file.clone(),
                    };
                    match crate::previewers::parse(name, pv, &where_) {
                        Ok(p) => previewers.push(p),
                        Err(e) => pre_errs.extend(e),
                    }
                }
            }
        }
    }
    let at = |key: &str| match line_of(&text, key) {
        Some(n) => format!("{file}:{n}"),
        None => file.clone(),
    };

    let string =
        |errs: &mut Vec<String>, v: &toml::Value, what: &str, key: &str| -> Option<String> {
            match v.as_str() {
                Some(s) => Some(s.to_string()),
                None => {
                    errs.push(format!("{}: `{what}` must be a string", at(key)));
                    None
                }
            }
        };
    let strings =
        |errs: &mut Vec<String>, v: &toml::Value, what: &str, key: &str| -> Option<Vec<String>> {
            match v.as_array() {
                Some(a) if a.iter().all(|x| x.is_str()) => {
                    Some(a.iter().map(|x| x.as_str().unwrap().to_string()).collect())
                }
                _ => {
                    errs.push(format!("{}: `{what}` must be a list of strings", at(key)));
                    None
                }
            }
        };

    for (k, v) in &doc {
        match k.as_str() {
            "editor" => {
                if let Some(e) = string(&mut errs, v, "editor", "editor") {
                    args.push(format!("--editor={e}"));
                }
            }
            "theme" => {
                if let Some(t) = string(&mut errs, v, "theme", "theme") {
                    // a missing theme file is not an error: the finder falls back to the
                    // terminal's colours (`lvim-fuzzy doctor` reports it)
                    args.push(format!("--theme={t}"));
                }
            }
            "bar" | "title_pos" => {
                if let Some(s) = string(&mut errs, v, k, k) {
                    let ok = match k.as_str() {
                        "bar" => matches!(s.as_str(), "top" | "bottom"),
                        _ => matches!(s.as_str(), "left" | "center" | "right"),
                    };
                    if ok {
                        args.push(format!("--{}={s}", k.replace('_', "-")));
                    } else {
                        let allowed = if k == "bar" { "top, bottom" } else { "left, center, right" };
                        errs.push(format!("{}: invalid `{k}` \"{s}\" ({allowed})", at(k)));
                    }
                }
            }
            "options" => {
                if let Some(list) = strings(&mut errs, v, "options", "options") {
                    match crate::options::Options::parse(&list) {
                        Ok(_) => args.extend(list),
                        Err(e) => errs.push(format!("{}: options: {e}", at("options"))),
                    }
                }
            }
            "preview" => match v.as_table() {
                None => errs.push(format!("{}: [preview] must be a table", at("preview"))),
                Some(t) => {
                    for (pk, pv) in t {
                        match pk.as_str() {
                            "order" => {
                                if let Some(list) = strings(&mut errs, pv, "preview.order", "order") {
                                    let bad: Vec<&String> = list
                                        .iter()
                                        .filter(|x| {
                                            !matches!(x.as_str(), "plugin" | "nvim" | "file" | "command")
                                                && !previewers.iter().any(|p| &p.name == *x)
                                        })
                                        .collect();
                                    if bad.is_empty() {
                                        args.push(format!("--preview-order={}", list.join(",")));
                                    } else {
                                        errs.push(format!(
                                            "{}: unknown preview provider(s) {bad:?} (nvim, file, command, or a [previewers.<name>])",
                                            at("order")
                                        ));
                                    }
                                }
                            }
                            "nvim" => {
                                if let Some(s) = string(&mut errs, pv, "preview.nvim", "nvim") {
                                    args.push(format!("--nvim={s}"));
                                }
                            }
                            "nvim_appname" => {
                                if let Some(s) = string(&mut errs, pv, "preview.nvim_appname", "nvim_appname") {
                                    args.push(format!("--nvim-appname={s}"));
                                }
                            }
                            other => errs.push(format!(
                                "{}: unknown key `preview.{other}` (order, nvim, nvim_appname)",
                                at(other)
                            )),
                        }
                    }
                }
            },
            "keys" => match v.as_table() {
                None => errs.push(format!("{}: [keys] must be a table", at("keys"))),
                Some(t) => {
                    for (key, act) in t {
                        let Some(a) = act.as_str() else {
                            errs.push(format!("{}: keys.\"{key}\" must be a string (an action)", at(key)));
                            continue;
                        };
                        let spec = format!("{key}:{a}");
                        match crate::keys::parse_binds(&spec) {
                            Ok(_) => args.push(format!("--bind={spec}")),
                            Err(e) => errs.push(format!("{}: keys.\"{key}\": {e}", at(key))),
                        }
                    }
                }
            },
            "modes" => match v.as_table() {
                None => errs.push(format!("{}: [modes] must be a table of modes", at("modes"))),
                Some(t) => {
                    let builtin: Vec<String> = modes::builtin().into_iter().map(|m| m.name).collect();
                    for (name, mv) in t {
                        let where_ = at(&format!("modes.{name}"));
                        if builtin.contains(name) {
                            errs.push(format!(
                                "{where_}: user mode `{name}` has the name of a built-in mode — built-in modes \
                                 cannot be replaced; rename it (e.g. `my-{name}`)"
                            ));
                            continue;
                        }
                        if modes::RESERVED.contains(&name.as_str()) {
                            errs.push(format!("{where_}: `{name}` is a subcommand, not a valid mode name"));
                            continue;
                        }
                        if name.is_empty()
                            || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
                        {
                            errs.push(format!(
                                "{where_}: mode name `{name}` may use only a-z, 0-9, `-` and `_`"
                            ));
                            continue;
                        }
                        match parse_mode(name, mv, &where_) {
                            Ok(m) => modes_out.push(m),
                            Err(e) => errs.extend(e),
                        }
                    }
                }
            },
            "popup" => match v.as_table() {
                None => errs.push(format!("{}: [popup] must be a table", at("popup"))),
                Some(t) => {
                    for (pk, pv) in t {
                        match pk.as_str() {
                            // a style name, or 8 strings in Neovim's order
                            "border" => {
                                let spec = match pv {
                                    toml::Value::String(s) => Some(s.clone()),
                                    toml::Value::Array(a) => {
                                        let parts: Option<Vec<String>> =
                                            a.iter().map(|x| x.as_str().map(str::to_string)).collect();
                                        match parts {
                                            Some(p) if p.iter().any(|s| s.contains(',')) => {
                                                errs.push(format!("{}: popup.border: an element cannot be a comma", at("border")));
                                                None
                                            }
                                            Some(p) => Some(p.join(",")),
                                            None => {
                                                errs.push(format!("{}: popup.border: the elements must be strings", at("border")));
                                                None
                                            }
                                        }
                                    }
                                    _ => {
                                        errs.push(format!(
                                            "{}: popup.border must be a style name or an array of 8 strings",
                                            at("border")
                                        ));
                                        None
                                    }
                                };
                                if let Some(spec) = spec {
                                    match crate::options::parse_popup_border(&spec) {
                                        Ok(_) => args.push(format!("--popup-border={spec}")),
                                        Err(e) => errs.push(format!("{}: popup.border: {e}", at("border"))),
                                    }
                                }
                            }
                            other => errs.push(format!(
                                "{}: unknown key `popup.{other}` (border)",
                                at(other)
                            )),
                        }
                    }
                }
            },
            // the looks' texts, file icons and parts shown (for every look)
            "text" => look_text(v, "text", &mut args, &mut errs, &at),
            "icons" => look_icons(v, "icons", &mut args, &mut errs, &at),
            "show" => look_show(v, "show", &mut args, &mut errs, &at),
            "previewers" => {} // read above
            "plugins" => {
                if !v.is_table() || v.as_table().is_some_and(|t| t.values().any(|x| !x.is_table())) {
                    errs.push(format!("{}: [plugins] must be a table of tables: [plugins.<plugin name>]", at("plugins")));
                }
                // a plugin is a program of its own now, installed by its `scripts/install.sh`: the old `install = …` is a mistake
                if let Some(t) = v.as_table() {
                    for (name, pv) in t {
                        if pv.as_table().is_some_and(|p| p.contains_key("install")) {
                            errs.push(format!("{}: [plugins.{name}] install is gone: a plugin is a program of its own — install it with its `scripts/install.sh` and delete this line", at("plugins")));
                        }
                    }
                }
            }
            "pool" => match v.as_table() {
                None => errs.push(format!("{}: [pool] must be a table", at("pool"))),
                Some(t) => {
                    for (pk, pv) in t {
                        match pk.as_str() {
                            "enabled" => match pv.as_bool() {
                                Some(on) => args.push(if on { "--pool" } else { "--no-pool" }.to_string()),
                                None => errs.push(format!("{}: pool.enabled must be true or false", at("enabled"))),
                            },
                            other => errs.push(format!("{}: unknown key `pool.{other}` (enabled)", at(other))),
                        }
                    }
                }
            },
            "shell" => {
                let (c, e) = crate::shell::parse(v, &|k: &str| at(k));
                shell_cfg = c;
                errs.extend(e);
            }
            other => errs.push(format!(
                "{}: unknown key `{other}` (theme, editor, shell, bar, title_pos, options, popup, text, icons, preview, pool, previewers, keys, modes)",
                at(other)
            )),
        }
    }
    // shell keys name modes: built-in, user or shell-history
    for b in &shell_cfg.binds {
        let known = b.mode == "shell-history"
            || modes::builtin().iter().any(|m| m.name == b.mode)
            || modes_out.iter().any(|m| m.name == b.mode);
        if !known {
            errs.push(format!("{}: shell.keys.\"{}\": unknown mode `{}` (`lvim-fuzzy-cli mode` lists them; or shell-history)", at(&b.key), b.key, b.mode));
        }
    }
    errs.splice(0..0, pre_errs);
    if errs.is_empty() {
        Ok(Config {
            path: Some(path),
            args,
            modes: modes_out,
            previewers,
            plugins: doc.get("plugins").and_then(|v| v.as_table()).cloned().unwrap_or_default(),
            shell: shell_cfg,
        })
    } else {
        Err(errs)
    }
}

fn parse_mode(name: &str, v: &toml::Value, at: &str) -> Result<Mode, Vec<String>> {
    let mut errs = Vec::new();
    let Some(t) = v.as_table() else {
        return Err(vec![format!("{at}: [modes.{name}] must be a table")]);
    };
    let mut m = Mode {
        name: name.to_string(),
        title: name.to_string(),
        ..Mode::default()
    };
    for (k, val) in t {
        let s = || val.as_str().map(str::to_string);
        let list = || -> Option<Vec<String>> {
            val.as_array()
                .filter(|a| a.iter().all(|x| x.is_str()))
                .map(|a| a.iter().map(|x| x.as_str().unwrap().to_string()).collect())
        };
        let bad = |what: &str| format!("{at}: modes.{name}.{k} must be {what}");
        match k.as_str() {
            "description" => match s() {
                Some(x) => m.description = x,
                None => errs.push(bad("a string")),
            },
            "title" => match s() {
                Some(x) => m.title = x,
                None => errs.push(bad("a string")),
            },
            "command" => match s() {
                Some(x) if !x.trim().is_empty() => m.command = Some(x),
                _ => errs.push(bad("a non-empty string")),
            },
            "preview" => match s() {
                Some(x) => m.preview = Some(x),
                None => errs.push(bad("a string")),
            },
            "target" => match s() {
                Some(x) => m.target = Some(x),
                None => errs.push(bad("a string")),
            },
            "open" => match s() {
                Some(x) if !x.trim().is_empty() => {
                    let a = crate::modes::open_action(&x);
                    match crate::keys::parse_binds(&format!("x:{a}")) {
                        Ok(_) => m.open = Some(x),
                        Err(e) => errs.push(format!("{at}: mode `{name}`: open: {e}")),
                    }
                }
                _ => errs.push(bad("an action (become(...), execute(...), open, accept) or a command")),
            },
            "scope" => match s() {
                Some(x) if crate::engine::Scope::parse(&x).is_some() => m.scope = Some(x),
                _ => errs.push(bad("one of \"line\", \"path\", \"file\", \"all\"")),
            },
            "live" => match val.as_bool() {
                Some(b) => m.live = b,
                None => errs.push(bad("true or false")),
            },
            "pattern" => match val.as_bool() {
                Some(b) => m.pattern = b,
                None => errs.push(bad("true or false")),
            },
            "requires" => match list() {
                Some(x) => m.requires = x,
                None => errs.push(bad("a list of program names")),
            },
            "options" => match list() {
                Some(x) => m.options = x,
                None => errs.push(bad("a list of strings")),
            },
            other => errs.push(format!(
                "{at}: unknown key `modes.{name}.{other}` (description, title, command, live, pattern, preview, \
                 target, scope, open, requires, options)"
            )),
        }
    }
    if m.command.is_none() {
        errs.push(format!("{at}: mode `{name}` needs a `command`"));
    }
    if m.live && m.pattern {
        errs.push(format!(
            "{at}: mode `{name}`: `live` and `pattern` exclude each other"
        ));
    }
    if errs.is_empty()
        && let Err(e) = crate::options::Options::parse(&m.args(""))
    {
        errs.push(format!("{at}: mode `{name}`: {e}"));
    }
    if errs.is_empty() { Ok(m) } else { Err(errs) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_text(t: &str) -> Result<Config, Vec<String>> {
        // a name of its own per call: the tests run in parallel
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("lvf-cfg-{}-{n}.toml", std::process::id()));
        std::fs::write(&p, t).unwrap();
        let r = load(Some(p.to_str().unwrap()));
        std::fs::remove_file(&p).ok();
        r
    }

    #[test]
    fn a_valid_config() {
        let c = load_text(
            r#"
bar = "top"
options = ["--cycle"]
[preview]
order = ["file", "command"]
[keys]
"ctrl-o" = "execute(less {})"
[modes.todo]
description = "TODO comments"
command = "rg -n TODO"
target = "{1}:{2}"
scope = "line"
"#,
        )
        .unwrap();
        assert!(c.args.contains(&"--bar=top".to_string()));
        assert!(c.args.contains(&"--preview-order=file,command".to_string()));
        assert!(
            c.args
                .contains(&"--bind=ctrl-o:execute(less {})".to_string())
        );
        assert_eq!(c.modes.len(), 1);
        assert_eq!(c.modes[0].name, "todo");
    }

    #[test]
    fn text_and_icons() {
        let c = load_text(
            "[text]\nsearch = \"Find\"\nno_matches = \"nothing\"\nmarker = \"*\"\n[icons]\nenabled = true\nrs = [\"R\", \"red\"]\ndir = \"D\"\n",
        )
        .unwrap();
        let o = crate::options::Options::parse(&c.args).unwrap();
        assert_eq!(o.prompt, "Find");
        assert_eq!(o.empty_text, "nothing");
        assert_eq!(o.marker.as_deref(), Some("*"));
        assert!(o.icons);
        assert_eq!(
            crate::lvim::icon_for(&o, "a/b.rs", false),
            Some(("R", "red"))
        );
        assert_eq!(
            crate::lvim::icon_for(&o, "a/b.lua", false),
            Some((crate::lvim::icon("a/b.lua", false).0, "blue"))
        );
        assert_eq!(
            crate::lvim::icon_for(&o, "a/src", true).map(|i| i.0),
            Some("D")
        );
        let off = load_text("[icons]\nenabled = false\n").unwrap();
        let o = crate::options::Options::parse(&off.args).unwrap();
        assert_eq!(crate::lvim::icon_for(&o, "a/b.rs", false), None);
        let e = load_text("[text]\ncolor = \"x\"\n[icons]\nrs = 3\n")
            .unwrap_err()
            .join("\n");
        assert!(
            e.contains("unknown key `text.color`") && e.contains("icons.rs"),
            "{e}"
        );
    }

    #[test]
    fn texts_icons_and_parts_shown() {
        let c = load_text(concat!(
            "[text]\nsearch = \"Търси\"\n",
            "[show]\ncounter = false\nsearch_label = false\n",
            "[icons]\nrs = \"R\"\n",
        ))
        .unwrap();
        let o = crate::options::Options::parse(&c.args).unwrap();
        assert_eq!(o.prompt, "Търси");
        assert!(o.hidden.contains("counter") && o.hidden.contains("search-label"));
        assert_eq!(
            crate::lvim::icon_for(&o, "a.rs", false).map(|i| i.0),
            Some("R")
        );
        let e = load_text("style = \"full\"\n[show]\nscrollbar = false\n")
            .unwrap_err()
            .join("\n");
        assert!(e.contains("style") && e.contains("show.scrollbar"), "{e}");
    }

    #[test]
    fn every_built_in_icon_is_a_glyph() {
        // the table stays sorted (it is searched by halves) and lookups work
        for t in [crate::icons::FILENAMES, crate::icons::EXTENSIONS] {
            assert!(t.windows(2).all(|w| w[0].0 < w[1].0));
        }
        assert_eq!(crate::lvim::icon("x/Cargo.toml", false).1, "orange");
        assert_eq!(
            crate::lvim::icon("x/foo.test.tsx", false),
            crate::lvim::icon("y.test.tsx", false)
        );
        let names = [
            "Makefile",
            "Dockerfile",
            "Cargo.toml",
            "LICENSE",
            ".gitignore",
            "a.lua",
            "a.rs",
            "a.go",
            "a.py",
            "a.js",
            "a.ts",
            "a.tsx",
            "a.c",
            "a.cpp",
            "a.java",
            "a.rb",
            "a.php",
            "a.sh",
            "a.vim",
            "a.md",
            "a.txt",
            "a.json",
            "a.toml",
            "a.yaml",
            "a.html",
            "a.css",
            "a.png",
            "a.pdf",
            "a.zip",
            "a.lock",
            "a.sql",
            "a.nix",
            "a.conf",
            "a.xml",
            "a.vue",
            "a.zig",
            "a.hs",
            "a.ex",
            "a.kt",
            "a.swift",
            "a.dart",
            "a.tex",
            "a.log",
            "a.unknown",
        ];
        for n in names {
            let (g, _) = crate::lvim::icon(n, false);
            assert_eq!(g.chars().count(), 1, "{n}: {g:?}");
        }
        assert_eq!(crate::lvim::icon("d", true).0.chars().count(), 1);
    }

    #[test]
    fn popup_border() {
        let c = load_text(
            "[popup]\nborder = [\"╭\", \"─\", \"╮\", \"│\", \"╯\", \"─\", \"╰\", \"│\"]\n",
        )
        .unwrap();
        let arg = c
            .args
            .iter()
            .find(|a| a.starts_with("--popup-border="))
            .unwrap();
        let o = crate::options::Options::parse(&[arg.clone()]).unwrap();
        assert_eq!(o.popup_border[0], "╭");
        assert_eq!(o.popup_border[7], "│");
        let c = load_text("[popup]\nborder = \"none\"\n").unwrap();
        assert!(c.args.contains(&"--popup-border=none".to_string()));
        let e = load_text("[popup]\nborder = [\"a\", \"b\"]\nshadow = 1\n")
            .unwrap_err()
            .join("\n");
        assert!(
            e.contains("popup.border") && e.contains("unknown key `popup.shadow`"),
            "{e}"
        );
        // the default: eight spaces
        assert_eq!(
            crate::options::Options::default().popup_border,
            std::array::from_fn(|_| " ".to_string())
        );
    }

    #[test]
    fn every_problem_is_reported() {
        let e = load_text(
            r#"
bar = "left"
colour = 1
[modes.grep]
command = "x"
[modes.Bad]
command = "x"
[modes.nocmd]
title = "x"
[keys]
"hyper-x" = "up"
"#,
        )
        .unwrap_err();
        let all = e.join("\n");
        assert!(all.contains("invalid `bar`"), "{all}");
        assert!(all.contains("unknown key `colour`"), "{all}");
        assert!(all.contains("built-in mode"), "{all}");
        assert!(all.contains("may use only"), "{all}");
        assert!(all.contains("needs a `command`"), "{all}");
        assert!(all.contains("hyper-x"), "{all}");
        // file:line locations
        assert!(all.contains(".toml:4"), "{all}");
    }
}
