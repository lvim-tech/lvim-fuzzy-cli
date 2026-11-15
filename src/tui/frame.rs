//! The one frame every popup of the finder is drawn in: notifications, the
//! help (F1), the modes (F2) and the themes (F5).
//!
//! The border is eight strings in Neovim's order — top-left, top, top-right,
//! right, bottom-right, bottom, bottom-left, left — set with `--popup-border`
//! or `[popup] border` in the config; an empty string leaves that side out.
//! The default is eight spaces: no visible line, a cell of the popup's
//! background around the content. The border takes its colour from the theme;
//! the background is the theme's (the terminal's for `terminal`), a step
//! lighter on a dark one or darker on a light one.

use crossterm::style::Color;

use super::App;
use crate::canvas::{Canvas, Rect, str_width};
use crate::theme::{Slot, Style};

/// The colours of a popup.
#[derive(Clone, Copy)]
pub(super) struct PopupStyle {
    /// Background (and the default text colour).
    pub panel: Style,
    pub border: Style,
    pub title: Style,
    /// The cursor row of a popup list: the selection colour, clearly off the
    /// popup's background.
    pub cursor: Style,
}

impl App {
    pub(super) fn popup_style(&self, look: Option<&crate::lvim::Look>) -> PopupStyle {
        let look = look.or(self.look.as_ref());
        let t = &self.theme;
        // the current theme's background a step off (it follows a live theme
        // change); the terminal's own for the `terminal` theme, which has none
        let bg = look
            .and_then(|l| l.panel.bg)
            .map(crate::lvim::shade)
            .or(self.popup_bg)
            .or(t.get(Slot::Bg).bg.map(crate::lvim::shade))
            .or(Some(Color::Reset));
        let (fg, border, title) = match look {
            Some(l) => (l.popup, l.popup_border, l.title),
            None => (t.get(Slot::Fg), t.get(Slot::Border), t.get(Slot::Label)),
        };
        let sel = match look {
            Some(l) => l.sel[0],
            None => t.get(Slot::CurFg),
        };
        let cursor_bg = match (sel.fg, bg) {
            (Some(f @ Color::Rgb { .. }), Some(b @ Color::Rgb { .. })) => {
                Some(crate::lvim::blend(f, b, 0.22))
            }
            _ => sel.bg.or(t.get(Slot::CurBg).bg),
        };
        PopupStyle {
            panel: Style { bg, ..fg },
            border: Style { bg, ..border },
            title: Style { bg, ..title },
            cursor: Style {
                bg: cursor_bg,
                ..sel
            },
        }
    }

    /// The border's thickness on each side: (top, right, bottom, left).
    pub(super) fn popup_sides(&self) -> (u16, u16, u16, u16) {
        let b = &self.opts.popup_border;
        let on = |i: usize| u16::from(!b[i].is_empty());
        (on(1), on(3), on(5), on(7))
    }

    /// The outer size for content `w` x `h` (with one cell of padding left
    /// and right inside the border).
    pub(super) fn popup_outer(&self, w: u16, h: u16) -> (u16, u16) {
        let (t, r, b, l) = self.popup_sides();
        (w + 2 + l + r, h + t + b)
    }

    /// Fill `r`, draw the border and the title (centred on the top border, or
    /// on the first row without one); returns the content area.
    pub(super) fn popup_frame(
        &self,
        cv: &mut Canvas,
        r: Rect,
        title: Option<&str>,
        st: &PopupStyle,
    ) -> Rect {
        cv.fill(r, st.panel);
        let (t, rt, b, l) = self.popup_sides();
        let chars: [&str; 8] = std::array::from_fn(|i| self.opts.popup_border[i].as_str());
        cv.border(r, &chars, [t > 0, rt > 0, b > 0, l > 0], st.border);
        let mut inner = Rect {
            x: r.x + l + 1,
            y: r.y + t,
            w: r.w.saturating_sub(l + rt + 2),
            h: r.h.saturating_sub(t + b),
        };
        if let Some(title) = title.filter(|s| !s.is_empty()) {
            if t == 0 {
                // no top border: the title takes the first row
                inner.y += 1;
                inner.h = inner.h.saturating_sub(1);
            }
            let label = format!(" {title} ");
            let x = r.x + r.w.saturating_sub(str_width(&label) as u16) / 2;
            cv.put(x, r.y, &label, st.title, r.x + r.w);
        }
        inner
    }
}
