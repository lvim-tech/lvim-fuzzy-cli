//! Key names, the `--bind` grammar and the default key map (as in fzf).

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub name: String,
    pub arg: Option<String>,
}

impl Action {
    fn new(name: &str) -> Action {
        Action {
            name: name.to_string(),
            arg: None,
        }
    }
}

/// Every fzf action name (0.75). Actions marked with a trailing `(` take an argument.
pub const ACTIONS: &[&str] = &[
    "abort",
    "accept",
    "accept-non-empty",
    "accept-or-print-query",
    "backward-char",
    "backward-delete-char",
    "backward-delete-char/eof",
    "backward-kill-subword",
    "backward-kill-word",
    "backward-subword",
    "backward-word",
    "beginning-of-line",
    "bell",
    "become(",
    "bg-cancel",
    "cancel",
    "change-ghost(",
    "change-header(",
    "change-multi",
    "change-multi(",
    "change-nth(",
    "change-preview(",
    "change-preview-label(",
    "change-preview-window(",
    "change-prompt(",
    "change-query(",
    "clear-multi",
    "clear-query",
    "clear-screen",
    "clear-selection",
    "close",
    "delete-char",
    "delete-char/eof",
    "deselect",
    "deselect-all",
    "disable-raw",
    "disable-search",
    "down",
    "down-match",
    "down-selected",
    "enable-raw",
    "enable-search",
    "end-of-line",
    "exclude",
    "exclude-multi",
    "execute(",
    "execute-silent(",
    "first",
    "forward-char",
    "forward-subword",
    "forward-word",
    "hide-header",
    "hide-input",
    "hide-preview",
    "ignore",
    "jump",
    "jump-accept",
    "kill-line",
    "kill-subword",
    "kill-word",
    "last",
    "next-history",
    "next-selected",
    "offset-down",
    "offset-middle",
    "offset-up",
    "page-down",
    "page-up",
    "half-page-down",
    "half-page-up",
    "pos(",
    "prev-history",
    "prev-selected",
    "preview(",
    "preview-down",
    "preview-up",
    "preview-page-down",
    "preview-page-up",
    "preview-half-page-down",
    "preview-half-page-up",
    "preview-bottom",
    "preview-top",
    "print(",
    "print-query",
    "put",
    "put(",
    "refresh-preview",
    "rebind(",
    "reload(",
    "reload-sync(",
    "replace-query",
    "row-tip",
    "divider-left",
    "divider-right",
    "divider-up",
    "divider-down",
    "search(",
    "select",
    "select-all",
    "show-header",
    "show-input",
    "show-preview",
    "toggle",
    "toggle-all",
    "toggle+down",
    "toggle+up",
    "toggle-bind",
    "toggle-header",
    "toggle-hscroll",
    "toggle-in",
    "toggle-input",
    "toggle-multi-line",
    "toggle-out",
    "toggle-preview",
    "toggle-preview-wrap",
    "toggle-preview-wrap-word",
    "toggle-raw",
    "toggle-search",
    "toggle-sort",
    "toggle-track",
    "toggle-track-current",
    "toggle-wrap",
    "toggle-wrap-word",
    "track",
    "track-current",
    "transform(",
    "transform-ghost(",
    "transform-header(",
    "transform-nth(",
    "transform-preview-label(",
    "transform-prompt(",
    "transform-query(",
    "transform-search(",
    "unbind(",
    "unix-line-discard",
    "unix-word-rubout",
    "untrack-current",
    "up",
    "up-match",
    "up-selected",
    "yank",
    "bg-transform(",
    "bg-transform-ghost(",
    "bg-transform-header(",
    "bg-transform-nth(",
    "bg-transform-preview-label(",
    "bg-transform-prompt(",
    "bg-transform-query(",
    "bg-transform-search(",
    "trigger(",
    "change-with-nth(",
    "transform-with-nth(",
    "bg-transform-with-nth(",
    // lvim-fuzzy extensions
    "help",
    "toggle-preview-focus",
    "rotate-preview",
    "rotate-preview-back",
    "toggle-scope",
    "mode-menu",
    "cd",
    "switch-mode(",
    "theme-menu",
    "toggle-grep-filter",
    "open",
];

fn action_known(name: &str, has_arg: bool) -> bool {
    if has_arg {
        ACTIONS.iter().any(|a| a.strip_suffix('(') == Some(name))
    } else {
        ACTIONS.iter().any(|a| *a == name)
    }
}

/// Canonical key / event names accepted in `--bind` and `--expect`.
pub fn canonical_key(s: &str) -> Result<String, String> {
    let lower = s.to_ascii_lowercase();
    let k = match lower.as_str() {
        "return" | "ctrl-m" => "enter".to_string(),
        "bs" | "ctrl-h" => "bspace".to_string(),
        "btab" => "shift-tab".to_string(),
        "delete" => "del".to_string(),
        "ctrl-i" => "tab".to_string(),
        "pgup" => "page-up".to_string(),
        "pgdn" => "page-down".to_string(),
        "ctrl-[" => "esc".to_string(),
        "ctrl-alt-h" => "ctrl-alt-bspace".to_string(),
        "change" | "start" | "load" | "focus" | "result" | "resize" | "one" | "zero" | "multi"
        | "backward-eof" | "jump" | "jump-cancel" | "click-header" | "click-footer" => {
            lower.clone()
        }
        _ => {
            if s.chars().count() == 1 {
                return Ok(s.to_string());
            }
            if let Some(c) = s.strip_prefix("alt-").filter(|c| c.chars().count() == 1) {
                return Ok(format!("alt-{c}"));
            }
            if is_known_key(&lower) {
                lower.clone()
            } else {
                return Err(format!("unsupported key: {s}"));
            }
        }
    };
    Ok(k)
}

fn is_known_key(k: &str) -> bool {
    const NAMED: &[&str] = &[
        "enter",
        "space",
        "bspace",
        "tab",
        "shift-tab",
        "esc",
        "del",
        "delete",
        "insert",
        "up",
        "down",
        "left",
        "right",
        "home",
        "end",
        "page-up",
        "page-down",
        "shift-up",
        "shift-down",
        "shift-left",
        "shift-right",
        "shift-delete",
        "alt-up",
        "alt-down",
        "alt-left",
        "alt-right",
        "alt-enter",
        "alt-space",
        "alt-bspace",
        "ctrl-space",
        "ctrl-\\",
        "ctrl-]",
        "ctrl-^",
        "ctrl-/",
        "ctrl-up",
        "ctrl-down",
        "ctrl-left",
        "ctrl-right",
        "ctrl-home",
        "ctrl-end",
        "ctrl-delete",
        "ctrl-page-up",
        "ctrl-page-down",
        "shift-home",
        "shift-end",
        "shift-page-up",
        "shift-page-down",
        "alt-home",
        "alt-end",
        "alt-page-up",
        "alt-page-down",
        "alt-shift-up",
        "alt-shift-down",
        "alt-shift-left",
        "alt-shift-right",
        "ctrl-alt-bspace",
        "ctrl-bspace",
        "left-click",
        "right-click",
        "double-click",
        "middle-click",
        "scroll-up",
        "scroll-down",
        "shift-scroll-up",
        "shift-scroll-down",
        "preview-scroll-up",
        "preview-scroll-down",
        "shift-left-click",
        "shift-right-click",
        "alt-delete",
    ];
    if NAMED.contains(&k) {
        return true;
    }
    if let Some(n) = k.strip_prefix('f') {
        return n.parse::<u8>().is_ok_and(|n| (1..=12).contains(&n));
    }
    for p in ["ctrl-alt-", "ctrl-", "alt-"] {
        if let Some(c) = k.strip_prefix(p) {
            return c.len() == 1 && c.as_bytes()[0].is_ascii_lowercase();
        }
    }
    false
}

/// Name of a terminal key event, in the same vocabulary.
pub fn key_name(e: &KeyEvent) -> Option<String> {
    let m = e.modifiers;
    let ctrl = m.contains(KeyModifiers::CONTROL);
    let alt = m.contains(KeyModifiers::ALT);
    let shift = m.contains(KeyModifiers::SHIFT);
    let pre = |base: &str| -> String {
        let mut s = String::new();
        if ctrl {
            s.push_str("ctrl-");
        }
        if alt {
            s.push_str("alt-");
        }
        if shift {
            s.push_str("shift-");
        }
        s.push_str(base);
        s
    };
    Some(match e.code {
        KeyCode::Char(c) => {
            if ctrl {
                let c = c.to_ascii_lowercase();
                let base = match c {
                    ' ' => "space".to_string(),
                    'm' => {
                        return Some(if alt {
                            "alt-enter".into()
                        } else {
                            "enter".into()
                        });
                    }
                    'h' if !alt => return Some("bspace".into()),
                    // Ctrl-i arrives as a key of its own only where the terminal tells it from Tab (the kitty keyboard
                    // protocol); anywhere else the terminal sends Tab and it is read as Tab (a KeyCode::Tab)
                    'i' if !alt => return Some("ctrl-i".into()),
                    '[' => return Some("esc".into()),
                    // the terminal sends 0x1c..0x1f, which crossterm reads as ctrl-4..7
                    '4' => "\\".to_string(),
                    '5' => "]".to_string(),
                    '6' => "^".to_string(),
                    '7' => "/".to_string(),
                    c => c.to_string(),
                };
                if alt {
                    format!("ctrl-alt-{base}")
                } else {
                    format!("ctrl-{base}")
                }
            } else if alt {
                if c == ' ' {
                    "alt-space".into()
                } else {
                    format!("alt-{c}")
                }
            } else if c == ' ' {
                "space".into()
            } else {
                c.to_string()
            }
        }
        KeyCode::Enter => {
            if alt {
                "alt-enter".into()
            } else if ctrl {
                // only terminals that report it (kitty keyboard protocol); most send enter
                "ctrl-enter".into()
            } else if shift {
                "shift-enter".into()
            } else {
                "enter".into()
            }
        }
        KeyCode::Backspace => {
            if ctrl && alt {
                "ctrl-alt-bspace".into()
            } else if alt {
                "alt-bspace".into()
            } else if ctrl {
                "ctrl-bspace".into()
            } else {
                "bspace".into()
            }
        }
        KeyCode::Tab => {
            if shift {
                "shift-tab".into()
            } else {
                "tab".into()
            }
        }
        KeyCode::BackTab => "shift-tab".into(),
        KeyCode::Esc => "esc".into(),
        KeyCode::Delete => pre("delete"),
        KeyCode::Insert => "insert".into(),
        KeyCode::Up => pre("up"),
        KeyCode::Down => pre("down"),
        KeyCode::Left => pre("left"),
        KeyCode::Right => pre("right"),
        KeyCode::Home => pre("home"),
        KeyCode::End => pre("end"),
        KeyCode::PageUp => pre("page-up"),
        KeyCode::PageDown => pre("page-down"),
        KeyCode::F(n) => format!("f{n}"),
        _ => return None,
    })
    .map(|s| if s == "delete" { "del".to_string() } else { s })
}

pub fn mouse_name(e: &MouseEvent) -> Option<&'static str> {
    let shift = e.modifiers.contains(KeyModifiers::SHIFT);
    Some(match e.kind {
        MouseEventKind::ScrollUp => {
            if shift {
                "shift-scroll-up"
            } else {
                "scroll-up"
            }
        }
        MouseEventKind::ScrollDown => {
            if shift {
                "shift-scroll-down"
            } else {
                "scroll-down"
            }
        }
        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
            if shift {
                "shift-left-click"
            } else {
                "left-click"
            }
        }
        MouseEventKind::Down(crossterm::event::MouseButton::Right) => "right-click",
        _ => return None,
    })
}

const OPENERS: &[(char, char)] = &[
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('<', '>'),
    ('~', '~'),
    ('!', '!'),
    ('@', '@'),
    ('#', '#'),
    ('$', '$'),
    ('%', '%'),
    ('^', '^'),
    ('&', '&'),
    ('*', '*'),
    (';', ';'),
    ('/', '/'),
    ('|', '|'),
];

/// Parse one `--bind` value into (key, actions) pairs.
pub fn parse_binds(spec: &str) -> Result<Vec<(String, Vec<Action>)>, String> {
    let chars: Vec<char> = spec.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        // key: up to the first ':' (a key may itself be ':' or ',')
        let key_start = i;
        if i < chars.len() {
            i += 1;
        }
        while i < chars.len() && chars[i] != ':' {
            i += 1;
        }
        let key: String = chars[key_start..i].iter().collect();
        if i >= chars.len() {
            return Err(format!("bind action not specified: {spec}"));
        }
        i += 1; // ':'
        let key = canonical_key(&key)?;
        let mut actions = Vec::new();
        loop {
            let ns = i;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '-' | '+' | '/'))
            {
                // `toggle+down`, `toggle+up`, `delete-char/eof` are single names
                if chars[i] == '+' {
                    let name: String = chars[ns..i].iter().collect();
                    let rest: String = chars[i..chars.len().min(i + 5)].iter().collect();
                    if name == "toggle" && (rest.starts_with("+down") || rest.starts_with("+up")) {
                        i += if rest.starts_with("+down") { 5 } else { 3 };
                        continue;
                    }
                    break;
                }
                if chars[i] == '/' {
                    let name: String = chars[ns..i].iter().collect();
                    let rest: String = chars[i..chars.len().min(i + 4)].iter().collect();
                    if rest == "/eof" && name.ends_with("char") {
                        i += 4;
                        continue;
                    }
                    break;
                }
                i += 1;
            }
            let name: String = chars[ns..i].iter().collect();
            if name.is_empty() {
                return Err(format!("invalid bind action: {spec}"));
            }
            let mut arg = None;
            if i < chars.len() {
                let c = chars[i];
                if c == ':' {
                    arg = Some(chars[i + 1..].iter().collect::<String>());
                    i = chars.len();
                } else if let Some(&(_, close)) = OPENERS.iter().find(|(o, _)| *o == c) {
                    let start = i + 1;
                    let mut j = start;
                    while j < chars.len() && chars[j] != close {
                        j += 1;
                    }
                    if j >= chars.len() {
                        return Err(format!("unclosed action argument: {spec}"));
                    }
                    arg = Some(chars[start..j].iter().collect());
                    i = j + 1;
                }
            }
            if !action_known(&name, arg.is_some()) {
                return Err(format!("unknown action: {name}"));
            }
            actions.push(Action { name, arg });
            if i < chars.len() && chars[i] == '+' {
                i += 1;
                continue;
            }
            break;
        }
        out.push((key, actions));
        if i < chars.len() {
            if chars[i] == ',' {
                i += 1;
            } else {
                return Err(format!("invalid bind specification: {spec}"));
            }
        }
    }
    Ok(out)
}

pub type KeyMap = HashMap<String, Vec<Action>>;

/// fzf's default bindings.
pub fn default_keymap() -> KeyMap {
    let mut m = KeyMap::new();
    let mut b = |k: &str, a: &str| {
        m.insert(k.to_string(), vec![Action::new(a)]);
    };
    for k in ["ctrl-c", "ctrl-g", "ctrl-q", "esc"] {
        b(k, "abort");
    }
    b("enter", "accept");
    b("double-click", "accept");
    for k in ["ctrl-j", "ctrl-n", "down"] {
        b(k, "down");
    }
    for k in ["ctrl-k", "up"] {
        b(k, "up");
    }
    // (fzf has ctrl-p as up: here ctrl-p is the preview of the row, shown and hidden; ctrl-k and the arrow move up)
    b("ctrl-p", "toggle-preview");
    // (the whole text of the row, when it is cut)
    b("ctrl-i", "row-tip");
    // (the divider between the list and the preview moves with the Shift-arrows; Alt-b / Alt-f move by a word in the query)
    b("shift-left", "divider-left");
    b("shift-right", "divider-right");
    b("shift-up", "divider-up");
    b("shift-down", "divider-down");
    b("tab", "toggle+down");
    b("shift-tab", "toggle+up");
    b("ctrl-a", "beginning-of-line");
    b("home", "beginning-of-line");
    b("ctrl-e", "end-of-line");
    b("end", "end-of-line");
    b("ctrl-b", "backward-char");
    b("left", "backward-char");
    b("ctrl-f", "forward-char");
    b("right", "forward-char");
    b("alt-b", "backward-word");
    b("alt-f", "forward-word");
    b("bspace", "backward-delete-char");
    b("ctrl-bspace", "backward-delete-char");
    b("del", "delete-char");
    b("ctrl-d", "delete-char/eof");
    b("ctrl-u", "unix-line-discard");
    b("ctrl-w", "unix-word-rubout");
    b("alt-d", "kill-word");
    b("alt-bspace", "backward-kill-word");
    b("ctrl-alt-bspace", "backward-kill-word");
    b("ctrl-y", "yank");
    b("ctrl-l", "clear-screen");
    b("page-up", "page-up");
    b("page-down", "page-down");
    b("scroll-up", "up");
    b("scroll-down", "down");
    b("shift-scroll-up", "preview-up");
    b("shift-scroll-down", "preview-down");
    b("preview-scroll-up", "preview-up");
    b("preview-scroll-down", "preview-down");
    b("left-click", "ignore");
    b("ctrl-/", "toggle-wrap");
    b("alt-/", "toggle-wrap");
    b("f1", "help");
    b("ctrl-]", "toggle-preview-focus");
    b("alt-p", "rotate-preview");
    b("alt-P", "rotate-preview-back");
    b("alt-e", "toggle-preview");
    // a function key: multiplexers take ctrl-s (Zellij), window managers alt-<letter>
    b("f4", "toggle-scope");
    b("f2", "mode-menu");
    b("f3", "toggle-grep-filter");
    b("f5", "theme-menu");
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_punctuation_keys() {
        let k = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        assert_eq!(key_name(&k('5')).as_deref(), Some("ctrl-]"));
        assert_eq!(key_name(&k('4')).as_deref(), Some("ctrl-\\"));
        assert_eq!(key_name(&k(']')).as_deref(), Some("ctrl-]"));
        assert_eq!(
            default_keymap().get("ctrl-]").map(|a| a[0].name.as_str()),
            Some("toggle-preview-focus")
        );
        assert_eq!(
            default_keymap().get("f4").map(|a| a[0].name.as_str()),
            Some("toggle-scope")
        );
    }

    #[test]
    fn binds() {
        let b = parse_binds("ctrl-a:select-all+accept,alt-x:execute[echo {}],f1:reload:ls -l, x")
            .unwrap();
        assert_eq!(b[0].0, "ctrl-a");
        assert_eq!(b[0].1.len(), 2);
        assert_eq!(b[1].1[0].arg.as_deref(), Some("echo {}"));
        assert_eq!(b[2].1[0].arg.as_deref(), Some("ls -l, x"));
        let b = parse_binds("tab:toggle+down,ctrl-d:delete-char/eof,::accept").unwrap();
        assert_eq!(b[0].1[0].name, "toggle+down");
        assert_eq!(b[1].1[0].name, "delete-char/eof");
        assert_eq!(b[2].0, ":");
        assert!(parse_binds("ctrl-a:nope").is_err());
        assert!(parse_binds("hyper-a:up").is_err());
    }
}
