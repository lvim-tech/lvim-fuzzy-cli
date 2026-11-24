//! What each look remembers between runs: whether its preview was left shown, and on which side. The minimal look (`--minimal`) and the full one keep
//! their own: one line each, `minimal=on left` / `full=off right`, in `$XDG_STATE_HOME/lvim-fuzzy/preview` (`~/.local/state/lvim-fuzzy/preview`). A look
//! that has none starts the way it always did: the minimal one without the preview, the full one with it, on the right. Options that name a side
//! (`--preview-window`) win over the remembered one.

use std::path::PathBuf;

use crate::preview::Pos;

fn path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("lvim-fuzzy/preview")
}

fn key(minimal: bool) -> &'static str {
    if minimal { "minimal" } else { "full" }
}

/// What a look left: the preview shown or not, and its side (None: not said yet).
#[derive(Default)]
pub struct Saved {
    pub visible: Option<bool>,
    pub side: Option<Pos>,
    /// the preview's share of the width (or height), in percent
    pub size: Option<u16>,
}

fn side_name(p: Pos) -> &'static str {
    match p {
        Pos::Up => "up",
        Pos::Down => "down",
        Pos::Left => "left",
        Pos::Right => "right",
    }
}

fn side_of(s: &str) -> Option<Pos> {
    match s {
        "up" => Some(Pos::Up),
        "down" => Some(Pos::Down),
        "left" => Some(Pos::Left),
        "right" => Some(Pos::Right),
        _ => None,
    }
}

pub fn get(minimal: bool) -> Saved {
    let Ok(text) = std::fs::read_to_string(path()) else { return Saved::default() };
    for l in text.lines() {
        let Some((k, v)) = l.split_once('=') else { continue };
        if k.trim() != key(minimal) {
            continue;
        }
        let mut w = v.split_whitespace();
        return Saved {
            visible: w.next().map(|s| s == "on"),
            side: w.next().and_then(side_of),
            size: w.next().and_then(|s| s.parse().ok()),
        };
    }
    Saved::default()
}

/// Remember it for this look (the other look's line stays). A file that cannot be written is no error: the preview just starts the old way.
pub fn save(minimal: bool, visible: bool, side: Pos, size: Option<u16>) {
    let p = path();
    let other = key(!minimal);
    let keep = std::fs::read_to_string(&p)
        .ok()
        .and_then(|t| t.lines().find(|l| l.split_once('=').is_some_and(|(k, _)| k.trim() == other)).map(str::to_string));
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let mut text = format!("{}={} {}", key(minimal), if visible { "on" } else { "off" }, side_name(side));
    if let Some(n) = size {
        text.push_str(&format!(" {n}"));
    }
    text.push('\n');
    if let Some(l) = keep {
        text.push_str(&l);
        text.push('\n');
    }
    let _ = std::fs::write(p, text);
}
