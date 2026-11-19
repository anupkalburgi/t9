//! Journal files: one Markdown file per project per day, holding appended entry blocks.
//!
//! ```markdown
//! <!-- t9 {"id":"01JD…","kind":"progress","at":"2026-11-17T14:02:11Z","item":"BIL-42","agent":"claude-code"} -->
//! ### 14:02 · progress · BIL-42 · claude-code
//! Replayed 500 webhook events; no duplicate charges.
//! ```
//!
//! The comment line is the machine-readable header; the `###` heading is for humans and is regenerated from it.
//! Everything up to the next header is the body, so a human may edit bodies freely.

use crate::Error;
use crate::model::EntryMeta;

const MARKER: &str = "<!-- t9 ";
const MARKER_END: &str = " -->";

pub fn file_header(project_name: &str, date: &str) -> String {
    format!("# Journal · {project_name} · {date}\n\n")
}

fn heading(meta: &EntryMeta) -> String {
    let time = crate::clock::time(&meta.at);
    let mut h = format!("### {time} · {}", meta.kind);
    if let Some(item) = &meta.item {
        h.push_str(" · ");
        h.push_str(item);
    }
    if !meta.agent.is_empty() {
        h.push_str(" · ");
        h.push_str(&meta.agent);
    }
    h
}

fn header_line(meta: &EntryMeta) -> Result<String, Error> {
    let json = serde_json::to_string(meta).map_err(|e| Error::Parse(e.to_string()))?;
    Ok(format!("{MARKER}{json}{MARKER_END}"))
}

/// Renders a block to append to a journal file. Starts with a blank line so blocks stay separated.
pub fn render_block(meta: &EntryMeta, body: &str) -> Result<String, Error> {
    let mut out = String::new();
    out.push_str(&header_line(meta)?);
    out.push('\n');
    out.push_str(&heading(meta));
    out.push('\n');
    for line in body.trim_end().lines() {
        // A body line must never be mistaken for a block header.
        if line.starts_with(MARKER) {
            out.push(' ');
        }
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');
    Ok(out)
}

fn parse_header(line: &str) -> Option<Result<EntryMeta, Error>> {
    let inner = line.trim_end().strip_prefix(MARKER)?.strip_suffix(MARKER_END)?;
    Some(serde_json::from_str(inner).map_err(|e| Error::Parse(format!("journal header: {e}"))))
}

/// Parses all blocks of a journal file. Malformed headers are reported, not silently dropped.
pub fn parse(text: &str) -> (Vec<(EntryMeta, String)>, Vec<Error>) {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    let mut current: Option<(EntryMeta, Vec<&str>)> = None;
    let mut skip_heading = false;

    let flush = |cur: Option<(EntryMeta, Vec<&str>)>, out: &mut Vec<(EntryMeta, String)>| {
        if let Some((meta, lines)) = cur {
            let body = lines.join("\n").trim().to_string();
            out.push((meta, body));
        }
    };

    for line in text.lines() {
        if let Some(parsed) = parse_header(line) {
            flush(current.take(), &mut out);
            match parsed {
                Ok(meta) => {
                    current = Some((meta, Vec::new()));
                    skip_heading = true;
                }
                Err(e) => errors.push(e),
            }
            continue;
        }
        if let Some((_, lines)) = current.as_mut() {
            if skip_heading && line.starts_with("### ") {
                skip_heading = false;
                continue;
            }
            skip_heading = false;
            lines.push(line.strip_prefix(' ').filter(|l| l.starts_with(MARKER)).unwrap_or(line));
        }
    }
    flush(current, &mut out);
    (out, errors)
}

/// Replaces the header (and the generated heading) of the block with `meta.id`, keeping everything else.
pub fn rewrite_header(text: &str, meta: &EntryMeta) -> Result<Option<String>, Error> {
    let mut out = String::with_capacity(text.len() + 64);
    let mut found = false;
    let mut replace_heading = false;
    for line in text.split_inclusive('\n') {
        if replace_heading {
            replace_heading = false;
            if line.starts_with("### ") {
                out.push_str(&heading(meta));
                out.push('\n');
                continue;
            }
        }
        if let Some(Ok(existing)) = parse_header(line.trim_end_matches('\n'))
            && existing.id == meta.id
        {
            out.push_str(&header_line(meta)?);
            out.push('\n');
            found = true;
            replace_heading = true;
            continue;
        }
        out.push_str(line);
    }
    Ok(found.then_some(out))
}

/// Removes the block with `id` (header, heading, body). Returns `None` if it is not in the file.
pub fn remove_block(text: &str, id: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut found = false;
    let mut skipping = false;
    for line in text.split_inclusive('\n') {
        if let Some(parsed) = parse_header(line.trim_end_matches('\n')) {
            skipping = matches!(parsed, Ok(m) if m.id == id);
            found |= skipping;
        }
        if !skipping {
            out.push_str(line);
        }
    }
    found.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EntryKind, now};

    fn meta(id: &str) -> EntryMeta {
        EntryMeta {
            id: id.into(),
            kind: EntryKind::Progress,
            at: now(),
            item: Some("BIL-1".into()),
            session: None,
            agent: "claude-code".into(),
            cwd: None,
            refs: vec![],
        }
    }

    #[test]
    fn round_trip_blocks() {
        let mut text = file_header("Demo", "2026-11-17");
        text.push_str(&render_block(&meta("A"), "first line\n\nsecond para").unwrap());
        text.push_str(&render_block(&meta("B"), "<!-- t9 sneaky -->\nok").unwrap());
        let (blocks, errs) = parse(&text);
        assert!(errs.is_empty());
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].1, "first line\n\nsecond para");
        assert_eq!(blocks[1].1, "<!-- t9 sneaky -->\nok");
    }

    #[test]
    fn rewrite() {
        let mut text = file_header("Demo", "d");
        text.push_str(&render_block(&meta("A"), "a").unwrap());
        text.push_str(&render_block(&meta("B"), "b").unwrap());
        let mut m = meta("B");
        m.item = Some("BIL-9".into());
        let new = rewrite_header(&text, &m).unwrap().unwrap();
        let (blocks, _) = parse(&new);
        assert_eq!(blocks[1].0.item.as_deref(), Some("BIL-9"));
        assert_eq!(blocks[1].1, "b");
        assert!(new.contains("· BIL-9 ·"));
        assert!(rewrite_header(&text, &meta("Z")).unwrap().is_none());
    }
}
