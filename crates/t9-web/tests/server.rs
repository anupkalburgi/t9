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
    let mut bytes = vec![];
    s.read_to_end(&mut bytes).unwrap();
    let split = bytes.windows(4).position(|w| w == b"\r\n\r\n").map_or(bytes.len(), |p| p + 4);
    let head = String::from_utf8_lossy(&bytes[..split]).trim_end().to_string();
    let mut body = bytes[split..].to_vec();
    if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        body = dechunk(&body);
    }
    let body = String::from_utf8(body).unwrap_or_else(|e| panic!("{method} {path}: not UTF-8: {e}"));
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    Res { status, headers: head, body }
}

/// Decodes a chunked HTTP body (large responses are sent in chunks; a chunk may end inside a UTF-8 character).
fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    while let Some(eol) = b.windows(2).position(|w| w == b"\r\n") {
        let line = String::from_utf8_lossy(&b[..eol]).to_string();
        let size = usize::from_str_radix(line.split(';').next().unwrap().trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        out.extend_from_slice(&b[eol + 2..eol + 2 + size]);
        b = &b[eol + 2 + size + 2..];
    }
    out
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

    // The story: the question's goal is inferred from its parent; the answer is its outcome.
    let page = json(http(port, "GET", "/api/item/BIL-2", &auth, ""));
    assert_eq!(page["story"]["goal"]["source"], "parent", "{}", page["story"]);
    assert_eq!(page["story"]["goal"]["from"], "BIL-1");
    assert_eq!(page["story"]["outcome"]["class"], "done");
    assert!(page["story"]["outcome"]["text"].as_str().unwrap().contains("recorded as BIL-3"), "{}", page["story"]);
    assert!(page["story"]["led_to"].as_array().unwrap().iter().any(|l| l["id"] == "BIL-3"), "{}", page["story"]);

    // A due date set from the web shows up as needing attention, on the item and in the list.
    let f = json(http(
        port,
        "POST",
        "/api/item/BIL-1/fields",
        &hdrs,
        r#"{"due":"2020-01-01","goal":"merchants hear about payments"}"#,
    ));
    assert_eq!((f["due"].as_str(), f["goal"].as_str()), (Some("2020-01-01"), Some("merchants hear about payments")));
    let att = json(http(port, "GET", "/api/p/BIL/attention", &auth, ""));
    assert_eq!(att["items"][0]["id"], "BIL-1", "{att}");
    assert_eq!(att["items"][0]["reason"], "overdue");
    let items = json(http(port, "GET", "/api/p/BIL/items", &auth, ""));
    let b1 = items.as_array().unwrap().iter().find(|i| i["id"] == "BIL-1").unwrap();
    assert_eq!(b1["fresh"]["reason"], "overdue");
    let b2 = items.as_array().unwrap().iter().find(|i| i["id"] == "BIL-2").unwrap();
    assert_eq!(b2["outcome"]["glyph"], "✓");
    assert!(http(port, "POST", "/api/item/BIL-1/still", &hdrs, r#"{"text":" "}"#).body.contains("still relevant"));
    let e = json(http(port, "POST", "/api/item/BIL-1/still", &hdrs, r#"{"text":"waiting for the provider"}"#));
    assert_eq!(e["kind"], "checkin");

    // The timeline: lanes, lifespans with outcomes, and each decision once.
    let tl = json(http(port, "GET", "/api/p/BIL/lanes", &auth, ""));
    assert!(tl["lanes"].as_array().unwrap().iter().any(|l| l["id"] == "BIL-1"), "{tl}");
    let span = tl["spans"].as_array().unwrap().iter().find(|s| s["id"] == "BIL-2").unwrap();
    assert_eq!((span["lane"].as_str(), span["outcome"].as_str()), (Some("BIL-1"), Some("done")));
    assert!(span["closed"].is_string());
    let decisions =
        tl["marks"].as_array().unwrap().iter().filter(|m| m["kind"] == "decision" && m["item"] == "BIL-3").count();
    assert_eq!(decisions, 1, "{tl}");
    let js = http(port, "GET", "/timeline.js", &auth, "");
    assert!(js.status == 200 && js.body.contains("function renderTimeline"));

    // Asking the agents from the web: a question for agents about the item, shown on its story until answered.
    let q = json(http(port, "POST", "/api/item/BIL-1/ask", &hdrs, r#"{"text":"Why do webhooks retry three times?"}"#));
    let qid = q["id"].as_str().unwrap().to_string();
    let asked = json(http(port, "GET", "/api/p/BIL/asked", &auth, ""));
    assert_eq!((asked[0]["id"].as_str(), asked[0]["about"].as_str()), (Some(qid.as_str()), Some("BIL-1")));
    assert!(asked[0]["answer"].is_null());
    let page = json(http(port, "GET", "/api/item/BIL-1", &auth, ""));
    assert_eq!(page["story"]["asked"][0]["id"], qid.as_str());
    let att = json(http(port, "GET", "/api/p/BIL/attention", &auth, ""));
    assert_eq!(
        (att["items"][0]["id"].as_str(), att["items"][0]["reason"].as_str()),
        (Some(qid.as_str()), Some("asked"))
    );

    // Unknown items.
    assert_eq!(http(port, "GET", "/api/item/BIL-99", &auth, "").status, 404);
}
