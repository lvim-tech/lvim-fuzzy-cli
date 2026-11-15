//! Input sources: stdin, a shell command, or the built-in directory walker.

use std::collections::HashSet;
use std::io::Read;
use std::path::Path;
use std::process::Child;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use crate::options::Walker;

pub enum Event {
    Lines(Vec<String>),
    /// Lines already turned into items (and their store) on the reading
    /// thread (index 0; the finder numbers them when it takes them).
    Items(crate::item::Part),
    /// How many items are coming (the list is sized once).
    Reserve(usize),
    Eof,
}

const BATCH: usize = 4096;

/// A sender tagged with the input epoch; `reload` bumps the epoch so lines
/// from the previous source are dropped.
#[derive(Clone)]
pub struct Feed {
    pub tx: Sender<(u64, Event)>,
    pub epoch: u64,
    /// When set, lines are built into items here, on the reading thread, so
    /// the finder's UI thread only appends them (a grep can send 100 000s).
    pub build: Option<Arc<std::sync::RwLock<crate::item::ItemBuilder>>>,
    /// Where a command source registers its process, so a reload (or the
    /// finder's exit) can stop it, the whole process group.
    pub child: Option<Arc<Mutex<Option<Child>>>>,
}

impl Feed {
    pub fn send(&self, e: Event) -> Result<(), ()> {
        let e = match (e, &self.build) {
            (Event::Lines(lines), Some(b)) => {
                let b = b.read().unwrap();
                let mut part = crate::item::Part::new();
                for l in lines {
                    part.push(b.build(0, l));
                }
                Event::Items(part)
            }
            (e, _) => e,
        };
        self.tx.send((self.epoch, e)).map_err(|_| ())
    }
}

/// Read `sep`-delimited records and send them in batches. Reads big blocks
/// and splits them with memchr: one allocation per record, no per-line reads.
pub fn read_stream<R: Read>(mut r: R, sep: u8, tx: &Feed) {
    let mut buf: Vec<u8> = Vec::with_capacity(1 << 20);
    let mut batch = Vec::with_capacity(BATCH);
    let mut eof = false;
    while !eof {
        // fill up to 1 MiB past what is left over from the last block
        let start = buf.len();
        buf.resize(start + (1 << 20), 0);
        let n = loop {
            match r.read(&mut buf[start..]) {
                Ok(n) => break n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break 0,
            }
        };
        buf.truncate(start + n);
        if n == 0 {
            eof = true;
        }
        let mut from = 0;
        // complete records only, unless this is the end
        let upto = if eof {
            buf.len()
        } else {
            memchr::memrchr(sep, &buf).map_or(0, |p| p + 1)
        };
        while from < upto {
            let end = memchr::memchr(sep, &buf[from..upto]).map_or(upto, |p| from + p);
            let mut rec = &buf[from..end];
            if sep == b'\n' && rec.last() == Some(&b'\r') {
                rec = &rec[..rec.len() - 1];
            }
            batch.push(match std::str::from_utf8(rec) {
                Ok(s) => s.to_owned(),
                Err(_) => String::from_utf8_lossy(rec).into_owned(),
            });
            from = end + 1;
            if batch.len() >= BATCH && tx.send(Event::Lines(std::mem::take(&mut batch))).is_err() {
                return;
            }
        }
        buf.drain(..upto.min(buf.len()));
    }
    if !batch.is_empty() {
        let _ = tx.send(Event::Lines(batch));
    }
}

/// Stop a command started by `read_command`: its whole process group.
pub fn stop(c: &mut Child) {
    #[cfg(unix)]
    {
        // SAFETY: kill(2) on the group this child leads (process_group(0)).
        unsafe {
            libc::kill(-(c.id() as libc::pid_t), libc::SIGKILL);
        }
    }
    let _ = c.kill();
    let _ = c.wait();
}

/// Start `cmd` in a process group of its own (stopping it then stops what
/// it runs: `sh -c "rg ..."` takes rg along).
pub fn spawn_in_group(cmd: &mut Command) -> std::io::Result<Child> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn()
}

/// Put `child` in `slot` (a command already there is stopped first); returns
/// its pid for `release`.
pub fn register(slot: &Arc<Mutex<Option<Child>>>, child: Child) -> u32 {
    let pid = child.id();
    let prev = slot.lock().unwrap().replace(child);
    if let Some(mut p) = prev {
        stop(&mut p);
    }
    pid
}

/// The command `pid` is done with its output: take it out of `slot` if it is
/// still there (someone may have stopped it and put the next one in) and
/// wait for it outside the lock.
pub fn release(slot: &Arc<Mutex<Option<Child>>>, pid: u32) {
    let mine = {
        let mut s = slot.lock().unwrap();
        if s.as_ref().is_some_and(|c| c.id() == pid) {
            s.take()
        } else {
            None
        }
    };
    if let Some(mut c) = mine {
        let _ = c.wait();
    }
}

/// Run `cmd` in a group of its own, registered in `slot`, and return what it
/// printed (stopped meanwhile: what came before).
pub fn capture(mut cmd: Command, slot: &Arc<Mutex<Option<Child>>>) -> Option<Vec<u8>> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = spawn_in_group(&mut cmd).ok()?;
    let mut out = child.stdout.take()?;
    let pid = register(slot, child);
    let mut buf = Vec::new();
    let _ = out.read_to_end(&mut buf);
    release(slot, pid);
    Some(buf)
}

/// Stop what a slot holds (the whole process group).
pub fn stop_slot(slot: &Arc<Mutex<Option<Child>>>) {
    let c = slot.lock().unwrap().take();
    if let Some(mut c) = c {
        stop(&mut c);
    }
}

/// Run `cmd` through the shell and stream its stdout.
pub fn read_command(
    shell: &[String],
    cmd: &str,
    sep: u8,
    tx: &Feed,
    slot: Option<Arc<Mutex<Option<Child>>>>,
) {
    let Some((prog, args)) = shell.split_first() else {
        return;
    };
    let mut command = Command::new(prog);
    command
        .args(args)
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = spawn_in_group(&mut command);
    let Ok(mut child) = child else { return };
    let out = child.stdout.take();
    match slot {
        Some(slot) => {
            let pid = register(&slot, child);
            if let Some(out) = out {
                read_stream(out, sep, tx);
            }
            release(&slot, pid);
        }
        None => {
            if let Some(out) = out {
                read_stream(out, sep, tx);
            }
            let _ = child.wait();
        }
    }
}

/// fzf's built-in walker: relative paths, depth first, skipping `skip` names.
pub fn walk(roots: &[String], w: &Walker, skip: &[String], tx: &Feed) {
    let mut batch = Vec::with_capacity(BATCH);
    let mut seen = HashSet::new();
    for root in roots {
        let prefix = if root == "." {
            String::new()
        } else {
            format!("{}/", root.trim_end_matches('/'))
        };
        walk_dir(Path::new(root), &prefix, w, skip, &mut seen, &mut batch, tx);
    }
    if !batch.is_empty() {
        let _ = tx.send(Event::Lines(batch));
    }
}

fn walk_dir(
    dir: &Path,
    prefix: &str,
    w: &Walker,
    skip: &[String],
    seen: &mut HashSet<(u64, u64)>,
    batch: &mut Vec<String>,
    tx: &Feed,
) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(md) = std::fs::metadata(dir)
            && !seen.insert((md.dev(), md.ino()))
        {
            return;
        }
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        if !w.hidden && name.starts_with('.') {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        let path = e.path();
        let is_dir = if ft.is_symlink() {
            w.follow
                && std::fs::metadata(&path)
                    .map(|m| m.is_dir())
                    .unwrap_or(false)
        } else {
            ft.is_dir()
        };
        let rel = format!("{prefix}{name}");
        if is_dir {
            if skip.iter().any(|s| s == &name) {
                continue;
            }
            if w.dir {
                push(batch, rel.clone(), tx);
            }
            walk_dir(&path, &format!("{rel}/"), w, skip, seen, batch, tx);
        } else if w.file {
            push(batch, rel, tx);
        }
    }
}

fn push(batch: &mut Vec<String>, s: String, tx: &Feed) {
    batch.push(s);
    if batch.len() >= BATCH {
        let _ = tx.send(Event::Lines(std::mem::take(batch)));
    }
}
