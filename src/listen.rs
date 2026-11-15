//! `--listen[=[ADDR:]PORT | SOCKET]`: an HTTP endpoint for driving the finder
//! from outside, as fzf's.
//!
//!   POST /   body = actions in --bind syntax (`change-query(x)+accept`)
//!   GET  /   the state as JSON (query, position, counts, current, matches, selected);
//!            `?limit=N&offset=M` page the matches (default 100)
//!
//! `LVIM_FUZZY_API_KEY`, when set, must come as the `X-API-Key` header. Without
//! `--listen-unsafe`, actions that run processes are refused.

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::mpsc::{Sender, channel};
use std::thread;

pub enum Req {
    /// Run these actions; reply with an error message or None.
    Actions(String, Sender<Option<String>>),
    /// The state as JSON (limit, offset).
    State(usize, usize, Sender<String>),
}

/// Actions that start processes (refused without --listen-unsafe).
pub const UNSAFE: &[&str] = &[
    "execute",
    "execute-silent",
    "execute-multi",
    "become",
    "reload",
    "reload-sync",
    "preview",
    "change-preview",
    "transform",
    "bg-transform",
];

pub fn is_unsafe(action: &str) -> bool {
    UNSAFE.contains(&action)
        || action.starts_with("transform-")
        || action.starts_with("bg-transform-")
}

/// Start listening; returns the address (port for TCP) for LVIM_FUZZY_PORT.
pub fn start(spec: &str, tx: Sender<Req>) -> Result<String, String> {
    let key = std::env::var("LVIM_FUZZY_API_KEY")
        .ok()
        .filter(|k| !k.is_empty());
    if spec.ends_with(".sock") || spec.contains('/') {
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(spec);
            let l = std::os::unix::net::UnixListener::bind(spec)
                .map_err(|e| format!("--listen {spec}: {e}"))?;
            let path = spec.to_string();
            thread::spawn(move || {
                for s in l.incoming().flatten() {
                    let tx = tx.clone();
                    let key = key.clone();
                    thread::spawn(move || {
                        let r = s.try_clone();
                        if let Ok(r) = r {
                            serve(BufReader::new(r), s, &tx, key.as_deref());
                        }
                    });
                }
            });
            return Ok(path);
        }
        #[cfg(not(unix))]
        return Err("unix sockets are not available here".into());
    }
    let addr = if spec.is_empty() {
        "localhost:0".to_string()
    } else if spec.chars().all(|c| c.is_ascii_digit()) {
        format!("localhost:{spec}")
    } else {
        spec.to_string()
    };
    let l = std::net::TcpListener::bind(&addr).map_err(|e| format!("--listen {addr}: {e}"))?;
    let port = l
        .local_addr()
        .map(|a| a.port().to_string())
        .unwrap_or_default();
    thread::spawn(move || {
        for s in l.incoming().flatten() {
            let tx = tx.clone();
            let key = key.clone();
            thread::spawn(move || {
                if let Ok(r) = s.try_clone() {
                    serve(BufReader::new(r), s, &tx, key.as_deref());
                }
            });
        }
    });
    Ok(port)
}

fn respond<W: Write>(mut w: W, code: u16, reason: &str, ctype: &str, body: &str) {
    let _ = write!(
        w,
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = w.flush();
}

fn serve<R: Read, W: Write>(mut r: BufReader<R>, w: W, tx: &Sender<Req>, key: Option<&str>) {
    let mut line = String::new();
    if r.read_line(&mut line).is_err() {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    let mut len = 0usize;
    let mut got_key: Option<String> = None;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).is_err() || h == "\r\n" || h == "\n" || h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_string());
            if k == "content-length" {
                len = v.parse().unwrap_or(0).min(1 << 20);
            } else if k == "x-api-key" {
                got_key = Some(v);
            }
        }
    }
    if let Some(k) = key
        && got_key.as_deref() != Some(k)
    {
        return respond(w, 401, "Unauthorized", "text/plain", "invalid API key\n");
    }
    match method.as_str() {
        "POST" => {
            let mut body = vec![0u8; len];
            if r.read_exact(&mut body).is_err() {
                return respond(w, 400, "Bad Request", "text/plain", "short body\n");
            }
            let (rtx, rrx) = channel();
            if tx
                .send(Req::Actions(
                    String::from_utf8_lossy(&body).trim().to_string(),
                    rtx,
                ))
                .is_err()
            {
                return;
            }
            match rrx.recv() {
                Ok(None) => respond(w, 200, "OK", "text/plain", ""),
                Ok(Some(e)) => respond(w, 400, "Bad Request", "text/plain", &format!("{e}\n")),
                Err(_) => {}
            }
        }
        "GET" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let param = |name: &str| -> Option<usize> {
                q.split('&')
                    .find_map(|kv| kv.strip_prefix(name)?.strip_prefix('=')?.parse().ok())
            };
            let (rtx, rrx) = channel();
            if tx
                .send(Req::State(
                    param("limit").unwrap_or(100),
                    param("offset").unwrap_or(0),
                    rtx,
                ))
                .is_err()
            {
                return;
            }
            if let Ok(json) = rrx.recv() {
                respond(w, 200, "OK", "application/json", &json);
            }
        }
        _ => respond(w, 405, "Method Not Allowed", "text/plain", "GET or POST\n"),
    }
}

/// A JSON string literal.
pub fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(port: &str, req: &str) -> String {
        let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn serves_get_and_post() {
        let (tx, rx) = channel();
        let port = start("0", tx).unwrap();
        // the finder side: answer one state query and one action request
        let h = thread::spawn(move || {
            for _ in 0..2 {
                match rx.recv().unwrap() {
                    Req::State(limit, offset, r) => r
                        .send(format!("{{\"limit\":{limit},\"offset\":{offset}}}"))
                        .unwrap(),
                    Req::Actions(body, r) => r
                        .send(if body == "accept" {
                            None
                        } else {
                            Some("bad".into())
                        })
                        .unwrap(),
                }
            }
        });
        let got = http(&port, "GET /?limit=5&offset=2 HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(got.starts_with("HTTP/1.1 200"), "{got}");
        assert!(got.ends_with("{\"limit\":5,\"offset\":2}"), "{got}");
        let got = http(&port, "POST / HTTP/1.1\r\nContent-Length: 6\r\n\r\naccept");
        assert!(got.starts_with("HTTP/1.1 200"), "{got}");
        h.join().unwrap();
    }

    #[test]
    fn json_and_safety() {
        assert_eq!(json_str("a\"b\n"), "\"a\\\"b\\n\"");
        assert!(is_unsafe("execute") && is_unsafe("transform-query") && !is_unsafe("change-query"));
    }
}
