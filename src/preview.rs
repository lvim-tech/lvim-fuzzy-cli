//! The preview window: `--preview-window` grammar and the background runner.

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::options::BorderStyle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pos {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub pos: Pos,
    pub size: u16,
    pub percent: bool,
    pub wrap: bool,
    pub wrap_word: bool,
    pub cycle: bool,
    pub follow: bool,
    pub info: bool,
    pub hidden: bool,
    pub border: BorderStyle,
    pub scroll: Option<String>,
    pub header_lines: usize,
    /// `<THRESHOLD(ALT)`: use `alt` when the finder is narrower (left/right)
    /// or shorter (up/down) than the threshold.
    pub alt: Option<(u16, Box<Spec>)>,
}

impl Default for Spec {
    fn default() -> Self {
        Spec {
            pos: Pos::Right,
            size: 50,
            percent: true,
            wrap: false,
            wrap_word: false,
            cycle: false,
            follow: false,
            info: true,
            hidden: false,
            border: BorderStyle::Rounded,
            scroll: None,
            header_lines: 0,
            alt: None,
        }
    }
}

impl Spec {
    /// Apply one `--preview-window` value on top of `self`.
    pub fn apply(&mut self, s: &str) -> Result<(), String> {
        // the alternative layout may contain commas: split it off first
        let (main, alt) = match s.find('<') {
            Some(p) => (&s[..p], Some(&s[p..])),
            None => (s, None),
        };
        for tok in main
            .split([',', ':'])
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            self.token(tok)?;
        }
        if let Some(a) = alt {
            let open = a
                .find('(')
                .ok_or_else(|| format!("invalid preview window option: {a}"))?;
            let close = a
                .rfind(')')
                .ok_or_else(|| format!("invalid preview window option: {a}"))?;
            let n: u16 = a[1..open]
                .parse()
                .map_err(|_| format!("invalid preview window option: {a}"))?;
            let mut alt_spec = Spec {
                alt: None,
                ..self.clone()
            };
            alt_spec.apply(&a[open + 1..close])?;
            self.alt = Some((n, Box::new(alt_spec)));
        }
        Ok(())
    }

    fn token(&mut self, t: &str) -> Result<(), String> {
        match t {
            "up" | "top" => self.pos = Pos::Up,
            "down" | "bottom" => self.pos = Pos::Down,
            "left" => self.pos = Pos::Left,
            "right" => self.pos = Pos::Right,
            "wrap" => self.wrap = true,
            "wrap-word" => {
                self.wrap = true;
                self.wrap_word = true;
            }
            "nowrap" => {
                self.wrap = false;
                self.wrap_word = false;
            }
            "cycle" => self.cycle = true,
            "nocycle" => self.cycle = false,
            "follow" => self.follow = true,
            "nofollow" => self.follow = false,
            "info" => self.info = true,
            "noinfo" => self.info = false,
            "hidden" => self.hidden = true,
            "nohidden" => self.hidden = false,
            "default" => *self = Spec::default(),
            "border" => self.border = BorderStyle::Rounded,
            "noborder" => self.border = BorderStyle::None,
            _ => {
                if let Some(b) = t.strip_prefix("border-") {
                    self.border = match b {
                        "inline" => BorderStyle::Line,
                        _ => BorderStyle::parse(b)
                            .ok_or_else(|| format!("invalid preview window option: {t}"))?,
                    };
                } else if t.starts_with('+') {
                    self.scroll = Some(t.to_string());
                } else if let Some(n) = t.strip_prefix('~') {
                    self.header_lines = n
                        .parse()
                        .map_err(|_| format!("invalid preview window option: {t}"))?;
                } else {
                    let (num, pct) = match t.strip_suffix('%') {
                        Some(n) => (n, true),
                        None => (t, false),
                    };
                    let v: u16 = num
                        .parse()
                        .map_err(|_| format!("invalid preview window option: {t}"))?;
                    if pct && v > 100 {
                        return Err(format!("invalid preview window option: {t}"));
                    }
                    self.size = v;
                    self.percent = pct;
                }
            }
        }
        Ok(())
    }

    /// The spec in effect for a finder of `w` x `h` cells.
    pub fn resolve(&self, w: u16, h: u16) -> &Spec {
        if let Some((n, alt)) = &self.alt {
            let dim = match self.pos {
                Pos::Left | Pos::Right => w,
                Pos::Up | Pos::Down => h,
            };
            if dim < *n {
                return alt;
            }
        }
        self
    }
}

/// Parse `+SCROLL[OFFSETS][/DENOM]` after placeholders were expanded:
/// returns the 0-based first visible line.
pub fn scroll_offset(expr: &str, height: usize) -> usize {
    let e = expr.trim().trim_start_matches('+');
    let (body, denom) = match e.split_once('/') {
        Some((b, d)) => (b, d.trim().parse::<i64>().ok().filter(|&d| d > 0)),
        None => (e, None),
    };
    // body: NUM followed by optional +NUM / -NUM terms
    let mut total: i64 = 0;
    let mut first = true;
    let mut cur = String::new();
    let mut sign = 1i64;
    let flush = |cur: &mut String, sign: i64, first: bool, total: &mut i64| {
        if let Ok(v) = cur.trim().trim_matches('\'').parse::<i64>() {
            *total += sign * v;
        }
        let _ = first;
        cur.clear();
    };
    for c in body.chars() {
        if (c == '+' || c == '-') && !cur.trim().is_empty() {
            flush(&mut cur, sign, first, &mut total);
            first = false;
            sign = if c == '-' { -1 } else { 1 };
        } else {
            cur.push(c);
        }
    }
    flush(&mut cur, sign, first, &mut total);
    let mut off = total - 1;
    if let Some(d) = denom {
        off -= height as i64 / d;
    }
    off.max(0) as usize
}

pub struct Output {
    pub version: u64,
    pub lines: Vec<String>,
    pub done: bool,
}

struct Job {
    version: u64,
    cmd: String,
    env: Vec<(String, String)>,
}

/// Runs preview commands in the background; a new job kills the previous one.
pub struct Runner {
    tx: Sender<Job>,
    pub output: Arc<Mutex<Output>>,
    /// The running preview command (stopped by the next job, or on exit).
    child: Arc<Mutex<Option<Child>>>,
}

const MAX_LINES: usize = 100_000;

impl Runner {
    /// Stop the running preview command (the finder is closing).
    pub fn stop(&self) {
        crate::reader::stop_slot(&self.child);
    }

    pub fn new(shell: Vec<String>, wake: Sender<()>) -> Runner {
        let (tx, rx): (Sender<Job>, Receiver<Job>) = channel();
        let output = Arc::new(Mutex::new(Output {
            version: 0,
            lines: Vec::new(),
            done: true,
        }));
        let out = output.clone();
        let current: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let child_slot = current.clone();
        thread::spawn(move || {
            let child = &child_slot;
            while let Ok(mut job) = rx.recv() {
                while let Ok(j) = rx.try_recv() {
                    job = j;
                }
                // the previous preview command, all of it (bat, git, ...)
                crate::reader::stop_slot(child);
                {
                    let mut o = out.lock().unwrap();
                    o.version = job.version;
                    o.lines.clear();
                    o.done = false;
                }
                let Some((prog, args)) = shell.split_first() else {
                    continue;
                };
                let mut command = Command::new(prog);
                command
                    .args(args)
                    .arg(&job.cmd)
                    .envs(job.env.iter().map(|(k, v)| (k, v)))
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                // a group of its own: stopping it stops what the shell runs
                let spawned = crate::reader::spawn_in_group(&mut command);
                let mut c = match spawned {
                    Ok(c) => c,
                    Err(e) => {
                        let mut o = out.lock().unwrap();
                        o.lines.push(format!("cannot run preview command: {e}"));
                        o.done = true;
                        let _ = wake.send(());
                        continue;
                    }
                };
                let pumps: Vec<_> = [
                    c.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
                    c.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
                ]
                .into_iter()
                .flatten()
                .map(|r| {
                    let out = out.clone();
                    let wake = wake.clone();
                    let v = job.version;
                    thread::spawn(move || pump(r, v, &out, &wake))
                })
                .collect();
                let pid = crate::reader::register(child, c);
                // wait for this job's output in a helper so a newer job can
                // still kill the child
                let out2 = out.clone();
                let wake2 = wake.clone();
                let v = job.version;
                let slot = child.clone();
                thread::spawn(move || {
                    for p in pumps {
                        let _ = p.join();
                    }
                    // its output is over: reaped (unless a newer job stopped it)
                    crate::reader::release(&slot, pid);
                    let mut o = out2.lock().unwrap();
                    if o.version == v {
                        o.done = true;
                    }
                    drop(o);
                    let _ = wake2.send(());
                });
                job.cmd.clear();
            }
        });
        Runner {
            tx,
            output,
            child: current,
        }
    }

    pub fn run(&self, version: u64, cmd: String, env: Vec<(String, String)>) {
        let _ = self.tx.send(Job { version, cmd, env });
    }
}

fn pump(r: Box<dyn Read + Send>, version: u64, out: &Arc<Mutex<Output>>, wake: &Sender<()>) {
    let mut br = BufReader::new(r);
    let mut buf = Vec::new();
    let mut pending = Vec::new();
    let mut last = std::time::Instant::now();
    loop {
        buf.clear();
        match br.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if buf.last() == Some(&b'\n') {
                    buf.pop();
                }
                pending.push(String::from_utf8_lossy(&buf).into_owned());
                if last.elapsed() > std::time::Duration::from_millis(30) {
                    if !flush(&mut pending, version, out) {
                        return;
                    }
                    let _ = wake.send(());
                    last = std::time::Instant::now();
                }
            }
        }
    }
    flush(&mut pending, version, out);
    let _ = wake.send(());
}

/// Append pending lines; false when the job was superseded.
fn flush(pending: &mut Vec<String>, version: u64, out: &Arc<Mutex<Output>>) -> bool {
    let mut o = out.lock().unwrap();
    if o.version != version {
        return false;
    }
    let room = MAX_LINES.saturating_sub(o.lines.len());
    o.lines.extend(pending.drain(..).take(room));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec() {
        let mut s = Spec::default();
        s.apply("up,40%,wrap,border-sharp,+{2}-5,~3").unwrap();
        assert_eq!(
            (s.pos, s.size, s.percent, s.wrap),
            (Pos::Up, 40, true, true)
        );
        assert_eq!(s.border, BorderStyle::Sharp);
        assert_eq!(s.scroll.as_deref(), Some("+{2}-5"));
        assert_eq!(s.header_lines, 3);
        let mut s = Spec::default();
        s.apply("right:60%:hidden,<80(down,30%)").unwrap();
        assert!(s.hidden);
        assert_eq!(s.resolve(70, 30).pos, Pos::Down);
        assert_eq!(s.resolve(120, 30).pos, Pos::Right);
        assert!(Spec::default().apply("sideways").is_err());
    }

    #[test]
    fn scroll() {
        assert_eq!(scroll_offset("+10", 20), 9);
        assert_eq!(scroll_offset("+10-5", 20), 4);
        assert_eq!(scroll_offset("+10/2", 20), 0);
        assert_eq!(scroll_offset("+30/2", 20), 19);
        assert_eq!(scroll_offset("+'42'-3", 20), 38);
    }
}
