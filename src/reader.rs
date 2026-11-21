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

/// fzf's built-in walker: relative paths, skipping `skip` names.
///
/// PARALLEL, as fzf's: each directory is a task on a pool of walker threads, so
/// many directories are read at once (a single thread waiting on one `read_dir`
/// after another walked a home directory in ~5.8 s where fzf took ~3.7 s). The
/// order is per directory (sorted entries), not global — fzf's is not either.
///
/// A directory reached through several paths (symlinks) is listed ONCE, and
/// always under the same path, whichever thread gets there first:
/// - a symlink to a directory the walk reaches by its real path anyway is not
///   followed (the real path is the name it is listed under);
/// - symlinks that lead OUTSIDE the walk are followed after it, in path order,
///   each target once (a target inside one already walked is skipped).
///
/// `w.dup` walks a directory again under every path that reaches it, as fzf
/// does — only a loop (a directory inside itself) is cut.
pub fn walk(roots: &[String], w: &Walker, skip: &[String], tx: &Feed) {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 16);
    let canon_roots: Vec<std::path::PathBuf> = roots
        .iter()
        .filter_map(|r| std::fs::canonicalize(r).ok())
        .collect();
    let ctx = WalkCtx {
        w,
        skip,
        tx,
        roots: canon_roots,
        seen: Mutex::new(HashSet::new()),
        deferred: Mutex::new(Vec::new()),
        stop: std::sync::atomic::AtomicBool::new(false),
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|i| format!("lvf-walk-{i}"))
        .build()
        .ok();
    let run = |jobs: Vec<(std::path::PathBuf, String)>| match &pool {
        Some(pool) => pool.scope(|s| {
            for (dir, prefix) in &jobs {
                let ctx = &ctx;
                s.spawn(move |s| walk_dir(ctx, Some(s), dir, prefix, &[]));
            }
        }),
        None => {
            for (dir, prefix) in &jobs {
                walk_dir(&ctx, None, dir, prefix, &[]);
            }
        }
    };
    run(roots
        .iter()
        .map(|r| (std::path::PathBuf::from(r), root_prefix(r)))
        .collect());
    // the symlinks that lead outside, level by level, in path order: each target once
    let mut walked: Vec<std::path::PathBuf> = Vec::new();
    loop {
        let mut next = std::mem::take(&mut *ctx.deferred.lock().unwrap());
        if next.is_empty() || ctx.stop.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        next.sort_by(|a, b| a.0.cmp(&b.0));
        let mut jobs = Vec::new();
        for (rel, target) in next {
            if walked.iter().any(|t| target.starts_with(t)) {
                continue; // inside a target already walked (under its first path)
            }
            if ctx.w.dir {
                emit(&ctx, rel.clone());
            }
            walked.push(target.clone());
            jobs.push((target, format!("{rel}/")));
        }
        // a target that CONTAINS one walked earlier in this level is still walked: its inner
        // directory is then skipped by the inode check below it
        run(jobs);
    }
    // each walker thread holds its last partial batch (and this one, the deferred directories)
    if let Some(pool) = &pool {
        pool.broadcast(|_| flush_local(tx));
    }
    flush_local(tx);
}

fn root_prefix(root: &str) -> String {
    if root == "." {
        String::new()
    } else {
        format!("{}/", root.trim_end_matches('/'))
    }
}

struct WalkCtx<'a> {
    w: &'a Walker,
    skip: &'a [String],
    tx: &'a Feed,
    /// The roots' real paths: a symlink into them is not followed.
    roots: Vec<std::path::PathBuf>,
    /// The directories walked so far (device, inode); unused with `w.dup`.
    seen: Mutex<HashSet<(u64, u64)>>,
    /// Symlinks to directories outside the roots: (listed path, real target),
    /// walked after the roots.
    deferred: Mutex<Vec<(String, std::path::PathBuf)>>,
    /// The finder is gone (or reloaded): stop walking.
    stop: std::sync::atomic::AtomicBool,
}

impl WalkCtx<'_> {
    /// Whether the walk reaches `target` (a real path) by itself: it lies in a
    /// root, and no directory on the way is skipped (a `skip` name, or hidden
    /// without `hidden`).
    fn reaches(&self, target: &Path) -> bool {
        self.roots.iter().any(|r| {
            target.strip_prefix(r).is_ok_and(|rest| {
                rest.components().all(|c| {
                    let n = c.as_os_str().to_string_lossy();
                    !self.skip.iter().any(|s| *s == n) && (self.w.hidden || !n.starts_with('.'))
                })
            })
        })
    }
}

thread_local! {
    /// The walker thread's batch of paths, sent when full and at the end.
    static LOCAL: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn emit(ctx: &WalkCtx, s: String) {
    let full = LOCAL.with(|b| {
        let mut b = b.borrow_mut();
        b.push(s);
        (b.len() >= BATCH).then(|| std::mem::take(&mut *b))
    });
    if let Some(batch) = full
        && ctx.tx.send(Event::Lines(batch)).is_err()
    {
        ctx.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

fn flush_local(tx: &Feed) {
    let rest = LOCAL.with(|b| std::mem::take(&mut *b.borrow_mut()));
    if !rest.is_empty() {
        let _ = tx.send(Event::Lines(rest));
    }
}

/// Walk one directory: its files go out, its subdirectories become tasks of
/// their own (or are walked here, without a pool). `ancestors` is the chain of
/// directories above this one, for cutting loops when every path is walked.
fn walk_dir<'s>(
    ctx: &'s WalkCtx<'s>,
    scope: Option<&rayon::Scope<'s>>,
    dir: &Path,
    prefix: &str,
    ancestors: &[(u64, u64)],
) {
    if ctx.stop.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let mut chain: Vec<(u64, u64)> = Vec::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(md) = std::fs::metadata(dir) {
            let id = (md.dev(), md.ino());
            if ctx.w.dup {
                if ancestors.contains(&id) {
                    return; // a loop: this directory contains itself
                }
                chain = ancestors.to_vec();
                chain.push(id);
            } else if !ctx.seen.lock().unwrap().insert(id) {
                return;
            }
        }
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    let chain = std::sync::Arc::new(chain);
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        if !ctx.w.hidden && name.starts_with('.') {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        let path = e.path();
        let is_dir = if ft.is_symlink() {
            ctx.w.follow
                && std::fs::metadata(&path)
                    .map(|m| m.is_dir())
                    .unwrap_or(false)
        } else {
            ft.is_dir()
        };
        let rel = format!("{prefix}{name}");
        if is_dir {
            if ctx.skip.iter().any(|s| s == &name) {
                continue;
            }
            // a symlinked directory: by its real path when the walk reaches that anyway, else after
            // the walk, in path order (see `walk`) — never whichever thread is first
            if ft.is_symlink() && !ctx.w.dup {
                if let Ok(target) = std::fs::canonicalize(&path)
                    && !ctx.reaches(&target)
                {
                    ctx.deferred.lock().unwrap().push((rel, target));
                }
                continue;
            }
            if ctx.w.dir {
                emit(ctx, rel.clone());
            }
            let sub_prefix = format!("{rel}/");
            match scope {
                Some(sc) => {
                    let chain = chain.clone();
                    sc.spawn(move |sc| walk_dir(ctx, Some(sc), &path, &sub_prefix, &chain));
                }
                None => walk_dir(ctx, None, &path, &sub_prefix, &chain),
            }
        } else if ctx.w.file {
            emit(ctx, rel);
        }
    }
}


#[cfg(test)]
mod walk_tests {
    use super::*;

    fn run(root: &Path, w: Walker) -> Vec<String> {
        let (tx, rx) = std::sync::mpsc::channel();
        let feed = Feed {
            tx,
            epoch: 0,
            build: None,
            child: None,
        };
        let r = root.to_string_lossy().into_owned();
        walk(&[r.clone()], &w, &[".git".into(), "node_modules".into()], &feed);
        drop(feed);
        let mut out: Vec<String> = Vec::new();
        for (_, e) in rx {
            if let Event::Lines(l) = e {
                out.extend(l.into_iter().map(|p| p.trim_start_matches(&format!("{r}/")).to_string()));
            }
        }
        out.sort();
        out
    }

    fn walker(dup: bool) -> Walker {
        Walker {
            file: true,
            dir: false,
            follow: true,
            hidden: true,
            dup,
        }
    }

    /// A tree with a symlink into itself, one outside, two to the same outside
    /// directory and a loop: each file once, under its real path (or the first
    /// symlink in path order); `dup` lists it under every path, as fzf; a loop
    /// never hangs either.
    #[test]
    fn the_walker_lists_each_directory_once_and_the_same_way() {
        use std::os::unix::fs::symlink;
        let base = std::env::temp_dir().join(format!("lvf-walk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("root");
        let out = base.join("outside");
        std::fs::create_dir_all(root.join("real/sub")).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(root.join("real/sub/a.txt"), "a").unwrap();
        std::fs::write(out.join("b.txt"), "b").unwrap();
        symlink(root.join("real"), root.join("alias")).unwrap(); // into the walk: not followed
        symlink(&out, root.join("x-out")).unwrap(); // outside: followed after the walk
        symlink(&out, root.join("y-out")).unwrap(); // the same target again: skipped
        symlink(&root, root.join("real/loop")).unwrap(); // a loop

        let once = run(&root, walker(false));
        assert_eq!(once, vec!["real/sub/a.txt", "x-out/b.txt"]);
        for _ in 0..5 {
            assert_eq!(run(&root, walker(false)), once, "the same names every time");
        }

        let all = run(&root, walker(true));
        assert!(all.contains(&"alias/sub/a.txt".to_string()), "{all:?}");
        assert!(all.contains(&"y-out/b.txt".to_string()), "{all:?}");
        assert!(all.contains(&"real/sub/a.txt".to_string()), "{all:?}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
