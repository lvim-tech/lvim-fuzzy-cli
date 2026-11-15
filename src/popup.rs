//! `--popup[=OPTS]` (fzf's `--tmux` is an alias): run the finder in a floating
//! pane of the multiplexer it is started in — a Zellij floating pane (`$ZELLIJ`)
//! or a tmux popup (`$TMUX`); outside both it simply runs in place.
//!
//! The input and the result pass through transparently: stdin is streamed into
//! the pane through a FIFO, the pane's output and exit code come back through
//! files, so a script (or the shell's CTRL-T) cannot tell the difference.
//!
//! OPTS: `[center|top|bottom|left|right][,WIDTH[%]][,HEIGHT[%]]`, default
//! `center,50%` (one size = both).

use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::placeholder::quote;

#[derive(Debug, PartialEq, Eq)]
pub struct Geometry {
    pub pos: String,
    pub width: String,
    pub height: String,
}

pub fn parse(opts: &str) -> Result<Geometry, String> {
    let mut g = Geometry {
        pos: "center".into(),
        width: "50%".into(),
        height: "50%".into(),
    };
    let mut sizes = Vec::new();
    for p in opts.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match p {
            "center" | "top" | "bottom" | "left" | "right" => g.pos = p.into(),
            "border-native" => {}
            s if s.trim_end_matches('%').parse::<u16>().is_ok() => sizes.push(s.to_string()),
            other => return Err(format!("invalid --popup option: {other}")),
        }
    }
    match sizes.as_slice() {
        [] => {}
        [one] => {
            // one size: along the side's axis for top/bottom/left/right, both for center
            match g.pos.as_str() {
                "top" | "bottom" => {
                    g.width = "100%".into();
                    g.height = one.clone();
                }
                "left" | "right" => {
                    g.width = one.clone();
                    g.height = "100%".into();
                }
                _ => {
                    g.width = one.clone();
                    g.height = one.clone();
                }
            }
        }
        [w, h, ..] => {
            g.width = w.clone();
            g.height = h.clone();
        }
    }
    Ok(g)
}

/// Which multiplexer the finder runs in.
pub fn host() -> Option<&'static str> {
    if std::env::var_os("ZELLIJ").is_some() {
        Some("zellij")
    } else if std::env::var_os("TMUX").is_some() {
        Some("tmux")
    } else {
        None
    }
}

/// `100% - size`, halved, for centring in percent (pane origin).
fn centred(size: &str) -> String {
    match size.strip_suffix('%').and_then(|n| n.parse::<u16>().ok()) {
        Some(p) => format!("{}%", (100u16.saturating_sub(p)) / 2),
        None => "0".into(),
    }
}

fn far(size: &str) -> String {
    match size.strip_suffix('%').and_then(|n| n.parse::<u16>().ok()) {
        Some(p) => format!("{}%", 100u16.saturating_sub(p)),
        None => "0".into(),
    }
}

fn temp_dir() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let d = base.join(format!("lvim-fuzzy-popup-{}", std::process::id()));
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

/// Run `args` (the user's own argv, --popup removed) in a floating pane;
/// returns the exit code to hand on.
pub fn run(opts: &str, args: &[String]) -> Result<i32, String> {
    let g = parse(opts)?;
    let Some(host) = host() else {
        return Err("not in a multiplexer".into()); // caller runs in place
    };
    let dir = temp_dir()?;
    let out = dir.join("out");
    let rc = dir.join("rc");
    let input = dir.join("in");
    let piped = !std::io::stdin().is_terminal();
    if piped {
        let st = Command::new("mkfifo")
            .arg(&input)
            .status()
            .map_err(|e| format!("mkfifo: {e}"))?;
        if !st.success() {
            return Err("mkfifo failed".into());
        }
    }
    // the command the pane runs: same program, same options, our env
    let exe = quote(&crate::self_exe());
    let argv: String = args.iter().map(|a| format!(" {}", quote(a))).collect();
    let envs: String = std::env::vars()
        .filter(|(k, _)| k.starts_with("LVIM_FUZZY_"))
        .map(|(k, v)| format!("{k}={} ", quote(&v)))
        .collect();
    let stdin = if piped {
        format!(" < {}", quote(&input.display().to_string()))
    } else {
        String::new()
    };
    let script = format!(
        "trap 'echo 130 > {rc}' HUP INT TERM; env {envs}LVIM_FUZZY_POPUP_CHILD=1 {exe}{argv}{stdin} > {out}; echo $? > {rc}",
        rc = quote(&rc.display().to_string()),
        out = quote(&out.display().to_string()),
    );
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".into());
    // feed stdin into the FIFO while the pane runs
    let opened = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let feeder = piped.then(|| {
        let input = input.clone();
        let opened = opened.clone();
        std::thread::spawn(move || {
            if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(&input) {
                opened.store(true, std::sync::atomic::Ordering::SeqCst);
                let mut buf = [0u8; 1 << 16];
                let mut sin = std::io::stdin().lock();
                while let Ok(n) = sin.read(&mut buf) {
                    if n == 0 || f.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            }
        })
    });
    let (x, y) = match g.pos.as_str() {
        "top" => (centred(&g.width), "0".to_string()),
        "bottom" => (centred(&g.width), far(&g.height)),
        "left" => ("0".to_string(), centred(&g.height)),
        "right" => (far(&g.width), centred(&g.height)),
        _ => (centred(&g.width), centred(&g.height)),
    };
    let status = match host {
        "zellij" => Command::new("zellij")
            .args([
                "run",
                "--floating",
                "--close-on-exit",
                "--block-until-exit",
                "--name",
                "lvim-fuzzy",
            ])
            .args([
                "--width", &g.width, "--height", &g.height, "--x", &x, "--y", &y, "--cwd", &cwd,
            ])
            .args(["--", "sh", "-c", &script])
            .stdin(Stdio::null())
            .status(),
        _ => {
            let (tx, ty) = match g.pos.as_str() {
                "top" => ("C", "0"),
                "bottom" => ("C", "S"),
                "left" => ("0", "C"),
                "right" => ("R", "C"),
                _ => ("C", "C"),
            };
            Command::new("tmux")
                .args([
                    "display-popup",
                    "-E",
                    "-w",
                    &g.width,
                    "-h",
                    &g.height,
                    "-x",
                    tx,
                    "-y",
                    ty,
                    "-d",
                    &cwd,
                ])
                .arg(format!("sh -c {}", quote(&script)))
                .stdin(Stdio::null())
                .status()
        }
    };
    if let Err(e) = status {
        return Err(format!("{host}: {e}"));
    }
    // a pane closed by hand never ran the trailing `echo $?`
    let code = std::fs::read_to_string(&rc)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(130);
    if let Ok(mut f) = std::fs::File::open(&out) {
        let mut so = std::io::stdout().lock();
        let _ = std::io::copy(&mut f, &mut so);
        let _ = so.flush();
    }
    if piped && !opened.load(std::sync::atomic::Ordering::SeqCst) {
        // the pane never opened the FIFO: open its read end (NON-blocking — a
        // plain open would wait for a writer forever) so the feeder's open returns
        use std::os::unix::fs::OpenOptionsExt;
        let _ = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&input);
    }
    // the feeder is not waited for: with a slow or endless input it would block
    // on stdin long after the pane is gone; the process exits right after this
    drop(feeder);
    for f in [&out, &rc, &input] {
        let _ = std::fs::remove_file(f);
    }
    let _ = std::fs::remove_dir(&dir);
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry() {
        assert_eq!(
            parse("").unwrap(),
            Geometry {
                pos: "center".into(),
                width: "50%".into(),
                height: "50%".into()
            }
        );
        assert_eq!(parse("center,80%").unwrap().height, "80%");
        let b = parse("bottom,40%").unwrap();
        assert_eq!((b.width.as_str(), b.height.as_str()), ("100%", "40%"));
        let c = parse("70%,30%").unwrap();
        assert_eq!((c.width.as_str(), c.height.as_str()), ("70%", "30%"));
        assert!(parse("sideways").is_err());
        assert_eq!(centred("60%"), "20%");
        assert_eq!(far("40%"), "60%");
    }
}
