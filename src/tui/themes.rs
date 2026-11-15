//! The theme picker (F5): the installed lvim themes in a popup. Moving through
//! the list switches the theme LIVE; Enter keeps it, Esc puts the old one back.
//! Themes are files in ~/.config/lvim-fuzzy/themes (lvim-colorscheme generates
//! them: `scripts/build lvim_fuzzy`); LvimBase_dark is built in.

use super::*;

pub(super) struct ThemeMenu {
    names: Vec<String>,
    query: String,
    cursor: usize,
    /// What to restore on Esc.
    original: (Option<crate::lvim::Look>, Theme),
    original_name: String,
}

impl App {
    fn theme_rows(&self) -> Vec<String> {
        let Some(m) = &self.theme_menu else {
            return Vec::new();
        };
        let q = m.query.to_lowercase();
        m.names
            .iter()
            .filter(|n| q.is_empty() || n.to_lowercase().contains(&q))
            .cloned()
            .collect()
    }

    pub(super) fn open_theme_menu(&mut self) {
        if self.look.is_none() {
            self.toast(
                "themes belong to the lvim look (--style=lvim)".into(),
                false,
            );
            return;
        }
        let names = crate::lvim::Theme::list();
        let current = self.theme_name.clone();
        let cursor = names.iter().position(|n| *n == current).unwrap_or(0);
        self.theme_menu = Some(ThemeMenu {
            names,
            query: String::new(),
            cursor,
            original: (self.look.clone(), self.theme.clone()),
            original_name: current,
        });
    }

    /// Switch to theme `name` now (the picker previews it live).
    fn apply_theme(&mut self, name: &str) {
        match crate::lvim::Theme::load(name) {
            Ok(t) => {
                let look = crate::lvim::Look::new(t);
                if let Ok(th) = build_theme(&self.opts, Some(&look)) {
                    self.theme = th;
                }
                self.look = Some(look);
                self.opts.theme = Some(name.to_string());
                self.theme_name = name.to_string();
            }
            Err(e) => self.toast(e, true),
        }
    }

    pub(super) fn theme_key(&mut self, key: &str) {
        let rows = self.theme_rows();
        let Some(m) = self.theme_menu.as_mut() else {
            return;
        };
        let mut moved = false;
        match key {
            "esc" | "f5" | "ctrl-c" | "ctrl-g" | "ctrl-q" => {
                let m = self.theme_menu.take().unwrap();
                self.look = m.original.0;
                self.theme = m.original.1;
                self.theme_name = m.original_name;
                return;
            }
            "enter" => {
                let name = self.theme_name.clone();
                self.theme_menu = None;
                self.toast(
                    format!("theme {name} for this session (ctrl-w in the picker saves it)"),
                    false,
                );
                return;
            }
            "ctrl-w" => {
                let name = self.theme_name.clone();
                self.theme_menu = None;
                match save_theme(&name) {
                    Ok(path) => self.toast(format!("theme = \"{name}\" saved in {path}"), false),
                    Err(e) => self.toast(format!("cannot save the theme: {e}"), true),
                }
                return;
            }
            "up" | "ctrl-p" | "ctrl-k" | "shift-tab" | "scroll-up" => {
                m.cursor = if m.cursor == 0 {
                    rows.len().saturating_sub(1)
                } else {
                    m.cursor - 1
                };
                moved = true;
            }
            "down" | "ctrl-n" | "ctrl-j" | "tab" | "scroll-down" => {
                m.cursor = if m.cursor + 1 >= rows.len() {
                    0
                } else {
                    m.cursor + 1
                };
                moved = true;
            }
            "bspace" => {
                m.query.pop();
                m.cursor = 0;
            }
            "ctrl-u" => {
                m.query.clear();
                m.cursor = 0;
            }
            k if k.chars().count() == 1 => {
                m.query.push_str(k);
                m.cursor = 0;
            }
            _ => {}
        }
        let rows = self.theme_rows();
        let cursor = self.theme_menu.as_ref().map_or(0, |m| m.cursor);
        if (moved || !rows.is_empty())
            && let Some(name) = rows.get(cursor).cloned()
            && name != self.theme_name
        {
            self.apply_theme(&name);
        }
    }

    pub(super) fn draw_theme_menu(
        &self,
        cv: &mut Canvas,
        area: Rect,
        look: Option<&crate::lvim::Look>,
    ) {
        let (Some(m), Some(l)) = (&self.theme_menu, look.or(self.look.as_ref())) else {
            return;
        };
        let rows = self.theme_rows();
        let w = area.w.saturating_sub(4).min(56);
        let h = area
            .h
            .saturating_sub(2)
            .min(rows.len() as u16 + 6)
            .max(6.min(area.h));
        // top-right, so the finder behind it shows the theme
        let r = Rect {
            x: area.x + area.w.saturating_sub(w + 2),
            y: area.y + 1,
            w,
            h,
        };
        let pst = self.popup_style(Some(l));
        let inner = self.popup_frame(cv, r, Some("Theme"), &pst);
        let limit = inner.x + inner.w;
        let mut x = cv.put(inner.x, inner.y, " 󰏘 ", l.prompt, limit);
        x = cv.put(x, inner.y, " ", pst.panel, limit);
        cv.fill(
            Rect {
                x,
                y: inner.y,
                w: limit.saturating_sub(x),
                h: 1,
            },
            l.input,
        );
        if m.query.is_empty() {
            cv.put(
                x + 1,
                inner.y,
                "type to filter",
                Style {
                    bg: l.input.bg,
                    ..l.count
                },
                limit,
            );
        } else {
            cv.put(x + 1, inner.y, &m.query, l.input, limit);
        }
        // filter line, a blank row, the list, the hint on the last row
        let body_h = inner.h.saturating_sub(3) as usize;
        let off = m.cursor.saturating_sub(body_h.saturating_sub(1));
        for (i, name) in rows.iter().enumerate().skip(off).take(body_h) {
            let y = inner.y + 2 + (i - off) as u16;
            // plain rows on the popup's background; only the cursor row is lit
            let st = if i == m.cursor { pst.cursor } else { pst.panel };
            cv.fill(
                Rect {
                    x: inner.x,
                    y,
                    w: inner.w,
                    h: 1,
                },
                st,
            );
            let mark = if *name == self.theme_name {
                "● "
            } else {
                "  "
            };
            // ● the theme in use, in the accent colour (apart from the cursor)
            let x = cv.put(
                inner.x,
                y,
                mark,
                Style {
                    bg: st.bg,
                    ..l.prompt
                },
                limit,
            );
            cv.put(x, y, name, st, limit);
        }
        if rows.is_empty() {
            cv.put(inner.x, inner.y + 2, "no themes match", l.count, limit);
        }
        let hint = format!("{} themes · enter use · ctrl-w save · esc back", rows.len());
        cv.put(
            inner.x + (inner.w.saturating_sub(str_width(&hint) as u16)) / 2,
            inner.y + inner.h.saturating_sub(1),
            &hint,
            Style {
                bg: pst.panel.bg,
                ..l.count
            },
            r.x + r.w - 1,
        );
    }
}

/// Write `theme = "<name>"` into the config (the line themer rewrites too).
fn save_theme(name: &str) -> Result<String, String> {
    save_top_key("theme", name)
}

/// Write `KEY = "<value>"` into the config: replace the top-level line, or
/// add one at the top. The rest of the file stays as it is.
pub(super) fn save_top_key(key: &str, value: &str) -> Result<String, String> {
    let path = crate::config::path(None).ok_or("no config path")?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let line = format!("{key} = \"{value}\"");
    let mut done = false;
    let mut out: Vec<String> = Vec::new();
    let mut in_table = false;
    for l in text.lines() {
        let t = l.trim_start();
        if t.starts_with('[') {
            in_table = true;
        }
        let is_theme = t
            .strip_prefix(key)
            .is_some_and(|r| r.trim_start().starts_with('='));
        if !in_table && !done && is_theme {
            out.push(line.clone());
            done = true;
        } else {
            out.push(l.to_string());
        }
    }
    if !done {
        out.insert(0, line);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, out.join("\n") + "\n").map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}
