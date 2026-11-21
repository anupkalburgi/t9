//! Runs the real server on a temp vault and talks HTTP to it.

use std::io::{Read, Write};
use std::net::TcpStream;

use t9_core::*;

struct Res {
    status: u16,
    headers: String,
    body: String,
}

fn http(port: u16, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Res {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
    if !headers.iter().any(|(k, _)| *k == "Host") {
        req.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
    }
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    Res { status, headers: head.to_string(), body: body.to_string() }
}

fn json(r: Res) -> serde_json::Value {
    serde_json::from_str(&r.body).unwrap_or_else(|e| panic!("{e}: {}", r.body))
}

#[test]
fn web_api_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let repo = root.join("billing");
    std::fs::create_dir_all(repo.join("reports")).unwrap();
    std::fs::write(repo.join("reports/options.md"), "# Options\n\n| A | B |\n|---|---|\n| batch | one by one |\n")
        .unwrap();
    std::fs::write(repo.join(".env"), "SECRET=1").unwrap();
    std::fs::write(root.join("outside.md"), "no").unwrap();
    let mut store = Store::open(Env::new(root.join("state"), root.join("vault"))).unwrap();
    store.init_project("Billing", Some("BIL"), "", Some(&repo)).unwrap();
    let c = Caller { cwd: repo.clone(), agent: Some("claude-code".into()), ..Default::default() };
    store.add_item(&c, NewItem { kind: Some(Kind::Feature), title: "Webhooks".into(), ..Default::default() }).unwrap();
    let note = LogInput {
        kind: EntryKind::Progress,
        body: "measured round trips".into(),
        item: Some("BIL-1".into()),
        refs: vec![],
    };
    store.log(&c, note).unwrap();
    // Make sure the question is created in a later second than the note before it.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let q = NewItem {
        kind: Some(Kind::Question),
        title: "Batch or not?".into(),
        body: "Options in reports/options.md".into(),
        parent: Some("BIL-1".into()),
        ..Default::default()
    };
    store.add_item(&c, q).unwrap();

    let web = t9_web::Web::bind(0).unwrap();
    let port = web.port();
    let url = web.url(None);
    let token_path = url[url.find("/?").unwrap()..].to_string();
    std::thread::spawn(move || web.serve(store));

    // Without the token: refused. Wrong Host: refused. With the token link: a cookie and a redirect.
    assert_eq!(http(port, "GET", "/api/projects", &[], "").status, 401);
    assert_eq!(http(port, "GET", &token_path, &[("Host", "evil.example")], "").status, 403);
    let r = http(port, "GET", &token_path, &[], "");
    assert_eq!(r.status, 303);
    let cookie_line = r.headers.lines().find(|l| l.starts_with("Set-Cookie")).unwrap().to_string();
    assert!(cookie_line.contains("HttpOnly") && cookie_line.contains("SameSite=Strict"), "{cookie_line}");
    let cookie = cookie_line.trim_start_matches("Set-Cookie: ").split(';').next().unwrap().to_string();
    let auth = [("Cookie", cookie.as_str())];

    // Pages and the API.
    let r = http(port, "GET", "/", &auth, "");
    assert!(r.status == 200 && r.body.contains("/app.js") && r.headers.contains("Content-Security-Policy"));
    let p = json(http(port, "GET", "/api/projects", &auth, ""));
    assert_eq!(p["projects"][0]["key"], "BIL");
    assert_eq!(p["projects"][0]["questions"], 1);

    // The question page: where it sits, who asked, what was happening.
    let page = json(http(port, "GET", "/api/item/BIL-2", &auth, ""));
    assert_eq!(page["chain"][0]["id"], "BIL-1");
    assert_eq!(page["asked"]["working_on"]["id"], "BIL-1");
    assert_eq!(page["asked"]["before"][0]["body"], "measured round trips");
    assert!(page["asked"]["session"]["id"].is_string());

    // Files the item refers to: shown if inside the project folder; refused otherwise.
    let f = json(http(port, "GET", "/api/p/BIL/file?path=reports%2Foptions.md", &auth, ""));
    assert!(f["text"].as_str().unwrap().contains("one by one") && f["markdown"] == true);
    assert!(http(port, "GET", "/api/p/BIL/file?path=.env", &auth, "").body.contains("not shown"));
    assert!(http(port, "GET", "/api/p/BIL/file?path=..%2Foutside.md", &auth, "").body.contains("outside"));

    // Writes need the X-T9 header; answering closes the question and records a decision.
    let body = r#"{"answer":"One by one for now","decide":true}"#;
    assert_eq!(http(port, "POST", "/api/item/BIL-2/answer", &auth, body).status, 403);
    let hdrs = [("Cookie", cookie.as_str()), ("X-T9", "1"), ("Content-Type", "application/json")];
    let a = json(http(port, "POST", "/api/item/BIL-2/answer", &hdrs, body));
    assert_eq!(a["item"]["status"], "done");
    assert_eq!(a["decision"], "BIL-3");
    let page = json(http(port, "GET", "/api/item/BIL-2", &auth, ""));
    assert!(page["entries"].as_array().unwrap().iter().any(|e| e["body"] == "Answer: One by one for now"), "{page}");
    let d = json(http(port, "GET", "/api/item/BIL-3", &auth, ""));
    assert!(d["item"]["body"].as_str().unwrap().contains("Answer to BIL-2"));

    // The map's signals.
    let ins = json(http(port, "GET", "/api/p/BIL/insights", &auth, ""));
    assert!(ins["signals"].is_array() && ins["branches"].is_array(), "{ins}");
    let app = http(port, "GET", "/app.js", &auth, "");
    assert!(app.status == 200 && app.body.contains("renderMap"));

    // Unknown items.
    assert_eq!(http(port, "GET", "/api/item/BIL-99", &auth, "").status, 404);
}
