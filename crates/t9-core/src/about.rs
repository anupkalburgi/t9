//! A project's About: what it is, goals, architecture, how to run it, key terms. It lives in the body of
//! `project.md` as `## Section` headings, so it reads well in any editor and outlives t9. Agents seed it from
//! the repository's README when a project is created and add durable facts as they learn them.

use serde::Serialize;

use crate::model::*;
use crate::store::Store;
use crate::{Error, Result, frontmatter};

/// Known sections, in display order: (key, title, aliases).
pub const SECTIONS: &[(&str, &str, &[&str])] = &[
    ("what", "What it is", &["what-it-is", "summary", "overview", "purpose"]),
    ("goals", "Goals", &["goal"]),
    ("non-goals", "Non-goals", &["nongoals", "out-of-scope"]),
    ("architecture", "Architecture", &["arch", "design"]),
    ("stack", "Stack", &["tech", "technology", "tools"]),
    ("run", "Build, test and run", &["build", "test", "how-to-run", "setup", "dev"]),
    ("data", "Data and resources", &["datasets", "resources", "models"]),
    ("terms", "Key terms", &["glossary", "terminology"]),
    ("constraints", "Constraints", &["limits", "rules"]),
    ("links", "Links", &["link", "references", "docs"]),
];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AboutSection {
    /// Known key (`stack`), or the heading itself for sections a human added.
    pub key: String,
    pub title: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct About {
    pub project: String,
    pub name: String,
    pub sections: Vec<AboutSection>,
    pub reviewed: Option<Timestamp>,
    pub path: String,
}

impl About {
    pub fn is_empty(&self) -> bool {
        self.sections.iter().all(|s| s.text.trim().is_empty())
    }
    pub fn get(&self, key: &str) -> Option<&AboutSection> {
        self.sections.iter().find(|s| s.key == key)
    }
}

/// Resolves `stack`, `Stack`, `tech`, `"Build, test and run"` … to a known section.
pub fn section(name: &str) -> Result<(&'static str, &'static str)> {
    let n = slugify(name);
    for (key, title, aliases) in SECTIONS {
        if n == *key || n == slugify(title) || aliases.contains(&n.as_str()) {
            return Ok((key, title));
        }
    }
    Err(Error::Invalid(format!(
        "unknown section '{name}' (one of: {})",
        SECTIONS.iter().map(|(k, _, _)| *k).collect::<Vec<_>>().join(", ")
    )))
}

/// Splits a project body into its sections. Text before the first `## ` heading (other than the `# Name`
/// title) is treated as "What it is", so older free-text descriptions keep working.
pub fn parse(body: &str) -> Vec<AboutSection> {
    let mut out: Vec<AboutSection> = vec![];
    let mut preamble: Vec<&str> = vec![];
    let mut current: Option<(String, Vec<&str>)> = None;
    for line in body.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            if let Some((t, lines)) = current.take() {
                out.push(mk(&t, &lines));
            }
            current = Some((h.trim().to_string(), vec![]));
        } else if let Some((_, lines)) = current.as_mut() {
            lines.push(line);
        } else if !line.starts_with("# ") {
            preamble.push(line);
        }
    }
    if let Some((t, lines)) = current.take() {
        out.push(mk(&t, &lines));
    }
    let pre = preamble.join("\n").trim().to_string();
    if !pre.is_empty() && !out.iter().any(|s| s.key == "what") {
        out.insert(0, AboutSection { key: "what".into(), title: "What it is".into(), text: pre });
    }
    out
}

fn mk(heading: &str, lines: &[&str]) -> AboutSection {
    let text = lines.join("\n").trim().to_string();
    match section(heading) {
        Ok((key, title)) => AboutSection { key: key.into(), title: title.into(), text },
        Err(_) => AboutSection { key: heading.into(), title: heading.into(), text },
    }
}

/// Renders sections back to a body: known sections in canonical order, then any others as they were.
pub fn render(name: &str, sections: &[AboutSection]) -> String {
    let mut out = format!("# {name}\n");
    let known = SECTIONS.iter().filter_map(|(k, _, _)| sections.iter().find(|s| s.key == *k));
    let other = sections.iter().filter(|s| !SECTIONS.iter().any(|(k, _, _)| *k == s.key));
    for s in known.chain(other) {
        if s.text.trim().is_empty() {
            continue;
        }
        out.push_str(&format!("\n## {}\n\n{}\n", s.title, s.text.trim_end()));
    }
    out
}

impl Store {
    fn project_file(&self, key: &str) -> Result<(Project, std::path::PathBuf)> {
        let p = self.project(key)?;
        let path = self.env.projects_dir().join(&p.slug).join("project.md");
        Ok((p, path))
    }

    pub fn about(&mut self, key: &str) -> Result<About> {
        self.sync_projects()?;
        let (p, path) = self.project_file(key)?;
        let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        let (_, body) = frontmatter::parse::<ProjectMeta>(&text, &path.to_string_lossy())?;
        Ok(About {
            project: p.meta.key.clone(),
            name: p.meta.name.clone(),
            sections: parse(&body),
            reviewed: p.meta.about_reviewed,
            path: path.to_string_lossy().into_owned(),
        })
    }

    /// Changes the About under the project lock and marks it reviewed.
    fn edit_about(&mut self, key: &str, f: impl FnOnce(&mut Vec<AboutSection>)) -> Result<About> {
        let (p, path) = self.project_file(key)?;
        {
            let _lock = self.lock(&p.meta.key)?;
            let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
            let (mut meta, body) = frontmatter::parse::<ProjectMeta>(&text, &path.to_string_lossy())?;
            let mut sections = parse(&body);
            f(&mut sections);
            meta.about_reviewed = Some(now());
            let body = render(&meta.name, &sections);
            self.put(&path, &frontmatter::render(&meta, &body)?)?;
        }
        self.sync_projects()?;
        self.about(key)
    }

    /// Adds a fact to a section, as a bullet.
    pub fn about_add(&mut self, key: &str, section_name: &str, text: &str) -> Result<About> {
        let (k, title) = section(section_name)?;
        let text = text.trim();
        if text.is_empty() {
            return Err(Error::Invalid("nothing to add".into()));
        }
        if let Some(what) = looks_like_secret(text) {
            return Err(Error::Invalid(format!("this looks like it contains {what}; t9 does not store credentials")));
        }
        let line = if text.starts_with("- ") { text.to_string() } else { format!("- {text}") };
        self.edit_about(key, |sections| match sections.iter_mut().find(|s| s.key == k) {
            Some(s) if !s.text.trim().is_empty() => s.text = format!("{}\n{line}", s.text.trim_end()),
            Some(s) => s.text = line,
            None => sections.push(AboutSection { key: k.into(), title: title.into(), text: line }),
        })
    }

    /// Replaces a section (empty text removes it).
    pub fn about_set(&mut self, key: &str, section_name: &str, text: &str) -> Result<About> {
        let (k, title) = section(section_name)?;
        if let Some(what) = looks_like_secret(text) {
            return Err(Error::Invalid(format!("this looks like it contains {what}; t9 does not store credentials")));
        }
        let text = text.trim().to_string();
        self.edit_about(key, |sections| match sections.iter_mut().find(|s| s.key == k) {
            Some(s) => s.text = text,
            None => sections.push(AboutSection { key: k.into(), title: title.into(), text }),
        })
    }

    /// Marks the About as reviewed without changing it.
    pub fn about_confirm(&mut self, key: &str) -> Result<About> {
        self.edit_about(key, |_| {})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_render() {
        let body = "# Billing\n\nOld free-text description.\n\n## Stack\n\n- Rust\n\n## Our own notes\n\nkeep me\n";
        let s = parse(body);
        assert_eq!(s[0].key, "what");
        assert_eq!(s[0].text, "Old free-text description.");
        assert_eq!(s[1].key, "stack");
        assert_eq!(s[2].key, "Our own notes");
        let out = render("Billing", &s);
        assert!(out.starts_with("# Billing\n\n## What it is\n\nOld free-text description.\n\n## Stack\n\n- Rust\n"));
        assert!(out.ends_with("## Our own notes\n\nkeep me\n"));
        assert_eq!(parse(&out), s);
    }

    #[test]
    fn section_names() {
        assert_eq!(section("tech").unwrap().0, "stack");
        assert_eq!(section("Build, test and run").unwrap().0, "run");
        assert!(section("nope").is_err());
    }
}
