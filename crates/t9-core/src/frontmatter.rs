//! YAML front matter: `---\n<yaml>---\n\n<body>`.
//!
//! Reading and writing are inverse: `parse(render(m, b)) == (m, b)`, and rewriting the front matter of a file
//! leaves its body byte-for-byte unchanged.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::Error;

/// Splits a document into (yaml, body). Returns `None` if it has no front matter.
pub fn split(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n"))?;
    // The closing fence is a line that is exactly `---`.
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            let yaml = &rest[..offset];
            let mut body = &rest[offset + line.len()..];
            // One blank separator line is part of the format, not the body.
            body = body.strip_prefix("\r\n").or_else(|| body.strip_prefix('\n')).unwrap_or(body);
            return Some((yaml, body));
        }
        offset += line.len();
    }
    None
}

pub fn parse<T: DeserializeOwned>(text: &str, what: &str) -> Result<(T, String), Error> {
    let (yaml, body) = split(text).ok_or_else(|| Error::Parse(format!("{what}: missing front matter")))?;
    let meta = serde_norway::from_str(yaml).map_err(|e| Error::Parse(format!("{what}: {e}")))?;
    Ok((meta, body.to_string()))
}

pub fn render<T: Serialize>(meta: &T, body: &str) -> Result<String, Error> {
    let yaml = serde_norway::to_string(meta).map_err(|e| Error::Parse(e.to_string()))?;
    let mut out = String::with_capacity(yaml.len() + body.len() + 10);
    out.push_str("---\n");
    out.push_str(&yaml);
    if !yaml.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("---\n\n");
    out.push_str(body);
    if !body.is_empty() && !body.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn round_trip() {
        let mut m = BTreeMap::new();
        m.insert("id".to_string(), "BIL-1".to_string());
        let body = "Hello\n\n---\nnot a fence inside body? it is after the first fence, fine\n";
        let text = render(&m, body).unwrap();
        let (m2, b2): (BTreeMap<String, String>, String) = parse(&text, "t").unwrap();
        assert_eq!(m, m2);
        assert_eq!(b2, body);
    }

    #[test]
    fn no_front_matter() {
        assert!(split("# hi\n").is_none());
        assert!(split("---\nid: 1\n").is_none());
    }

    #[test]
    fn empty_body() {
        let (y, b) = split("---\na: 1\n---\n").unwrap();
        assert_eq!(y, "a: 1\n");
        assert_eq!(b, "");
    }
}
