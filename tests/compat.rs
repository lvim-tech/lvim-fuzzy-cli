//! Compatibility with fzf: the same input, query and options through both
//! programs' --filter must select the same items (and, where fzf's order is
//! fixed — --no-sort, --tac, --tail — the same order). Skipped when fzf is not
//! installed.

use std::io::Write;
use std::process::{Command, Stdio};

const ME: &str = env!("CARGO_BIN_EXE_lvim-fuzzy-cli");

fn have_fzf() -> bool {
    Command::new("fzf")
        .arg("--version")
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn run(prog: &str, input: &[u8], args: &[&str]) -> (Vec<u8>, i32) {
    let mut c = Command::new(prog)
        .args(args)
        .env_remove("FZF_DEFAULT_OPTS")
        .env_remove("FZF_DEFAULT_OPTS_FILE")
        .env_remove("LVIM_FUZZY_DEFAULT_OPTS")
        .env_remove("LVIM_FUZZY_DEFAULT_OPTS_FILE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(input).unwrap();
    let out = c.wait_with_output().unwrap();
    (out.stdout, out.status.code().unwrap_or(-1))
}

/// A corpus with paths, words, case, unicode and grep-like lines.
fn corpus() -> String {
    let mut v = Vec::new();
    for dir in [
        "src",
        "src/tui",
        "lua/lvim-ui",
        "tests",
        ".config/nvim",
        "docs/Guide",
    ] {
        for f in [
            "main.rs",
            "view_lvim.rs",
            "init.lua",
            "README.md",
            "Config.toml",
            "health.lua",
            "café.txt",
        ] {
            v.push(format!("{dir}/{f}"));
        }
    }
    v.push("src/tui/view.rs:216:    fn draw_item(".into());
    v.push("lua/lvim-ui/health.lua:21:    h.start(\"lvim-ui\")".into());
    v.push("A B C  spaced   fields".into());
    v.push("key=value=more".into());
    v.join("\n") + "\n"
}

fn sorted(b: &[u8]) -> Vec<String> {
    let mut v: Vec<String> = String::from_utf8_lossy(b)
        .lines()
        .map(str::to_string)
        .collect();
    v.sort();
    v
}

#[test]
fn filter_matches_fzf() {
    if !have_fzf() {
        eprintln!("fzf not installed: skipped");
        return;
    }
    let input = corpus();
    let queries = [
        "",
        "rs",
        "lvim",
        "init",
        "INIT",
        "Init",
        "tui view",
        "'health",
        "^src",
        "lua$",
        "!lua",
        "rs | md",
        "src !tui",
        "'lvim-ui'",
        "cafe",
        "café",
        "^lua 'health",
        "h.start",
        "zzzz",
    ];
    let option_sets: &[&[&str]] = &[
        &[],
        &["-e"],
        &["-i"],
        &["+i"],
        &["--literal"],
        &["+x"],
        &["--nth=1", "-d/"],
        &["--nth=-1", "-d/"],
        &["--nth=2..", "-d", ":"],
        &["--with-nth=2", "-d", "/"],
        &["--scheme=path"],
        &["--scheme=history"],
    ];
    let mut fails = Vec::new();
    for opts in option_sets {
        for q in queries {
            let mut args: Vec<&str> = opts.to_vec();
            args.push("-f");
            args.push(q);
            let (a, ca) = run("fzf", input.as_bytes(), &args);
            let (b, cb) = run(ME, input.as_bytes(), &args);
            if sorted(&a) != sorted(&b) || ca != cb {
                fails.push(format!(
                    "{args:?}: fzf={:?} ({ca}) lvim={:?} ({cb})",
                    sorted(&a),
                    sorted(&b)
                ));
            }
        }
    }
    assert!(
        fails.is_empty(),
        "{} mismatch(es):\n{}",
        fails.len(),
        fails.join("\n")
    );
}

#[test]
fn ordered_options_match_fzf() {
    if !have_fzf() {
        return;
    }
    let input = corpus();
    let cases: &[&[&str]] = &[
        &["--no-sort", "-f", "rs"],
        &["--tac", "--no-sort", "-f", "lua"],
        &["--tail=5", "-f", ""],
        &["--tail=10", "--no-sort", "-f", "s"],
        &["--accept-nth=1", "-d/", "--no-sort", "-f", "health"],
        &["--accept-nth=2,1", "-d/", "--no-sort", "-f", "init"],
        &["--print-query", "--no-sort", "-f", "view"],
        &["--read0", "--print0", "--no-sort", "-f", "rs"],
    ];
    let mut fails = Vec::new();
    for args in cases {
        let data = if args.contains(&"--read0") {
            input.replace('\n', "\0")
        } else {
            input.clone()
        };
        let (a, ca) = run("fzf", data.as_bytes(), args);
        let (b, cb) = run(ME, data.as_bytes(), args);
        if a != b || ca != cb {
            fails.push(format!(
                "{args:?}:\n  fzf  ({ca}) {:?}\n  lvim ({cb}) {:?}",
                String::from_utf8_lossy(&a),
                String::from_utf8_lossy(&b)
            ));
        }
    }
    assert!(
        fails.is_empty(),
        "{} mismatch(es):\n{}",
        fails.len(),
        fails.join("\n")
    );
}

#[test]
fn exit_codes_match_fzf() {
    if !have_fzf() {
        return;
    }
    for (args, input) in [
        (vec!["-f", "zzz"], "a\nb\n"),
        (vec!["-f", "a"], "a\nb\n"),
        (vec!["--bogus-option"], ""),
    ] {
        let (_, ca) = run("fzf", input.as_bytes(), &args);
        let (_, cb) = run(ME, input.as_bytes(), &args);
        assert_eq!(ca, cb, "{args:?}");
    }
}

/// The ranked order (with sorting on) for a real-world list: the files of this
/// repository. Reported as a ratio — the scoring models differ by design
/// (lvim-fuzzy's affine gaps), so only the top hit must agree.
#[test]
fn ranking_top_hit_matches_fzf() {
    if !have_fzf() {
        return;
    }
    let mut files = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "target" {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else {
                files.push(
                    p.strip_prefix(env!("CARGO_MANIFEST_DIR"))
                        .unwrap()
                        .to_string_lossy()
                        .to_string(),
                );
            }
        }
    }
    files.sort();
    let input = files.join("\n") + "\n";
    let mut same = 0;
    let mut diffs = Vec::new();
    let queries = [
        "main", "view", "tuiv", "lvim", "rs", "compat", "readme", "act", "pvw", "theme", "srcm",
    ];
    for q in queries {
        let (a, _) = run("fzf", input.as_bytes(), &["--scheme=path", "-f", q]);
        let (b, _) = run(ME, input.as_bytes(), &["--scheme=path", "-f", q]);
        let ta = String::from_utf8_lossy(&a)
            .lines()
            .next()
            .map(str::to_string);
        let tb = String::from_utf8_lossy(&b)
            .lines()
            .next()
            .map(str::to_string);
        if ta == tb {
            same += 1;
        } else {
            diffs.push(format!("{q}: fzf={ta:?} lvim={tb:?}"));
        }
    }
    eprintln!("top hit agrees for {same}/{}", queries.len());
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
}

/// Input edge cases of the --filter fast path, from a pipe and from a file.
#[test]
fn input_edges_match_fzf() {
    if !have_fzf() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("lvim-fuzzy-compat-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cases: &[(&[u8], &[&str])] = &[
        (b"h1\nh2\na\r\nb\n\nc", &["--header-lines=2", "-f", ""]),
        (b"one\ntwo", &["--header-lines=5", "-f", ""]),
        (b"", &["-f", ""]),
        (b"x\ny\n", &["-f", "y"]),
        (b"a\0b\0ab", &["--read0", "--print0", "-f", "a"]),
        (b"caf\xe9\nok\n", &["-f", "caf"]),
    ];
    for (k, (input, args)) in cases.iter().enumerate() {
        let (a, ca) = run("fzf", input, args);
        let (b, cb) = run(ME, input, args);
        assert_eq!(
            (String::from_utf8_lossy(&a), ca),
            (String::from_utf8_lossy(&b), cb),
            "pipe {args:?}"
        );
        // the same from a file on stdin (mapped)
        let f = dir.join(format!("in{k}"));
        std::fs::write(&f, input).unwrap();
        let out = Command::new(ME)
            .args(*args)
            .stdin(std::fs::File::open(&f).unwrap())
            .output()
            .unwrap();
        assert_eq!(out.stdout, b, "file {args:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
