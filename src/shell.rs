//! Shell integration: `eval "$(lvim-fuzzy --zsh)"` (and --bash, --fish).
//!
//! NOTHING is bound by default: every key comes from the config's `[shell]`.
//!
//! ```toml
//! [shell]
//! completion = "**"            # <Tab> completion trigger (zsh, fish); absent = off
//! height     = "50%"           # below the prompt, outside Zellij / tmux
//! popup      = "center,80%"    # a floating pane in Zellij / tmux
//!
//! [shell.keys]                 # key = "MODE[:insert|cd|replace]"
//! "ctrl-t" = "files"           # insert the chosen items at the cursor (default)
//! "ctrl-r" = "shell-history"   # the shell's own history (replaces the line)
//! "alt-c"  = "dirs:cd"         # cd into the chosen directory
//! "ctrl-g" = "git-status"
//! ```
//!
//! MODE is any built-in or user mode, or `shell-history`. Keys use fzf's names:
//! `ctrl-x`, `alt-x`, `ctrl-alt-x`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Insert,
    Cd,
    Replace,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bind {
    pub key: String,
    pub mode: String,
    pub act: Act,
}

#[derive(Clone, Debug, Default)]
pub struct ShellCfg {
    pub completion: Option<String>,
    pub height: Option<String>,
    pub popup: Option<String>,
    pub binds: Vec<Bind>,
}

/// A key the shells can bind: `ctrl-[a-z]`, `alt-<char>`, `ctrl-alt-[a-z]`.
fn key_ok(k: &str) -> bool {
    let one = |s: &str| s.chars().count() == 1;
    if let Some(c) = k.strip_prefix("ctrl-alt-") {
        return one(c) && c.chars().all(|c| c.is_ascii_lowercase());
    }
    if let Some(c) = k.strip_prefix("ctrl-") {
        return one(c) && c.chars().all(|c| c.is_ascii_lowercase());
    }
    if let Some(c) = k.strip_prefix("alt-") {
        return one(c)
            && c.chars()
                .all(|c| c.is_ascii_graphic() && c != '\'' && c != '"' && c != '\\');
    }
    false
}

/// Parse `[shell]`: what could be read, and every problem (errors carry `at`).
/// Modes are checked by the caller.
pub fn parse(v: &toml::Value, at: &dyn Fn(&str) -> String) -> (ShellCfg, Vec<String>) {
    let mut errs = Vec::new();
    let mut cfg = ShellCfg::default();
    let Some(t) = v.as_table() else {
        return (
            cfg,
            vec![format!("{}: [shell] must be a table", at("shell"))],
        );
    };
    for (k, val) in t {
        match k.as_str() {
            "completion" | "height" | "popup" => match val.as_str() {
                Some(s) => {
                    let s = s.to_string();
                    match k.as_str() {
                        "completion" => cfg.completion = (!s.is_empty()).then_some(s),
                        "height" => cfg.height = Some(s),
                        _ => {
                            if let Err(e) = crate::popup::parse(&s) {
                                errs.push(format!("{}: shell.popup: {e}", at("popup")));
                            }
                            cfg.popup = Some(s);
                        }
                    }
                }
                None => errs.push(format!("{}: shell.{k} must be a string", at(k))),
            },
            "keys" => match val.as_table() {
                None => errs.push(format!(
                    "{}: [shell.keys] must be a table",
                    at("shell.keys")
                )),
                Some(keys) => {
                    for (key, m) in keys {
                        let Some(spec) = m.as_str() else {
                            errs.push(format!("{}: shell.keys.\"{key}\" must be a string (\"MODE[:insert|cd|replace]\")", at(key)));
                            continue;
                        };
                        if !key_ok(key) {
                            errs.push(format!(
                                "{}: shell.keys: \"{key}\" cannot be bound in a shell (ctrl-x, alt-x, ctrl-alt-x)",
                                at(key)
                            ));
                            continue;
                        }
                        let (mode, act) = match spec.split_once(':') {
                            None => (spec, Act::Insert),
                            Some((m, "insert")) => (m, Act::Insert),
                            Some((m, "cd")) => (m, Act::Cd),
                            Some((m, "replace")) => (m, Act::Replace),
                            Some((_, other)) => {
                                errs.push(format!(
                                    "{}: shell.keys.\"{key}\": unknown action `{other}` (insert, cd, replace)",
                                    at(key)
                                ));
                                continue;
                            }
                        };
                        cfg.binds.push(Bind {
                            key: key.clone(),
                            mode: mode.to_string(),
                            act,
                        });
                    }
                }
            },
            other => errs.push(format!(
                "{}: unknown key `shell.{other}` (completion, height, popup, keys)",
                at(other)
            )),
        }
    }
    (cfg, errs)
}

fn zsh_key(k: &str) -> String {
    if let Some(c) = k.strip_prefix("ctrl-alt-") {
        format!("\\e^{}", c.to_ascii_uppercase())
    } else if let Some(c) = k.strip_prefix("ctrl-") {
        format!("^{}", c.to_ascii_uppercase())
    } else {
        format!("\\e{}", k.trim_start_matches("alt-"))
    }
}

fn bash_key(k: &str) -> String {
    if let Some(c) = k.strip_prefix("ctrl-alt-") {
        format!("\\e\\C-{c}")
    } else if let Some(c) = k.strip_prefix("ctrl-") {
        format!("\\C-{c}")
    } else {
        format!("\\e{}", k.trim_start_matches("alt-"))
    }
}

fn fish_key(k: &str) -> String {
    if let Some(c) = k.strip_prefix("ctrl-alt-") {
        format!("\\e\\c{c}")
    } else if let Some(c) = k.strip_prefix("ctrl-") {
        format!("\\c{c}")
    } else {
        format!("\\e{}", k.trim_start_matches("alt-"))
    }
}

fn quote(s: &str) -> String {
    crate::placeholder::quote(s)
}

fn defaults(cfg: &ShellCfg) -> (String, String) {
    (
        cfg.height.clone().unwrap_or_else(|| "50%".into()),
        cfg.popup.clone().unwrap_or_else(|| "center,80%".into()),
    )
}

const ZSH_BASE: &str = r#"
__lvf() {
  local bin=${LVIM_FUZZY_BIN:-lvim-fuzzy}
  "$bin" "$@" --height="${LVIM_FUZZY_HEIGHT:-$__lvf_height}" --popup="${LVIM_FUZZY_POPUP:-$__lvf_popup}"
}
# MODE's choice inserted at the cursor (quoted), cd'd into, or replacing the line
__lvf_insert() {
  local out
  out=$(__lvf "$1" --multi < /dev/tty)
  [[ -n $out ]] && LBUFFER+="${(j: :)${(q-)${(f)out}}} "
  zle reset-prompt
}
__lvf_cd() {
  local dir
  dir=$(__lvf "$1" < /dev/tty)
  if [[ -n $dir ]]; then
    builtin cd -- "$dir" || return
    local f; for f in $precmd_functions; do "$f"; done
  fi
  zle reset-prompt
}
__lvf_replace() {
  local out
  out=$(__lvf "$1" --query="$BUFFER" < /dev/tty)
  [[ -n $out ]] && { BUFFER=$out; CURSOR=${#BUFFER}; }
  zle reset-prompt
}
# the shell's own history, newest first; multi-line entries stay whole
__lvf_shell_history() {
  local out k
  out=$(for k in ${(Onk)history}; do printf '%s\0' "$history[$k]"; done |
    __lvf --read0 --no-sort --scheme=history --title=History --query="$LBUFFER")
  [[ -n $out ]] && { BUFFER=$out; CURSOR=${#BUFFER}; }
  zle reset-prompt
}
"#;

const ZSH_COMPLETION: &str = r#"
# <TRIGGER><Tab>: complete the word before the trigger
lvim-fuzzy-completion() {
  local trig=$__lvf_trigger
  local -a tokens=(${(z)LBUFFER})
  if [[ $LBUFFER == *"$trig" && ${#tokens} -ge 1 ]]; then
    local last=${tokens[-1]}
    local prefix=${last%$trig} dir=. leaf mode=files
    leaf=$prefix
    if [[ $prefix == */* ]]; then
      dir=${prefix%/*}; leaf=${prefix##*/}
      [[ -z $dir ]] && dir=/
      dir=${~dir}
    fi
    case ${tokens[1]} in cd|pushd|rmdir) mode=dirs ;; esac
    local out
    out=$(__lvf $mode --multi --walker-root="$dir" --query="$leaf" < /dev/tty)
    [[ -n $out ]] && LBUFFER="${LBUFFER%$last}${(j: :)${(q-)${(f)out}}} "
    zle reset-prompt
  else
    zle ${__lvim_fuzzy_default_completion:-expand-or-complete}
  fi
}
__lvim_fuzzy_default_completion=$(bindkey '^I' | awk '{print $2}')
[[ $__lvim_fuzzy_default_completion == lvim-fuzzy-completion || -z $__lvim_fuzzy_default_completion ]] &&
  __lvim_fuzzy_default_completion=expand-or-complete
zle -N lvim-fuzzy-completion
bindkey -M emacs '^I' lvim-fuzzy-completion
bindkey -M viins '^I' lvim-fuzzy-completion
"#;

const BASH_BASE: &str = r#"
__lvf() {
  local bin=${LVIM_FUZZY_BIN:-lvim-fuzzy}
  "$bin" "$@" --height="${LVIM_FUZZY_HEIGHT:-$__lvf_height}" --popup="${LVIM_FUZZY_POPUP:-$__lvf_popup}"
}
__lvf_insert() {
  local line sel=
  while IFS= read -r line; do sel+="$(printf '%q' "$line") "; done < <(__lvf "$1" --multi < /dev/tty)
  READLINE_LINE="${READLINE_LINE:0:READLINE_POINT}$sel${READLINE_LINE:READLINE_POINT}"
  READLINE_POINT=$(( READLINE_POINT + ${#sel} ))
}
__lvf_cd() {
  local dir
  dir=$(__lvf "$1" < /dev/tty)
  [[ -n $dir ]] && builtin cd -- "$dir"
}
__lvf_replace() {
  local out
  out=$(__lvf "$1" --query="$READLINE_LINE" < /dev/tty)
  [[ -n $out ]] && { READLINE_LINE=$out; READLINE_POINT=${#READLINE_LINE}; }
}
__lvf_shell_history() {
  local out
  out=$(HISTTIMEFORMAT= builtin history | sed -E 's/^ *[0-9]+\*? *//' | tac | awk '!seen[$0]++' |
    __lvf --no-sort --scheme=history --title=History --query="$READLINE_LINE")
  [[ -n $out ]] && { READLINE_LINE=$out; READLINE_POINT=${#READLINE_LINE}; }
}
"#;

const FISH_BASE: &str = r#"
function __lvf
    set -l bin lvim-fuzzy
    set -q LVIM_FUZZY_BIN; and set bin $LVIM_FUZZY_BIN
    set -l height $__lvf_height
    set -q LVIM_FUZZY_HEIGHT; and set height $LVIM_FUZZY_HEIGHT
    set -l popup $__lvf_popup
    set -q LVIM_FUZZY_POPUP; and set popup $LVIM_FUZZY_POPUP
    $bin $argv --height=$height --popup=$popup
end
function __lvf_insert
    set -l out (__lvf $argv[1] --multi </dev/tty)
    test (count $out) -gt 0; and commandline -i -- (string join ' ' (string escape -- $out))' '
    commandline -f repaint
end
function __lvf_cd
    set -l dir (__lvf $argv[1] </dev/tty)
    test -n "$dir"; and builtin cd -- $dir
    commandline -f repaint
end
function __lvf_replace
    set -l out (__lvf $argv[1] --query=(commandline) </dev/tty | string collect)
    test -n "$out"; and commandline -r -- $out
    commandline -f repaint
end
function __lvf_shell_history
    set -l out (builtin history --null | __lvf --read0 --no-sort --scheme=history --title=History --query=(commandline) | string collect)
    test -n "$out"; and commandline -r -- $out
    commandline -f repaint
end
"#;

const FISH_COMPLETION: &str = r#"
function lvim-fuzzy-completion
    set -l trig $__lvf_trigger
    set -l token (commandline -t)
    if string match -q -- "*$trig" $token
        set -l prefix (string sub -e -(string length -- $trig) -- $token)
        set -l dir .
        set -l leaf $prefix
        if string match -q -- '*/*' $prefix
            set dir (string replace -r '/[^/]*$' '' -- $prefix)
            test -z "$dir"; and set dir /
            set leaf (string replace -r '.*/' '' -- $prefix)
        end
        set -l mode files
        switch (commandline -po)[1]
            case cd pushd rmdir
                set mode dirs
        end
        set -l out (__lvf $mode --multi --walker-root=$dir --query=$leaf </dev/tty)
        test (count $out) -gt 0; and commandline -t -- (string join ' ' (string escape -- $out))' '
        commandline -f repaint
    else
        commandline -f complete
    end
end
bind \t lvim-fuzzy-completion
bind -M insert \t lvim-fuzzy-completion 2>/dev/null
"#;

fn func_for(b: &Bind) -> (&'static str, Option<&str>) {
    if b.mode == "shell-history" {
        return ("__lvf_shell_history", None);
    }
    let f = match b.act {
        Act::Insert => "__lvf_insert",
        Act::Cd => "__lvf_cd",
        Act::Replace => "__lvf_replace",
    };
    (f, Some(&b.mode))
}

fn header(shell: &str, cfg: &ShellCfg) -> String {
    let mut s =
        format!("# lvim-fuzzy — {shell} integration, generated from the config's [shell].\n");
    if cfg.binds.is_empty() && cfg.completion.is_none() {
        s.push_str(
            "# No keys are set: add [shell.keys] to ~/.config/lvim-fuzzy/config.toml, e.g.\n",
        );
        s.push_str("#   [shell.keys]\n#   \"ctrl-t\" = \"files\"\n#   \"ctrl-r\" = \"shell-history\"\n#   \"alt-c\" = \"dirs:cd\"\n");
    }
    s
}

/// Completion of the `lvim-fuzzy` / `lfz` command itself (modes, subcommands,
/// options and their values); candidates come from `--complete-list`, so user
/// modes and installed themes are always current. Not a key binding.
const ZSH_COMMAND_COMPLETION: &str = r#"
_lvim_fuzzy() {
  local bin=${words[1]} opt
  local -a vals opts flags valued cmds
  # --opt=VALUE / --opt VALUE
  if [[ $PREFIX == --*=* ]]; then
    opt=${PREFIX%%=*}
    compset -P '*='
  elif [[ $words[CURRENT-1] == --* && $words[CURRENT-1] != *=* ]] &&
       "$bin" --complete-list options 2>/dev/null | grep -qx -- "$words[CURRENT-1]="; then
    opt=$words[CURRENT-1]
  fi
  if [[ -n $opt ]]; then
    vals=(${(f)"$("$bin" --complete-list values "$opt" 2>/dev/null)"})
    if (( $#vals )); then compadd -a vals; else _files; fi
    return
  fi
  if [[ $PREFIX == -* ]]; then
    opts=(${(f)"$("$bin" --complete-list options 2>/dev/null)"})
    flags=(${opts:#*=}); valued=(${(M)opts:#*=})
    compadd -a flags
    compadd -S '' -a valued
    return
  fi
  if (( CURRENT == 2 )); then
    cmds=(${(f)"$("$bin" --complete-list commands 2>/dev/null)"})
    cmds=(${cmds//:/\:}); cmds=(${cmds/$'	'/:})
    _describe -t commands 'lvim-fuzzy mode' cmds
    return
  fi
  if [[ $words[2] == config ]] && (( CURRENT == 3 )); then
    compadd check path
    return
  fi
  _files
}
if (( $+functions[compdef] )); then
  compdef _lvim_fuzzy lvim-fuzzy lfz
fi
"#;

const BASH_COMMAND_COMPLETION: &str = r#"
_lvim_fuzzy() {
  local bin=${COMP_WORDS[0]} cur=${COMP_WORDS[COMP_CWORD]} prev=${COMP_WORDS[COMP_CWORD-1]} opt= vals
  COMPREPLY=()
  # bash splits --opt=value at '='
  if [[ $cur == = ]]; then
    opt=$prev; cur=
  elif [[ $prev == = ]]; then
    opt=${COMP_WORDS[COMP_CWORD-2]}
  elif [[ $prev == --* ]] && "$bin" --complete-list options 2>/dev/null | grep -qx -- "$prev="; then
    opt=$prev
  fi
  if [[ -n $opt ]]; then
    vals=$("$bin" --complete-list values "$opt" 2>/dev/null)
    if [[ -n $vals ]]; then
      COMPREPLY=($(compgen -W "$vals" -- "$cur"))
    else
      COMPREPLY=($(compgen -f -- "$cur"))
    fi
    return
  fi
  if [[ $cur == -* ]]; then
    COMPREPLY=($(compgen -W "$("$bin" --complete-list options 2>/dev/null)" -- "$cur"))
    [[ ${#COMPREPLY[@]} -eq 1 && ${COMPREPLY[0]} == *= ]] && compopt -o nospace
    return
  fi
  if (( COMP_CWORD == 1 )); then
    COMPREPLY=($(compgen -W "$("$bin" --complete-list commands 2>/dev/null | cut -f1)" -- "$cur"))
    return
  fi
  if [[ ${COMP_WORDS[1]} == config ]] && (( COMP_CWORD == 2 )); then
    COMPREPLY=($(compgen -W "check path" -- "$cur"))
    return
  fi
  COMPREPLY=($(compgen -f -- "$cur"))
}
complete -o default -F _lvim_fuzzy lvim-fuzzy lfz
"#;

/// The fish completion: one `complete` line per option, values from
/// `--complete-list values`.
fn fish_command_completion() -> String {
    let mut s = String::from(
        "\ncomplete -c lvim-fuzzy -c lfz -f\n\
         complete -c lvim-fuzzy -c lfz -n __fish_is_first_arg -a '(lvim-fuzzy --complete-list commands)'\n\
         complete -c lvim-fuzzy -c lfz -n '__fish_seen_subcommand_from config' -a 'check path'\n",
    );
    for o in crate::options::completion_options() {
        let name = o.trim_start_matches("--");
        match name.strip_suffix('=') {
            Some(n) => s.push_str(&format!(
                "complete -c lvim-fuzzy -c lfz -l {n} -r -a '(lvim-fuzzy --complete-list values {n})'\n"
            )),
            None => s.push_str(&format!("complete -c lvim-fuzzy -c lfz -l {name}\n")),
        }
    }
    s
}

pub fn zsh(cfg: &ShellCfg) -> String {
    let (h, p) = defaults(cfg);
    let mut s = header("zsh", cfg);
    s.push_str("[[ -o interactive ]] || return 0\n");
    s.push_str(ZSH_COMMAND_COMPLETION);
    s.push_str(&format!(
        "__lvf_height={}\n__lvf_popup={}\n",
        quote(&h),
        quote(&p)
    ));
    s.push_str(ZSH_BASE);
    for (i, b) in cfg.binds.iter().enumerate() {
        let (f, mode) = func_for(b);
        let w = format!("__lvf_key_{i}");
        let call = match mode {
            Some(m) => format!("{f} {}", quote(m)),
            None => f.to_string(),
        };
        s.push_str(&format!(
            "{w}() {{ {call}; }}\nzle -N {w}\nbindkey -M emacs '{k}' {w}\nbindkey -M viins '{k}' {w}\n",
            k = zsh_key(&b.key)
        ));
    }
    if let Some(t) = &cfg.completion {
        s.push_str(&format!("__lvf_trigger={}\n", quote(t)));
        s.push_str(ZSH_COMPLETION);
    }
    s
}

pub fn bash(cfg: &ShellCfg) -> String {
    let (h, p) = defaults(cfg);
    let mut s = header("bash", cfg);
    s.push_str("[[ $- == *i* ]] || return 0\n");
    s.push_str(BASH_COMMAND_COMPLETION);
    s.push_str(&format!(
        "__lvf_height={}\n__lvf_popup={}\n",
        quote(&h),
        quote(&p)
    ));
    s.push_str(BASH_BASE);
    for b in &cfg.binds {
        let (f, mode) = func_for(b);
        let call = match mode {
            Some(m) => format!("{f} {m}"),
            None => f.to_string(),
        };
        let k = bash_key(&b.key);
        s.push_str(&format!(
            "bind -m emacs-standard -x '\"{k}\": {call}'\nbind -m vi-insert -x '\"{k}\": {call}'\n"
        ));
    }
    s
}

pub fn fish(cfg: &ShellCfg) -> String {
    let (h, p) = defaults(cfg);
    let mut s = header("fish", cfg);
    s.push_str("status is-interactive; or exit 0\n");
    s.push_str(&fish_command_completion());
    s.push_str(&format!(
        "set -g __lvf_height {}\nset -g __lvf_popup {}\n",
        quote(&h),
        quote(&p)
    ));
    s.push_str(FISH_BASE);
    for b in &cfg.binds {
        let (f, mode) = func_for(b);
        let call = match mode {
            Some(m) => format!("{f} {m}"),
            None => f.to_string(),
        };
        let k = fish_key(&b.key);
        s.push_str(&format!(
            "bind {k} '{call}'\nbind -M insert {k} '{call}' 2>/dev/null\n"
        ));
    }
    if let Some(t) = &cfg.completion {
        s.push_str(&format!("set -g __lvf_trigger {}\n", quote(t)));
        s.push_str(FISH_COMPLETION);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_per_shell() {
        assert_eq!(zsh_key("ctrl-t"), "^T");
        assert_eq!(zsh_key("alt-c"), "\\ec");
        assert_eq!(bash_key("ctrl-alt-g"), "\\e\\C-g");
        assert_eq!(fish_key("ctrl-r"), "\\cr");
        assert!(key_ok("ctrl-t") && key_ok("alt-c") && key_ok("ctrl-alt-x"));
        assert!(!key_ok("f1") && !key_ok("ctrl-tab") && !key_ok("hyper-x"));
    }

    #[test]
    fn nothing_bound_without_keys() {
        let s = zsh(&ShellCfg::default());
        assert!(!s.contains("bindkey -M"), "{s}");
        assert!(s.contains("No keys are set"));
    }

    #[test]
    fn binds_what_the_config_says() {
        let cfg = ShellCfg {
            completion: Some("**".into()),
            binds: vec![
                Bind {
                    key: "ctrl-t".into(),
                    mode: "files".into(),
                    act: Act::Insert,
                },
                Bind {
                    key: "alt-c".into(),
                    mode: "dirs".into(),
                    act: Act::Cd,
                },
                Bind {
                    key: "ctrl-r".into(),
                    mode: "shell-history".into(),
                    act: Act::Insert,
                },
            ],
            ..ShellCfg::default()
        };
        let z = zsh(&cfg);
        assert!(z.contains("__lvf_key_0() { __lvf_insert 'files'; }"));
        assert!(z.contains("bindkey -M emacs '\\ec' __lvf_key_1"));
        assert!(z.contains("__lvf_key_2() { __lvf_shell_history; }"));
        assert!(z.contains("lvim-fuzzy-completion"));
        let b = bash(&cfg);
        assert!(b.contains("bind -m emacs-standard -x '\"\\C-t\": __lvf_insert files'"));
        assert!(fish(&cfg).contains("bind \\ec '__lvf_cd dirs'"));
    }
}
