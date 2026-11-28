//! Search modes: named presets (source command, preview, scope, title, ...).
//!
//! Every mode — built in or defined in the config — starts as a subcommand
//! (`lvim-fuzzy git-log`), from `--mode NAME`, from `lvim-fuzzy mode` (a list
//! of all modes), or from the in-finder mode menu (F2), which switches in place
//! and keeps the query.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mode {
    pub name: String,
    pub description: String,
    pub title: String,
    /// The source command; None = the built-in directory walker.
    pub command: Option<String>,
    /// Re-run the command with the query (`{q}`) on every change (live grep).
    pub live: bool,
    /// Run the command once with the starting query as its pattern, then
    /// filter the results (grep-filter).
    pub pattern: bool,
    /// Preview command (shell, fzf placeholders).
    pub preview: Option<String>,
    /// `file[:line[:col]]` template for the Neovim / built-in file preview.
    pub target: Option<String>,
    pub scope: Option<String>,
    /// Programs the mode needs; a mode whose programs are missing is listed
    /// but not started.
    pub requires: Vec<String>,
    /// Extra options (fzf syntax), applied before the user's own.
    pub options: Vec<String>,
    /// What the `open` action does with an item of this mode (bound to enter by
    /// default, to any key in [keys]): `--bind` action syntax (`become(...)`,
    /// `execute(...)`, `accept`) or a plain command (= `become(command)`).
    /// None = a file in the editor, else print.
    pub open: Option<String>,
    pub builtin: bool,
}

/// Names that are not modes (subcommands of their own).
pub const RESERVED: &[&str] = &["mode", "modes", "config", "doctor", "help", "version"];

/// An `open` value as a --bind action: a plain shell command becomes
/// `become(command)`.
pub fn open_action(e: &str) -> String {
    let e = e.trim();
    let is_action = crate::keys::parse_binds(&format!("x:{e}")).is_ok_and(|b| {
        b.iter().all(|(_, acts)| {
            acts.iter().all(|a| {
                crate::keys::ACTIONS
                    .iter()
                    .any(|n| n.trim_end_matches('(') == a.name)
            })
        })
    });
    if is_action {
        e.to_string()
    } else {
        format!("become({e})")
    }
}

/// The modes the plugin `lvim-fuzzy-plugin-modes` serves (asked once, at the first use), and why there are none when it does not answer.
static BUILTIN: std::sync::OnceLock<(Vec<Mode>, Option<String>)> = std::sync::OnceLock::new();

/// A mode from the plugin's JSON.
fn mode_of(v: &serde_json::Value) -> Option<Mode> {
    let text = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    let list = |k: &str| -> Vec<String> { v.get(k).and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default() };
    let name = text("name")?;
    Some(Mode {
        title: text("title").unwrap_or_else(|| name.clone()),
        description: text("description").unwrap_or_default(),
        command: text("command"),
        live: v.get("live").and_then(|x| x.as_bool()).unwrap_or(false),
        pattern: v.get("pattern").and_then(|x| x.as_bool()).unwrap_or(false),
        preview: text("preview"),
        target: text("target"),
        scope: text("scope"),
        requires: list("requires"),
        options: list("options"),
        open: text("open"),
        builtin: true,
        name,
    })
}

/// In the tests: the plugin's own `plugin.toml` beside this repository stands in for the installed plugin (None: it is not there).
#[cfg(test)]
pub(crate) fn sibling_modes() -> Option<Vec<Mode>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../lvim-fuzzy-plugin-modes/plugin.toml");
    let t: toml::Table = toml::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let cfg = serde_json::to_value(t.get("config")?.clone()).ok()?;
    let mut list = Vec::new();
    for n in cfg["order"].as_array()? {
        let name = n.as_str()?;
        let mut m = cfg["modes"][name].clone();
        m["name"] = name.into();
        list.push(mode_of(&m)?);
    }
    Some(list)
}

#[cfg(test)]
fn load_builtin() -> (Vec<Mode>, Option<String>) {
    (sibling_modes().unwrap_or_default(), None)
}

#[cfg(not(test))]
fn load_builtin() -> (Vec<Mode>, Option<String>) {
    let (wake, _rx) = std::sync::mpsc::channel();
    let mut c = lvim_fuzzy_plugin::Modes::load(&crate::plugin_roots(), "lvim-fuzzy-cli", &toml::Table::new(), wake);
    let answer = c.ask(std::time::Duration::from_secs(5));
    c.stop();
    match answer {
        Ok(list) => (list.iter().filter_map(mode_of).collect(), None),
        Err(e) => (Vec::new(), Some(e)),
    }
}

/// Every mode the plugin serves (the ones that are not the user's own), in its order. None when it is not installed.
pub fn builtin() -> Vec<Mode> {
    BUILTIN.get_or_init(load_builtin).0.clone()
}

/// Why `builtin()` is empty: the plugin is not there, or did not answer.
pub fn builtin_error() -> Option<String> {
    BUILTIN.get_or_init(load_builtin).1.clone()
}

/// Whether `prog` is on the PATH.
pub fn on_path(prog: &str) -> bool {
    if prog.contains('/') {
        return std::path::Path::new(prog).is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(prog).is_file()))
}

impl Mode {
    /// The programs the mode needs that are missing.
    pub fn missing(&self) -> Vec<String> {
        self.requires
            .iter()
            .filter(|r| !on_path(r))
            .cloned()
            .collect()
    }

    /// The options that set the finder up for this mode (fzf syntax).
    /// `query` is the starting query (the grep pattern for `pattern` modes).
    pub fn args(&self, query: &str) -> Vec<String> {
        let mut a = vec![
            format!("--title={}", self.title),
            format!("--mode-name={}", self.name),
        ];
        if let Some(cmd) = &self.command {
            if self.live {
                a.push("--disabled".into());
                a.push(format!("--bind=start:reload:{cmd}"));
                a.push(format!("--bind=change:reload:{cmd}"));
                if !query.is_empty() {
                    a.push(format!("--query={query}"));
                }
            } else if self.pattern {
                // run once with the starting query, then filter from an empty query
                let q = crate::placeholder::quote(query);
                a.push(format!("--command={}", cmd.replace("{q}", &q)));
                // what found the lines: marked in them, as the filter narrows
                if !query.is_empty() {
                    a.push(format!("--grep-pattern={query}"));
                }
            } else {
                a.push(format!("--command={cmd}"));
                if !query.is_empty() {
                    a.push(format!("--query={query}"));
                }
            }
        } else if !query.is_empty() {
            a.push(format!("--query={query}"));
        }
        if let Some(p) = &self.preview {
            a.push(format!("--preview={p}"));
        }
        if let Some(t) = &self.target {
            a.push(format!("--target={t}"));
        }
        if let Some(sc) = &self.scope {
            a.push(format!("--scope={sc}"));
        }
        if let Some(e) = &self.open {
            a.push(format!("--open={}", open_action(e)));
        }
        a.extend(self.options.iter().cloned());
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin_modes() -> Option<Vec<Mode>> {
        super::sibling_modes()
    }

    #[test]
    fn open_values() {
        assert_eq!(open_action("execute(less {})"), "execute(less {})");
        assert_eq!(open_action("accept"), "accept");
        assert_eq!(
            open_action("zellij attach {1}"),
            "become(zellij attach {1})"
        );
        let z = Mode { name: "z".into(), title: "Z".into(), open: Some("zellij attach {1}".into()), ..Mode::default() };
        assert!(
            z.args("").iter().any(|a| a.starts_with("--open=become(")),
            "{:?}",
            z.args("")
        );
    }

    #[test]
    fn a_mode_comes_from_the_plugin_json() {
        let m = mode_of(&serde_json::json!({"name": "x", "title": "X", "command": "echo", "live": true, "requires": ["rg"], "options": ["--ansi"]})).unwrap();
        assert!(m.builtin && m.live && !m.pattern);
        assert_eq!((m.command.as_deref(), m.requires, m.options), (Some("echo"), vec!["rg".to_string()], vec!["--ansi".to_string()]));
        assert!(mode_of(&serde_json::json!({"title": "no name"})).is_none());
    }

    #[test]
    fn the_plugins_modes_are_unique_and_parse() {
        let Some(all) = plugin_modes() else { return };
        let mut names: Vec<&str> = all.iter().map(|m| m.name.as_str()).collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(n, names.len(), "duplicate mode names");
        assert!(n >= 30, "only {n} modes");
        for md in &all {
            assert!(!RESERVED.contains(&md.name.as_str()));
            let args = md.args("x");
            crate::options::Options::parse(&args)
                .unwrap_or_else(|e| panic!("mode {}: {e}", md.name));
        }
    }

    #[test]
    fn pattern_modes_bake_the_query_into_the_command() {
        let Some(all) = plugin_modes() else { return };
        let g = all.into_iter().find(|m| m.name == "grep-filter").unwrap();
        let a = g.args("health");
        assert!(
            a.iter()
                .any(|x| x.starts_with("--command=") && x.contains("'health'"))
        );
        assert!(!a.iter().any(|x| x.starts_with("--query=")));
    }
}
