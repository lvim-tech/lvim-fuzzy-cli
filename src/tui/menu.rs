//! The mode menu (F2): every mode in a popup — type to filter, Enter to switch.
//! Switching re-runs lvim-fuzzy in the chosen mode with the same query and the
//! user's own options, in the same terminal.

use super::*;

pub(super) struct Menu {
    pub query: String,
    pub cursor: usize,
}

impl App {
    /// The modes matching the menu's filter (name, description, missing programs).
    fn menu_rows(&self) -> Vec<&'static crate::modes::Mode> {
        let q = self
            .menu
            .as_ref()
            .map(|m| m.query.to_lowercase())
            .unwrap_or_default();
        let mut ranked: Vec<(u8, usize, &'static crate::modes::Mode)> = crate::all_modes()
            .iter()
            .enumerate()
            .filter_map(|(i, m)| mode_rank(m, &q).map(|r| (r, i, m)))
            .collect();
        ranked.sort_by_key(|(r, i, _)| (*r, *i));
        ranked.into_iter().map(|(_, _, m)| m).collect()
    }

    pub(super) fn open_menu(&mut self) {
        let current = self.opts.mode_name.clone().unwrap_or_default();
        let cursor = crate::all_modes()
            .iter()
            .position(|m| m.name == current)
            .unwrap_or(0);
        self.menu = Some(Menu {
            query: String::new(),
            cursor,
        });
    }

    /// A key while the menu is open; Some(mode) when one was chosen.
    pub(super) fn menu_key(&mut self, name: &str) -> Option<String> {
        let rows = self.menu_rows().len();
        let menu = self.menu.as_mut()?;
        match name {
            "esc" | "f2" | "ctrl-c" | "ctrl-g" | "ctrl-q" => self.menu = None,
            "up" | "ctrl-p" | "ctrl-k" | "shift-tab" | "scroll-up" => {
                menu.cursor = if menu.cursor == 0 {
                    rows.saturating_sub(1)
                } else {
                    menu.cursor - 1
                }
            }
            "down" | "ctrl-n" | "ctrl-j" | "tab" | "scroll-down" => {
                menu.cursor = if menu.cursor + 1 >= rows {
                    0
                } else {
                    menu.cursor + 1
                }
            }
            "bspace" => {
                menu.query.pop();
                menu.cursor = 0;
            }
            "ctrl-u" => {
                menu.query.clear();
                menu.cursor = 0;
            }
            "enter" => {
                let pick = self.menu_rows().get(self.menu.as_ref()?.cursor).copied()?;
                let missing = pick.missing();
                if !missing.is_empty() {
                    self.toast(
                        format!("mode `{}` needs: {}", pick.name, missing.join(", ")),
                        true,
                    );
                    return None;
                }
                self.menu = None;
                return Some(pick.name.clone());
            }
            "space" => {
                menu.query.push(' ');
                menu.cursor = 0;
            }
            n if n.chars().count() == 1 => {
                menu.query.push_str(n);
                menu.cursor = 0;
            }
            _ => {}
        }
        None
    }

    pub(super) fn draw_menu(&self, cv: &mut Canvas, area: Rect, look: Option<&crate::lvim::Look>) {
        let look = look.or(self.look.as_ref());
        let Some(menu) = &self.menu else { return };
        let rows = self.menu_rows();
        let current = self.opts.mode_name.clone().unwrap_or_default();
        // styles: the lvim cheatsheet stripes when the lvim look is on
        let matched = match look {
            Some(l) => l.matched,
            None => self.theme.get(Slot::Hl),
        };
        let (_, _, _, key, _, _, dim, prompt, input) = match look {
            Some(l) => (
                l.popup,
                l.popup_border,
                l.title,
                l.help_key,
                l.help_desc,
                l.sel[0],
                l.count,
                l.prompt,
                l.input,
            ),
            None => {
                let t = &self.theme;
                let p = Style {
                    bg: t.get(Slot::Bg).bg,
                    ..t.get(Slot::Fg)
                };
                (
                    p,
                    t.get(Slot::Border),
                    t.get(Slot::Label),
                    [t.get(Slot::Hl), t.get(Slot::Hl)],
                    [p, p],
                    Style {
                        bg: t.get(Slot::CurBg).bg,
                        ..t.get(Slot::CurFg)
                    },
                    t.get(Slot::Info),
                    t.get(Slot::Prompt),
                    t.get(Slot::Query),
                )
            }
        };
        let name_w = crate::all_modes()
            .iter()
            .map(|m| str_width(&m.name))
            .max()
            .unwrap_or(8) as u16
            + 2;
        let w = area.w.saturating_sub(4).min(96);
        let h = area
            .h
            .saturating_sub(2)
            .min(rows.len() as u16 + 5)
            .max(6.min(area.h));
        let r = Rect {
            x: area.x + (area.w - w) / 2,
            y: area.y + (area.h - h) / 2,
            w,
            h,
        };
        let pst = self.popup_style(look);
        let panel = pst.panel;
        let inner = self.popup_frame(cv, r, Some("Modes"), &pst);
        let limit = inner.x + inner.w;
        // filter line
        let mut x = cv.put(inner.x, inner.y, "  Mode ", prompt, limit);
        x = cv.put(x, inner.y, " ", panel, limit);
        cv.fill(
            Rect {
                x,
                y: inner.y,
                w: limit.saturating_sub(x),
                h: 1,
            },
            input,
        );
        if menu.query.is_empty() {
            cv.put(
                x + 1,
                inner.y,
                "type to filter the modes",
                Style {
                    bg: input.bg,
                    ..dim
                },
                limit,
            );
        } else {
            let e = cv.put(x + 1, inner.y, &menu.query, input, limit);
            cv.put(
                e,
                inner.y,
                "▏",
                Style {
                    bg: input.bg,
                    ..input
                },
                limit,
            );
        }
        let body_h = inner.h.saturating_sub(2) as usize;
        let off = menu.cursor.saturating_sub(body_h.saturating_sub(1));
        for (i, m) in rows.iter().enumerate().skip(off).take(body_h) {
            let y = inner.y + 2 + (i - off) as u16;
            // plain rows on the popup's background: the name in the key colour,
            // the description in the text colour; only the cursor row is lit
            let missing = m.missing();
            let chosen = i == menu.cursor;
            let (ks, ds) = if chosen {
                (pst.cursor, pst.cursor)
            } else {
                (
                    Style {
                        bg: panel.bg,
                        ..key[0]
                    },
                    panel,
                )
            };
            cv.fill(
                Rect {
                    x: inner.x,
                    y,
                    w: inner.w,
                    h: 1,
                },
                ds,
            );
            // ● the mode running now, in the accent colour (apart from the cursor)
            let mark = if m.name == current { "● " } else { "  " };
            let mut x = cv.put(
                inner.x,
                y,
                mark,
                Style {
                    bg: ks.bg,
                    ..prompt
                },
                limit,
            );
            // the name, with the part the filter matched lit up
            let hl = Style {
                bg: ks.bg,
                ..matched
            };
            let q = menu.query.to_lowercase();
            let lower = m.name.to_lowercase();
            let span = (!q.is_empty())
                .then(|| lower.find(&q).map(|p| (p, p + q.len())))
                .flatten();
            let name_x = x;
            for (bi, ch) in m.name.char_indices() {
                let st = if span.is_some_and(|(a, b)| bi >= a && bi < b) {
                    hl
                } else {
                    ks
                };
                let mut buf = [0u8; 4];
                x = cv.put(x, y, ch.encode_utf8(&mut buf), st, limit);
            }
            x = x.max(name_x + name_w);
            let mut d = m.description.clone();
            if !m.builtin {
                d.push_str("  [user]");
            }
            if !missing.is_empty() {
                d.push_str(&format!("  (needs {})", missing.join(", ")));
            }
            let ds = if missing.is_empty() {
                ds
            } else {
                Style { bg: ds.bg, ..dim }
            };
            cv.put(x + 1, y, &d, ds, limit);
        }
        let hint = " ↑↓ choose · enter switch · esc close ";
        cv.put(
            r.x + (r.w.saturating_sub(str_width(hint) as u16)) / 2,
            r.y + r.h - 1,
            hint,
            Style {
                bg: panel.bg,
                ..dim
            },
            r.x + r.w - 1,
        );
    }
}

/// How well `m` matches the menu filter `q` (lower is better), None = not at
/// all: the NAME first (exact, prefix, substring, in-order letters), then
/// the description (a word starting with `q`, then anywhere).
pub(super) fn mode_rank(m: &crate::modes::Mode, q: &str) -> Option<u8> {
    if q.is_empty() {
        return Some(0);
    }
    let name = m.name.to_lowercase();
    if name == q {
        return Some(0);
    }
    if name.starts_with(q) {
        return Some(1);
    }
    if name.split(['-', '_']).any(|part| part.starts_with(q)) {
        return Some(2);
    }
    if name.contains(q) {
        return Some(3);
    }
    let mut it = name.chars();
    if q.chars().all(|c| it.any(|n| n == c)) {
        return Some(4);
    }
    let desc = m.description.to_lowercase();
    if desc
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| w.starts_with(q))
    {
        return Some(5);
    }
    if desc.contains(q) {
        return Some(6);
    }
    None
}

/// Re-run lvim-fuzzy in `mode`, keeping the query and the user's options.
pub(super) fn switch_mode(mode: &str, query: &str) -> Result<i32, String> {
    let mut args = vec![mode.to_string()];
    let pass = crate::PASSTHROUGH.get().cloned().unwrap_or_default();
    let mut pass: Vec<String> = pass
        .into_iter()
        .filter(|a| !a.starts_with("--query") && a != "-q")
        .collect();
    if !query.is_empty() {
        if query.starts_with('-') {
            pass.push(format!("--query={query}"));
        } else {
            args.push(query.to_string());
        }
    }
    args.extend(pass);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = Command::new(crate::self_exe()).args(&args).exec();
        Err(format!("cannot switch to mode {mode}: {err}"))
    }
    #[cfg(not(unix))]
    {
        let st = Command::new(crate::self_exe())
            .args(&args)
            .status()
            .map_err(|e| e.to_string())?;
        Ok(st.code().unwrap_or(1))
    }
}

#[cfg(test)]
mod tests {
    use super::mode_rank;

    #[test]
    fn names_beat_descriptions() {
        let all = crate::modes::builtin();
        let rank = |q: &str| {
            let mut v: Vec<(u8, usize, String)> = all
                .iter()
                .enumerate()
                .filter_map(|(i, m)| mode_rank(m, q).map(|r| (r, i, m.name.clone())))
                .collect();
            v.sort();
            v.into_iter().map(|x| x.2).collect::<Vec<_>>()
        };
        let man = rank("man");
        assert_eq!(man[0], "man");
        assert!(man.iter().position(|x| x == "history") > man.iter().position(|x| x == "commands"));
        assert_eq!(rank("log")[0], "git-log");
        assert_eq!(rank("dock")[0], "docker");
        assert!(rank("zzzz").is_empty());
    }
}
