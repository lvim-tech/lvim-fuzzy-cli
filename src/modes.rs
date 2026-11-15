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

fn m(name: &str, description: &str, title: &str) -> Mode {
    Mode {
        name: name.into(),
        description: description.into(),
        title: title.into(),
        builtin: true,
        ..Mode::default()
    }
}

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

fn s(v: &str) -> Option<String> {
    Some(v.to_string())
}

fn v(list: &[&str]) -> Vec<String> {
    list.iter().map(|x| x.to_string()).collect()
}

/// The shell history file to read: $HISTFILE, then zsh, bash, fish.
fn history_command() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let hist = std::env::var("HISTFILE").ok().filter(|h| !h.is_empty());
    let zsh = format!("{home}/.zsh_history");
    let bash = format!("{home}/.bash_history");
    let fish = format!("{home}/.local/share/fish/fish_history");
    let pick = hist.or_else(|| {
        [&zsh, &bash, &fish]
            .into_iter()
            .find(|p| std::path::Path::new(p).is_file())
            .cloned()
    });
    match pick {
        Some(f) if f.ends_with("fish_history") => {
            format!(
                "sed -n 's/^- cmd: //p' {} | tac | awk '!seen[$0]++'",
                crate::placeholder::quote(&f)
            )
        }
        // zsh extended history lines start with ": <time>:<elapsed>;"
        Some(f) => format!(
            "sed -E 's/^: [0-9]+:[0-9]+;//' {} | tac | awk '!seen[$0]++'",
            crate::placeholder::quote(&f)
        ),
        None => "true".into(),
    }
}

/// Every built-in mode.
pub fn builtin() -> Vec<Mode> {
    let rg = "rg --column --line-number --no-heading --color=always --smart-case";
    let mut v_ = Vec::new();
    let mut add = |md: Mode| v_.push(md);

    add(Mode {
        target: s("{}"),
        options: v(&["--scheme=path"]),
        ..m("files", "files under the current directory", "Files")
    });
    add(Mode {
        open: s(r#"accept"#),
        target: s("{}"),
        options: v(&["--walker=dir,follow,hidden", "--scheme=path"]),
        ..m(
            "dirs",
            "directories under the current directory",
            "Directories",
        )
    });
    add(Mode {
        // nothing to search until a character is typed (rg would list every line)
        command: Some(format!("[ -n {{q}} ] && {rg} -- {{q}} || true")),
        live: true,
        target: s("{1}:{2}:{3}"),
        requires: v(&["rg"]),
        options: v(&["--ansi", "--delimiter=:"]),
        ..m("grep", "search file contents as you type (ripgrep)", "Grep")
    });
    add(Mode {
        command: Some(format!("{rg} -- {{q}} || true")),
        pattern: true,
        target: s("{1}:{2}:{3}"),
        scope: s("line"),
        requires: v(&["rg"]),
        options: v(&["--ansi", "--delimiter=:"]),
        ..m(
            "grep-filter",
            "grep once for the query, then filter the results (line/path/file)",
            "Grep ➤ Filter",
        )
    });
    add(Mode {
        command: s("git ls-files --cached --others --exclude-standard"),
        target: s("{}"),
        requires: v(&["git"]),
        options: v(&["--scheme=path"]),
        ..m("git-files", "files tracked by git", "Git files")
    });
    add(Mode {
        open: s(r#"open"#),
        command: s("git -c color.status=always status --short --untracked-files=all"),
        preview: s(
            "git diff --color=always -- {-1} | head -1000; git diff --cached --color=always -- {-1} | head -1000",
        ),
        requires: v(&["git"]),
        options: v(&["--ansi", "--no-sort"]),
        ..m("git-status", "changed files, with their diff", "Git status")
    });
    add(Mode {
        open: s(r#"execute(git show --color=always {1} | less -R)"#),
        command: s("git log --oneline --decorate --color=always"),
        preview: s("git show --stat --patch --color=always {1}"),
        requires: v(&["git"]),
        options: v(&["--ansi", "--no-sort"]),
        ..m("git-log", "commits, with their changes", "Git log")
    });
    add(Mode {
        open: s(r#"become(b={-1}; git switch "${b#remotes/*/}")"#),
        command: s("git branch --all --color=always --sort=-committerdate"),
        preview: s("git log --oneline --graph --decorate --color=always -n 80 {-1}"),
        requires: v(&["git"]),
        options: v(&["--ansi", "--no-sort"]),
        ..m(
            "git-branches",
            "local and remote branches, with their history",
            "Git branches",
        )
    });
    add(Mode {
        open: s(r#"execute(git stash show -p --color=always {1} | less -R)"#),
        command: s("git stash list"),
        preview: s("git stash show --patch --color=always {1}"),
        requires: v(&["git"]),
        options: v(&["--delimiter=:", "--no-sort"]),
        ..m("git-stash", "stashes, with their changes", "Git stash")
    });
    add(Mode {
        open: s(r#"execute(git show --color=always {1} | less -R)"#),
        command: s("git tag --sort=-creatordate"),
        preview: s("git show --stat --color=always {}"),
        requires: v(&["git"]),
        options: v(&["--no-sort"]),
        ..m("git-tags", "tags, newest first", "Git tags")
    });
    add(Mode {
        open: s(r#"execute(git show --color=always {1} | less -R)"#),
        command: s("git reflog --color=always"),
        preview: s("git show --stat --color=always {1}"),
        requires: v(&["git"]),
        options: v(&["--ansi", "--no-sort"]),
        ..m("git-reflog", "where HEAD has been", "Git reflog")
    });
    add(Mode {
        command: Some(history_command()),
        options: v(&["--scheme=history", "--no-sort"]),
        ..m("history", "shell command history, newest first", "History")
    });
    add(Mode {
        command: s("ps -eo pid,user,%cpu,%mem,etime,args --sort=-%cpu"),
        preview: s("ps -o pid,ppid,user,%cpu,%mem,rss,etime,stat,args -p {1}"),
        requires: v(&["ps"]),
        options: v(&["--header-lines=1", "--no-sort"]),
        ..m("processes", "running processes, busiest first", "Processes")
    });
    add(Mode {
        command: s("ss -tulpnH 2>/dev/null"),
        requires: v(&["ss"]),
        options: v(&["--no-sort"]),
        ..m("ports", "listening TCP/UDP ports", "Ports")
    });
    add(Mode {
        open: s(r#"become(man {1})"#),
        command: s("man -k . 2>/dev/null"),
        preview: s("MANWIDTH=$LVIM_FUZZY_PREVIEW_COLUMNS man {1} 2>/dev/null | col -bx"),
        requires: v(&["man"]),
        ..m("man", "manual pages", "Man pages")
    });
    add(Mode {
        command: s("env | sort"),
        preview: s("printenv {1}"),
        options: v(&["--delimiter=="]),
        ..m("env", "environment variables", "Environment")
    });
    add(Mode {
        command: s(
            "printf '%s\\n' $(echo \"$PATH\" | tr ':' ' ') | xargs -I{} find {} -maxdepth 1 \\( -type f -o -type l \\) -perm -u+x -printf '%f\\n' 2>/dev/null | sort -u",
        ),
        preview: s("command -v {}; whatis {} 2>/dev/null"),
        ..m("commands", "programs on the PATH", "Commands")
    });
    add(Mode {
        open: s(r#"become(tmux switch-client -t {1} 2>/dev/null || tmux attach -t {1})"#),
        command: s(
            "tmux list-sessions -F '#{session_name}: #{session_windows} windows#{?session_attached, (attached),}'",
        ),
        preview: s("tmux list-windows -t {1}"),
        requires: v(&["tmux"]),
        options: v(&["--delimiter=:"]),
        ..m("tmux", "tmux sessions", "Tmux sessions")
    });
    add(Mode {
        open: s(
            r#"become(if [ -n "$ZELLIJ" ]; then zellij action switch-session {1}; else zellij attach {1}; fi)"#,
        ),
        command: s("zellij list-sessions --no-formatting"),
        requires: v(&["zellij"]),
        ..m("zellij", "zellij sessions", "Zellij sessions")
    });
    add(Mode {
        open: s(r#"execute(docker logs --tail 500 {1} 2>&1 | less -R)"#),
        command: s("docker ps -a --format '{{.ID}}\\t{{.Names}}\\t{{.Image}}\\t{{.Status}}'"),
        preview: s("docker inspect {1}"),
        requires: v(&["docker"]),
        options: v(&["--no-sort"]),
        ..m("docker", "docker containers", "Docker containers")
    });
    add(Mode {
        command: s("docker images --format '{{.Repository}}:{{.Tag}}\\t{{.ID}}\\t{{.Size}}'"),
        preview: s("docker history {2}"),
        requires: v(&["docker"]),
        ..m("docker-images", "docker images", "Docker images")
    });
    add(Mode {
        open: s(r#"execute(systemctl status {1} | less -R)"#),
        command: s("systemctl list-units --all --no-pager --plain --no-legend"),
        preview: s("SYSTEMD_COLORS=1 systemctl status --no-pager {1}"),
        requires: v(&["systemctl"]),
        options: v(&["--ansi"]),
        ..m("systemd", "system units", "Systemd units")
    });
    add(Mode {
        open: s(r#"execute(systemctl --user status {1} | less -R)"#),
        command: s("systemctl --user list-units --all --no-pager --plain --no-legend"),
        preview: s("SYSTEMD_COLORS=1 systemctl --user status --no-pager {1}"),
        requires: v(&["systemctl"]),
        options: v(&["--ansi"]),
        ..m("systemd-user", "user units", "Systemd user units")
    });
    add(Mode {
        command: s("journalctl -n 5000 --no-pager -o short-iso"),
        requires: v(&["journalctl"]),
        options: v(&["--tac", "--no-sort"]),
        ..m("journal", "the latest system log entries", "Journal")
    });
    add(Mode {
        open: s(r#"become(ssh {})"#),
        command: s(
            "awk 'tolower($1)==\"host\" {for(i=2;i<=NF;i++) if ($i !~ /[*?]/) print $i}' ~/.ssh/config 2>/dev/null",
        ),
        preview: s("ssh -G {} 2>/dev/null | head -40"),
        requires: v(&["ssh"]),
        ..m("ssh", "hosts from ~/.ssh/config", "SSH hosts")
    });
    add(Mode {
        command: s("fc-list --format '%{family[0]}\\n' | sort -u"),
        requires: v(&["fc-list"]),
        ..m("fonts", "installed font families", "Fonts")
    });
    add(Mode {
        command: s(
            "{ pacman -Qq || dpkg-query -W -f='${Package}\\n' || rpm -qa --qf '%{NAME}\\n'; } 2>/dev/null | sort -u",
        ),
        preview: s("pacman -Qi {} 2>/dev/null || dpkg -s {} 2>/dev/null || rpm -qi {} 2>/dev/null"),
        ..m("packages", "installed packages", "Packages")
    });
    add(Mode {
        open: s(r#"accept"#),
        command: s("zoxide query --list"),
        target: s("{}"),
        requires: v(&["zoxide"]),
        options: v(&["--no-sort", "--scheme=path"]),
        ..m("zoxide", "frequent directories (zoxide)", "Zoxide")
    });
    add(Mode {
        open: s(r#"execute(kubectl -n {1} logs --tail 500 {2} | less -R)"#),
        command: s("kubectl get pods --all-namespaces --no-headers"),
        preview: s("kubectl describe pod -n {1} {2}"),
        requires: v(&["kubectl"]),
        ..m("kube-pods", "Kubernetes pods", "Kubernetes pods")
    });
    add(Mode {
        open: s(r#"execute-silent(gh pr view --web {1})"#),
        command: s("gh pr list --limit 200"),
        preview: s("gh pr view {1}"),
        requires: v(&["gh"]),
        options: v(&["--no-sort"]),
        ..m(
            "gh-prs",
            "GitHub pull requests of this repository",
            "GitHub PRs",
        )
    });
    add(Mode {
        open: s(r#"execute-silent(gh issue view --web {1})"#),
        command: s("gh issue list --limit 200"),
        preview: s("gh issue view {1}"),
        requires: v(&["gh"]),
        options: v(&["--no-sort"]),
        ..m(
            "gh-issues",
            "GitHub issues of this repository",
            "GitHub issues",
        )
    });
    v_
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

    #[test]
    fn open_values() {
        assert_eq!(open_action("execute(less {})"), "execute(less {})");
        assert_eq!(open_action("accept"), "accept");
        assert_eq!(
            open_action("zellij attach {1}"),
            "become(zellij attach {1})"
        );
        let z = builtin().into_iter().find(|m| m.name == "zellij").unwrap();
        assert!(
            z.args("").iter().any(|a| a.starts_with("--open=become(")),
            "{:?}",
            z.args("")
        );
    }

    #[test]
    fn builtins_are_unique_and_parse() {
        let all = builtin();
        let mut names: Vec<&str> = all.iter().map(|m| m.name.as_str()).collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(n, names.len(), "duplicate built-in mode names");
        assert!(n >= 30, "only {n} built-in modes");
        for md in &all {
            assert!(!RESERVED.contains(&md.name.as_str()));
            let args = md.args("x");
            crate::options::Options::parse(&args)
                .unwrap_or_else(|e| panic!("mode {}: {e}", md.name));
        }
    }

    #[test]
    fn pattern_modes_bake_the_query_into_the_command() {
        let g = builtin()
            .into_iter()
            .find(|m| m.name == "grep-filter")
            .unwrap();
        let a = g.args("health");
        assert!(
            a.iter()
                .any(|x| x.starts_with("--command=") && x.contains("'health'"))
        );
        assert!(!a.iter().any(|x| x.starts_with("--query=")));
    }
}
