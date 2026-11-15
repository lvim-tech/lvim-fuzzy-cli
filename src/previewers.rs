//! User previewers: named preview commands from the config, used in the
//! preview chain beside the built-in ones (`nvim`, `file`, `command`).
//!
//! ```toml
//! [previewers.bat]
//! command  = "bat --color=always --style=numbers --highlight-line {line} {file}"
//! for      = "file"        # file | dir | any
//! requires = ["bat"]
//!
//! [previewers.glow]
//! command    = "glow -s dark {file}"
//! extensions = ["md"]
//!
//! [preview]
//! order = ["glow", "bat", "nvim", "file", "command"]
//! ```
//!
//! `{file}`, `{line}` and `{col}` come from the item's target (`--target`,
//! e.g. `{1}:{2}:{3}` for grep results); every fzf placeholder works as well.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum For {
    File,
    Dir,
    Any,
}

#[derive(Clone, Debug)]
pub struct Previewer {
    pub name: String,
    pub command: String,
    pub for_: For,
    pub extensions: Vec<String>,
    pub requires: Vec<String>,
}

/// The built-in provider names a previewer may not take.
pub const BUILTIN: &[&str] = &["nvim", "neovim", "file", "builtin", "command", "cmd"];

impl Previewer {
    /// Whether this previewer can show `path` (an existing file or directory,
    /// or "" when the item names none).
    pub fn applies(&self, path: &str) -> bool {
        if self.requires.iter().any(|r| !crate::modes::on_path(r)) {
            return false;
        }
        let p = std::path::Path::new(path);
        let ok = match self.for_ {
            For::Any => true,
            For::File => !path.is_empty() && p.is_file(),
            For::Dir => !path.is_empty() && p.is_dir(),
        };
        if !ok {
            return false;
        }
        if self.extensions.is_empty() {
            return true;
        }
        let ext = p
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        self.extensions
            .iter()
            .any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(&ext))
    }

    /// The command with `{file}` / `{line}` / `{col}` filled in (shell-quoted);
    /// the other placeholders are left for the usual expansion.
    pub fn command_for(&self, file: &str, line: usize, col: usize) -> String {
        self.command
            .replace("{file}", &crate::placeholder::quote(file))
            .replace("{line}", &line.max(1).to_string())
            .replace("{col}", &col.max(1).to_string())
    }
}

/// Parse `[previewers.<name>]`; errors carry `at` (file:line).
pub fn parse(name: &str, v: &toml::Value, at: &str) -> Result<Previewer, Vec<String>> {
    let mut errs = Vec::new();
    if BUILTIN.contains(&name) {
        return Err(vec![format!(
            "{at}: previewer `{name}` has the name of a built-in preview — rename it (e.g. `my-{name}`)"
        )]);
    }
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(vec![format!(
            "{at}: previewer name `{name}` may use only a-z, 0-9, `-` and `_`"
        )]);
    }
    let Some(t) = v.as_table() else {
        return Err(vec![format!("{at}: [previewers.{name}] must be a table")]);
    };
    let mut p = Previewer {
        name: name.into(),
        command: String::new(),
        for_: For::File,
        extensions: vec![],
        requires: vec![],
    };
    let list = |v: &toml::Value| -> Option<Vec<String>> {
        v.as_array()
            .filter(|a| a.iter().all(|x| x.is_str()))
            .map(|a| a.iter().map(|x| x.as_str().unwrap().to_string()).collect())
    };
    for (k, val) in t {
        match k.as_str() {
            "command" => match val.as_str() {
                Some(c) if !c.trim().is_empty() => p.command = c.to_string(),
                _ => errs.push(format!(
                    "{at}: previewers.{name}.command must be a non-empty string"
                )),
            },
            "for" => match val.as_str() {
                Some("file") => p.for_ = For::File,
                Some("dir") => p.for_ = For::Dir,
                Some("any") => p.for_ = For::Any,
                _ => errs.push(format!(
                    "{at}: previewers.{name}.for must be \"file\", \"dir\" or \"any\""
                )),
            },
            "extensions" => match list(val) {
                Some(x) => p.extensions = x,
                None => errs.push(format!(
                    "{at}: previewers.{name}.extensions must be a list of strings"
                )),
            },
            "requires" => match list(val) {
                Some(x) => p.requires = x,
                None => errs.push(format!(
                    "{at}: previewers.{name}.requires must be a list of program names"
                )),
            },
            other => errs.push(format!(
                "{at}: unknown key `previewers.{name}.{other}` (command, for, extensions, requires)"
            )),
        }
    }
    if p.command.is_empty() && errs.is_empty() {
        errs.push(format!("{at}: previewer `{name}` needs a `command`"));
    }
    if errs.is_empty() { Ok(p) } else { Err(errs) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_and_fills_the_target() {
        let p = Previewer {
            name: "md".into(),
            command: "glow {file} +{line}".into(),
            for_: For::File,
            extensions: vec!["toml".into()],
            requires: vec![],
        };
        assert!(p.applies("Cargo.toml"));
        assert!(!p.applies("src")); // a directory
        assert!(!p.applies("src/main.rs")); // another extension
        assert_eq!(p.command_for("a b.toml", 3, 0), "glow 'a b.toml' +3");
        let any = Previewer {
            for_: For::Any,
            extensions: vec![],
            ..p.clone()
        };
        assert!(any.applies(""));
        let missing = Previewer {
            requires: vec!["no-such-program-xyz".into()],
            ..any
        };
        assert!(!missing.applies(""));
    }
}
