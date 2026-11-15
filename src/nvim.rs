//! An embedded Neovim (`nvim --embed`) used as the preview: we attach as a UI
//! client (ext_linegrid) and keep its grid, which the finder blits into the
//! preview area. One instance lives for the whole session; each item is
//! opened with `:edit`.

use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

use crossterm::style::{Attribute, Attributes, Color};
use rmpv::Value;

use crate::theme::Style;

#[derive(Clone, Debug, Default)]
pub struct Cell {
    pub text: String,
    pub hl: u64,
}

#[derive(Default)]
pub struct Grid {
    pub w: usize,
    pub h: usize,
    pub cells: Vec<Cell>,
    pub cursor: (usize, usize),
    pub hl: HashMap<u64, Style>,
    pub default_fg: Option<Color>,
    pub default_bg: Option<Color>,
    /// Bumped on every `flush`.
    pub frame: u64,
    pub ready: bool,
    pub exited: bool,
}

impl Grid {
    fn resize(&mut self, w: usize, h: usize) {
        self.w = w;
        self.h = h;
        self.cells = vec![
            Cell {
                text: " ".into(),
                hl: 0
            };
            w * h
        ];
    }

    fn clear(&mut self) {
        for c in self.cells.iter_mut() {
            *c = Cell {
                text: " ".into(),
                hl: 0,
            };
        }
    }

    /// The style of highlight `id` with the default colours filled in.
    pub fn style(&self, id: u64) -> Style {
        let s = self.hl.get(&id).copied().unwrap_or_default();
        Style {
            fg: s.fg.or(self.default_fg),
            bg: s.bg.or(self.default_bg),
            ..s
        }
    }
}

pub struct Nvim {
    child: Child,
    writer: Arc<Mutex<BufWriter<ChildStdin>>>,
    next_id: Arc<Mutex<u64>>,
    /// One preview open at a time: the request in flight, and only the
    /// latest one waiting (a key held down must not queue every file it
    /// passes; Neovim opens each with its filetype, autocmds and plugins).
    opens: Arc<Mutex<Opens>>,
    pub grid: Arc<Mutex<Grid>>,
    attached: (usize, usize),
}

#[derive(Default)]
struct Opens {
    in_flight: Option<u64>,
    waiting: Option<Value>,
}

/// Write one request (`[0, id, method, params]`); returns its id.
fn write_request(
    writer: &Mutex<BufWriter<ChildStdin>>,
    next_id: &Mutex<u64>,
    method: &str,
    params: Value,
) -> u64 {
    let id = {
        let mut n = next_id.lock().unwrap();
        *n += 1;
        *n
    };
    let msg = Value::Array(vec![Value::from(0), Value::from(id), s(method), params]);
    let mut w = writer.lock().unwrap();
    let _ = rmpv::encode::write_value(&mut *w, &msg);
    let _ = w.flush();
    id
}

fn rgb(v: &Value) -> Option<Color> {
    let n = v.as_i64()?;
    if n < 0 {
        return None;
    }
    Some(Color::Rgb {
        r: (n >> 16) as u8,
        g: (n >> 8) as u8,
        b: n as u8,
    })
}

fn s(v: &str) -> Value {
    Value::from(v)
}

impl Nvim {
    /// Start `nvim --embed`. `program` may carry arguments ("nvim -u NONE").
    pub fn start(program: &str, appname: Option<&str>, wake: Sender<()>) -> std::io::Result<Nvim> {
        let words = crate::options::shell_words(program).unwrap_or_else(|_| vec!["nvim".into()]);
        let (prog, args) = words
            .split_first()
            .map(|(p, a)| (p.clone(), a.to_vec()))
            .unwrap_or(("nvim".into(), vec![]));
        let mut cmd = Command::new(prog);
        // mark the instance so plugins can tell it is the finder's preview
        // (lvim-hud hides its tabline there: the finder shows the file name itself)
        cmd.args(args)
            .args([
                "--cmd",
                "let g:lvim_fuzzy_preview = 1",
                "--embed",
                "-n",
                "-i",
                "NONE",
            ])
            .env("LVIM_FUZZY_PREVIEW", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(a) = appname {
            cmd.env("NVIM_APPNAME", a);
        }
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let grid = Arc::new(Mutex::new(Grid::default()));
        let g = grid.clone();
        let writer = Arc::new(Mutex::new(BufWriter::new(stdin)));
        let next_id = Arc::new(Mutex::new(1u64));
        let opens: Arc<Mutex<Opens>> = Arc::new(Mutex::new(Opens::default()));
        let (w2, n2, o2) = (writer.clone(), next_id.clone(), opens.clone());
        thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            while let Ok(msg) = rmpv::decode::read_value(&mut r) {
                // a response ([1, id, error, result]) to the open in flight:
                // the waiting one (the latest target) goes next
                if let Some(arr) = msg.as_array()
                    && arr.first().and_then(Value::as_u64) == Some(1)
                {
                    let id = arr.get(1).and_then(Value::as_u64);
                    let mut o = o2.lock().unwrap();
                    if id.is_some() && o.in_flight == id {
                        o.in_flight = o
                            .waiting
                            .take()
                            .map(|p| write_request(&w2, &n2, "nvim_exec_lua", p));
                    }
                    continue;
                }
                handle(&msg, &g, &wake);
            }
            g.lock().unwrap().exited = true;
            let _ = wake.send(());
        });
        Ok(Nvim {
            child,
            writer,
            next_id,
            opens,
            grid,
            attached: (0, 0),
        })
    }

    fn send(&self, method: &str, params: Vec<Value>) {
        write_request(&self.writer, &self.next_id, method, Value::Array(params));
    }

    /// Attach (first call) or resize the UI to `w` x `h`.
    pub fn size(&mut self, w: usize, h: usize) {
        if w == 0 || h == 0 || self.attached == (w, h) {
            return;
        }
        if self.attached == (0, 0) {
            let opts = Value::Map(vec![
                (s("rgb"), Value::from(true)),
                (s("ext_linegrid"), Value::from(true)),
            ]);
            self.send(
                "nvim_ui_attach",
                vec![Value::from(w as u64), Value::from(h as u64), opts],
            );
            // Nothing is set in the instance from here: how the preview looks is up to the
            // user's config and plugins, which see g:lvim_fuzzy_preview.
        } else {
            self.send(
                "nvim_ui_try_resize",
                vec![Value::from(w as u64), Value::from(h as u64)],
            );
        }
        self.attached = (w, h);
    }

    /// Show `path` at `line` (1-based, 0 = keep), or `text` in a scratch buffer
    /// when `path` is not a file.
    pub fn open(&self, path: &str, line: usize, col: usize, text: &str, query: &str) {
        // How the preview looks lives in Neovim: the lvim-fuzzy plugin's
        // `lvim-fuzzy.preview` (opens the target, marks the query). Without that
        // plugin only the target is opened.
        const CODE: &str = r#"
local path, line, col, text, query = ...
local ok, pv = pcall(require, "lvim-fuzzy.preview")
if ok and type(pv) == "table" and pv.show then
  pv.show({ path = path, line = line, col = col, text = text, query = query })
  return
end
local prev = vim.api.nvim_get_current_buf()
local ok = false
if path ~= "" and vim.fn.filereadable(path) == 1 then
  ok = pcall(vim.cmd, "silent keepalt edit " .. vim.fn.fnameescape(path))
end
if not ok then
  local b = vim.api.nvim_create_buf(false, true)
  vim.bo[b].bufhidden = "wipe"
  vim.api.nvim_buf_set_lines(b, 0, -1, false, vim.split(text, "\n", { plain = true }))
  vim.api.nvim_set_current_buf(b)
end
local cur = vim.api.nvim_get_current_buf()
if prev ~= cur and vim.api.nvim_buf_is_valid(prev) and not vim.bo[prev].modified
   and vim.fn.bufwinnr(prev) == -1 and vim.b[prev].lvim_fuzzy_keep == nil then
  pcall(vim.api.nvim_buf_delete, prev, { force = false })
end
local n = vim.api.nvim_buf_line_count(cur)
if line > 0 then
  line = math.min(line, n)
  pcall(vim.api.nvim_win_set_cursor, 0, { line, math.max(col - 1, 0) })
  vim.cmd("normal! zz")
else
  pcall(vim.api.nvim_win_set_cursor, 0, { 1, 0 })
end
"#;
        let params = Value::Array(vec![
            s(CODE),
            Value::Array(vec![
                s(path),
                Value::from(line as u64),
                Value::from(col as u64),
                s(text),
                s(query),
            ]),
        ]);
        // one open at a time; meanwhile only the latest target waits
        let mut o = self.opens.lock().unwrap();
        if o.in_flight.is_some() {
            o.waiting = Some(params);
        } else {
            o.in_flight = Some(write_request(
                &self.writer,
                &self.next_id,
                "nvim_exec_lua",
                params,
            ));
        }
    }

    /// Send keys in Neovim's notation (`<C-d>`, `gg`, ...).
    pub fn input(&self, keys: &str) {
        self.send("nvim_input", vec![s(keys)]);
    }

    pub fn mouse(&self, button: &str, action: &str, modifier: &str, row: usize, col: usize) {
        self.send(
            "nvim_input_mouse",
            vec![
                s(button),
                s(action),
                s(modifier),
                Value::from(0),
                Value::from(row as u64),
                Value::from(col as u64),
            ],
        );
    }

    pub fn quit(mut self) {
        self.send("nvim_command", vec![s("qa!")]);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn handle(msg: &Value, grid: &Arc<Mutex<Grid>>, wake: &Sender<()>) {
    let Some(arr) = msg.as_array() else { return };
    // notification: [2, method, params]
    if arr.first().and_then(Value::as_u64) != Some(2) {
        return;
    }
    if arr.get(1).and_then(Value::as_str) != Some("redraw") {
        return;
    }
    let Some(batches) = arr.get(2).and_then(Value::as_array) else {
        return;
    };
    let mut g = grid.lock().unwrap();
    let mut flushed = false;
    for ev in batches {
        let Some(ev) = ev.as_array() else { continue };
        let Some(name) = ev.first().and_then(Value::as_str) else {
            continue;
        };
        for args in &ev[1..] {
            let Some(a) = args.as_array() else { continue };
            match name {
                "grid_resize" => {
                    let w = a.get(1).and_then(Value::as_u64).unwrap_or(0) as usize;
                    let h = a.get(2).and_then(Value::as_u64).unwrap_or(0) as usize;
                    g.resize(w, h);
                }
                "default_colors_set" => {
                    g.default_fg = a.first().and_then(rgb);
                    g.default_bg = a.get(1).and_then(rgb);
                }
                "hl_attr_define" => {
                    let id = a.first().and_then(Value::as_u64).unwrap_or(0);
                    let mut st = Style::default();
                    let mut reverse = false;
                    if let Some(m) = a.get(1).and_then(Value::as_map) {
                        let mut attrs = Attributes::default();
                        for (k, v) in m {
                            match k.as_str().unwrap_or("") {
                                "foreground" => st.fg = rgb(v),
                                "background" => st.bg = rgb(v),
                                "bold" if v.as_bool() == Some(true) => attrs.set(Attribute::Bold),
                                "italic" if v.as_bool() == Some(true) => {
                                    attrs.set(Attribute::Italic)
                                }
                                "underline" | "undercurl" if v.as_bool() == Some(true) => {
                                    attrs.set(Attribute::Underlined)
                                }
                                "strikethrough" if v.as_bool() == Some(true) => {
                                    attrs.set(Attribute::CrossedOut)
                                }
                                "reverse" if v.as_bool() == Some(true) => reverse = true,
                                _ => {}
                            }
                        }
                        st.attrs = attrs;
                    }
                    if reverse {
                        let fg = st.fg.or(g.default_fg);
                        st.fg = st.bg.or(g.default_bg);
                        st.bg = fg;
                    }
                    g.hl.insert(id, st);
                }
                "grid_clear" => g.clear(),
                "grid_cursor_goto" => {
                    let r = a.get(1).and_then(Value::as_u64).unwrap_or(0) as usize;
                    let c = a.get(2).and_then(Value::as_u64).unwrap_or(0) as usize;
                    g.cursor = (r, c);
                }
                "grid_line" => {
                    let row = a.get(1).and_then(Value::as_u64).unwrap_or(0) as usize;
                    let mut col = a.get(2).and_then(Value::as_u64).unwrap_or(0) as usize;
                    let Some(cells) = a.get(3).and_then(Value::as_array) else {
                        continue;
                    };
                    let mut hl = 0u64;
                    if row >= g.h {
                        continue;
                    }
                    let w = g.w;
                    for cell in cells {
                        let Some(c) = cell.as_array() else { continue };
                        let text = c.first().and_then(Value::as_str).unwrap_or(" ").to_string();
                        if let Some(h) = c.get(1).and_then(Value::as_u64) {
                            hl = h;
                        }
                        let rep = c.get(2).and_then(Value::as_u64).unwrap_or(1) as usize;
                        for _ in 0..rep {
                            if col < w {
                                g.cells[row * w + col] = Cell {
                                    text: text.clone(),
                                    hl,
                                };
                            }
                            col += 1;
                        }
                    }
                }
                "grid_scroll" => {
                    let top = a.get(1).and_then(Value::as_u64).unwrap_or(0) as usize;
                    let bot = a.get(2).and_then(Value::as_u64).unwrap_or(0) as usize;
                    let left = a.get(3).and_then(Value::as_u64).unwrap_or(0) as usize;
                    let right = a.get(4).and_then(Value::as_u64).unwrap_or(0) as usize;
                    let rows = a.get(5).and_then(Value::as_i64).unwrap_or(0);
                    let w = g.w;
                    let bot = bot.min(g.h);
                    let right = right.min(w);
                    if rows > 0 {
                        let rows = rows as usize;
                        for r in top..bot.saturating_sub(rows) {
                            for c in left..right {
                                g.cells[r * w + c] = g.cells[(r + rows) * w + c].clone();
                            }
                        }
                    } else if rows < 0 {
                        let rows = (-rows) as usize;
                        for r in (top + rows..bot).rev() {
                            for c in left..right {
                                g.cells[r * w + c] = g.cells[(r - rows) * w + c].clone();
                            }
                        }
                    }
                }
                "flush" => flushed = true,
                _ => {}
            }
        }
    }
    if flushed {
        g.frame += 1;
        g.ready = true;
        drop(g);
        let _ = wake.send(());
    }
}

/// Translate a terminal key into Neovim's key notation.
pub fn key_notation(e: &crossterm::event::KeyEvent) -> Option<String> {
    use crossterm::event::{KeyCode, KeyModifiers};
    let m = e.modifiers;
    let mut pre = String::new();
    if m.contains(KeyModifiers::CONTROL) {
        pre.push_str("C-");
    }
    if m.contains(KeyModifiers::ALT) {
        pre.push_str("M-");
    }
    let shift = m.contains(KeyModifiers::SHIFT);
    let named = |n: &str, pre: &str| -> String {
        let s = if shift {
            format!("S-{pre}")
        } else {
            pre.to_string()
        };
        format!("<{s}{n}>")
    };
    Some(match e.code {
        KeyCode::Char(c) => {
            if pre.is_empty() {
                if c == '<' {
                    "<lt>".into()
                } else {
                    c.to_string()
                }
            } else {
                let c = if c == '<' {
                    "lt".to_string()
                } else {
                    c.to_string()
                };
                format!("<{pre}{c}>")
            }
        }
        KeyCode::Enter => named("CR", &pre),
        KeyCode::Esc => named("Esc", &pre),
        KeyCode::Backspace => named("BS", &pre),
        KeyCode::Tab => named("Tab", &pre),
        KeyCode::BackTab => "<S-Tab>".into(),
        KeyCode::Delete => named("Del", &pre),
        KeyCode::Insert => named("Insert", &pre),
        KeyCode::Up => named("Up", &pre),
        KeyCode::Down => named("Down", &pre),
        KeyCode::Left => named("Left", &pre),
        KeyCode::Right => named("Right", &pre),
        KeyCode::Home => named("Home", &pre),
        KeyCode::End => named("End", &pre),
        KeyCode::PageUp => named("PageUp", &pre),
        KeyCode::PageDown => named("PageDown", &pre),
        KeyCode::F(n) => named(&format!("F{n}"), &pre),
        _ => return None,
    })
}

/// Split a preview target like `file:12:3:text` into (file, line, col).
pub fn parse_target(s: &str) -> (String, usize, usize) {
    let s = s.trim();
    if std::path::Path::new(s).exists() {
        return (s.to_string(), 0, 0);
    }
    let mut parts = s.splitn(4, ':');
    let file = parts.next().unwrap_or("").to_string();
    let line = parts
        .next()
        .and_then(|p| p.trim().parse().ok())
        .unwrap_or(0);
    let col = parts
        .next()
        .and_then(|p| p.trim().parse().ok())
        .unwrap_or(0);
    (file, line, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs `nvim` on PATH: cargo test -- --ignored
    #[test]
    #[ignore]
    fn embedded_nvim_renders_a_file() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut n = Nvim::start("nvim -u NONE", None, tx).unwrap();
        n.size(60, 8);
        n.open("Cargo.toml", 2, 1, "", "");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let _ = rx.recv_timeout(std::time::Duration::from_millis(100));
            let g = n.grid.lock().unwrap();
            let text: String = g.cells.iter().map(|c| c.text.as_str()).collect();
            if text.contains("lvim-fuzzy-cli") {
                assert_eq!((g.w, g.h), (60, 8));
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no file text in the grid: {text}"
            );
        }
        n.quit();
    }

    #[test]
    fn targets() {
        assert_eq!(
            parse_target("src/x.rs:12:3:fn main"),
            ("src/x.rs".into(), 12, 3)
        );
        assert_eq!(parse_target("a.lua:7"), ("a.lua".into(), 7, 0));
        assert_eq!(parse_target("plain"), ("plain".into(), 0, 0));
    }
}
