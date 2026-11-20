//! `t9 web`: a local web UI over the vault.
//!
//! A small single-threaded HTTP server (tiny_http) on 127.0.0.1 serving an embedded single-page app and a JSON
//! API over `t9-core`. No build step, no external assets, nothing leaves the machine.
//!
//! Security: the server binds to loopback only; every request must carry the per-run token (set as an
//! HttpOnly, SameSite=Strict cookie by the link `t9 web` prints); the Host header must be the loopback address
//! (defeats DNS rebinding); writes additionally require an `X-T9` header, which cross-site pages cannot send.

mod api;

use std::io::Read;

use anyhow::{Context, Result};
use t9_core::Store;
use tiny_http::{Header, Method, Request, Response, Server};

const INDEX: &str = include_str!("../assets/index.html");
const APP_JS: &str = include_str!("../assets/app.js");
const APP_CSS: &str = include_str!("../assets/app.css");
const COOKIE: &str = "t9_token";

pub struct Web {
    server: Server,
    token: String,
    port: u16,
}

impl Web {
    /// Binds to `127.0.0.1:port` (0 picks a free port).
    pub fn bind(port: u16) -> Result<Web> {
        let server = Server::http(("127.0.0.1", port)).map_err(|e| anyhow::anyhow!("starting the web server: {e}"))?;
        let port = server.server_addr().to_ip().context("server address")?.port();
        Ok(Web { server, token: t9_core::new_id(), port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The link that opens the UI (it carries the access token), optionally at a route such as `#/i/QR-12`.
    pub fn url(&self, route: Option<&str>) -> String {
        let mut u = format!("http://127.0.0.1:{}/?token={}", self.port, self.token);
        if let Some(r) = route {
            u.push_str(r);
        }
        u
    }

    /// Serves requests until the process ends.
    pub fn serve(self, mut store: Store) -> Result<()> {
        for req in self.server.incoming_requests() {
            if let Err(e) = handle(&mut store, &self.token, self.port, req) {
                eprintln!("t9 web: {e:#}");
            }
        }
        Ok(())
    }
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid header")
}

fn get_header<'a>(req: &'a Request, name: &'static str) -> Option<&'a str> {
    req.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str())
}

fn has_token(req: &Request, token: &str) -> bool {
    get_header(req, "Cookie").is_some_and(|c| {
        c.split(';').any(|kv| kv.trim().strip_prefix(&format!("{COOKIE}=")).is_some_and(|v| v == token))
    })
}

fn text(status: u16, body: &str, content_type: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body)
        .with_status_code(status)
        .with_header(header("Content-Type", content_type))
        .with_header(header("Cache-Control", "no-store"))
        .with_header(header("X-Content-Type-Options", "nosniff"))
        .with_header(header("Referrer-Policy", "no-referrer"))
        .with_header(header(
            "Content-Security-Policy",
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'",
        ))
}

fn handle(store: &mut Store, token: &str, port: u16, mut req: Request) -> Result<()> {
    // Only answer requests addressed to the loopback server itself (DNS-rebinding defense).
    let host = get_header(&req, "Host").unwrap_or("").to_string();
    if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") {
        return Ok(req.respond(text(403, "forbidden", "text/plain"))?);
    }
    let url = req.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
    let params = parse_query(query);

    // The link printed by `t9 web` carries the token once; trade it for a cookie and a clean URL.
    if path == "/" && params.iter().any(|(k, v)| k == "token" && v == token) {
        let resp = Response::empty(303)
            .with_header(header("Location", "/"))
            .with_header(header("Set-Cookie", &format!("{COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/")))
            .with_header(header("Cache-Control", "no-store"));
        return Ok(req.respond(resp)?);
    }
    if !has_token(&req, token) {
        let msg = "<!doctype html><meta charset=utf-8><title>t9</title><body style=\"font:15px system-ui;margin:3em\">\
                   <h2>t9</h2><p>Open the link printed by <code>t9 web</code> (it carries a one-time access token).</p>";
        return Ok(req.respond(text(401, msg, "text/html; charset=utf-8"))?);
    }

    let resp = match (req.method().clone(), path) {
        (Method::Get, "/") => text(200, INDEX, "text/html; charset=utf-8"),
        (Method::Get, "/app.js") => text(200, APP_JS, "text/javascript; charset=utf-8"),
        (Method::Get, "/app.css") => text(200, APP_CSS, "text/css; charset=utf-8"),
        (Method::Get, "/favicon.ico") => text(204, "", "image/x-icon"),
        (m, p) if p.starts_with("/api/") => {
            let write = m == Method::Post;
            if write && get_header(&req, "X-T9") != Some("1") {
                text(403, "{\"error\":\"missing X-T9 header\"}", "application/json")
            } else {
                let mut body = String::new();
                if write {
                    req.as_reader().take(1 << 20).read_to_string(&mut body)?;
                }
                let segs: Vec<&str> = p.trim_start_matches("/api/").split('/').filter(|s| !s.is_empty()).collect();
                match api::route(store, &m, &segs, &params, &body) {
                    Ok(v) => text(200, &v.to_string(), "application/json"),
                    Err(e) => {
                        let status = if e.to_string().starts_with("not found") { 404 } else { 400 };
                        text(status, &serde_json::json!({ "error": format!("{e:#}") }).to_string(), "application/json")
                    }
                }
            }
        }
        _ => text(404, "not found", "text/plain"),
    };
    Ok(req.respond(resp)?)
}

fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|kv| !kv.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (decode(k), decode(v))
        })
        .collect()
}

/// Percent-decoding for query strings (`+` is a space).
fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoding() {
        assert_eq!(decode("a+b%20c%2Fd"), "a b c/d");
        assert_eq!(decode("100%"), "100%");
        assert_eq!(
            parse_query("q=webhook+batching&p=BIL"),
            vec![("q".into(), "webhook batching".into()), ("p".into(), "BIL".into())]
        );
    }
}
