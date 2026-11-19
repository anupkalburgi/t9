//! `t9 install`: writes the harness integrations that are embedded in the binary.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Subcommand;
use t9_core::env::{display_path, expand_tilde};

const MARKETPLACE: &str = include_str!("../../../integrations/claude/.claude-plugin/marketplace.json");
const PLUGIN: &str = include_str!("../../../integrations/claude/t9/.claude-plugin/plugin.json");
const HOOKS: &str = include_str!("../../../integrations/claude/t9/hooks/hooks.json");
const HOOK_SH: &str = include_str!("../../../integrations/claude/t9/hooks/t9-hook.sh");
const README: &str = include_str!("../../../integrations/claude/t9/README.md");
const SKILL: &str = include_str!("../../../integrations/claude/t9/skills/t9/SKILL.md");
const AGENTS_MD: &str = include_str!("../../../integrations/agents-md/t9.md");

const BEGIN: &str = "<!-- t9:begin -->";
const END: &str = "<!-- t9:end -->";

#[derive(Subcommand)]
pub enum InstallCmd {
    /// Write the Claude Code plugin (a local marketplace) and print the commands that add it.
    Claude {
        /// Where to write it. Default: ~/.t9/integrations/claude.
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Add or update the t9 section of an AGENTS.md (Codex and other agents that read one).
    AgentsMd {
        /// Default: ~/.codex/AGENTS.md (Codex's global instructions).
        #[arg(long)]
        file: Option<PathBuf>,
        /// Only print the section.
        #[arg(long)]
        print: bool,
    },
}

fn write(path: &Path, content: &str) -> Result<()> {
    if let Some(d) = path.parent() {
        fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
    }
    fs::write(path, content).with_context(|| format!("writing {}", path.display()))
}

/// Replaces the marked t9 section of `text`, or appends it.
pub fn upsert_section(text: &str, section: &str) -> String {
    let section = section.trim_end();
    if let (Some(b), Some(e)) = (text.find(BEGIN), text.find(END))
        && b < e
    {
        return format!("{}{section}{}", &text[..b], &text[e + END.len()..]);
    }
    let mut out = text.trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(section);
    out.push('\n');
    out
}

pub fn run(env: &t9_core::Env, cmd: InstallCmd) -> Result<()> {
    match cmd {
        InstallCmd::Claude { dir } => {
            let dir = dir.unwrap_or_else(|| env.integrations_dir().join("claude"));
            write(&dir.join(".claude-plugin/marketplace.json"), MARKETPLACE)?;
            let p = dir.join("t9");
            write(&p.join(".claude-plugin/plugin.json"), PLUGIN)?;
            write(&p.join("hooks/hooks.json"), HOOKS)?;
            write(&p.join("hooks/t9-hook.sh"), HOOK_SH)?;
            write(&p.join("skills/t9/SKILL.md"), SKILL)?;
            write(&p.join("README.md"), README)?;
            let shown = display_path(&dir);
            println!("Wrote the t9 Claude Code plugin to {shown}\n");
            println!("Add it from a shell:");
            println!("  claude plugin marketplace add {}", dir.display());
            println!("  claude plugin install t9@t9\n");
            println!("or inside Claude Code: /plugin marketplace add {shown}, then /plugin install t9@t9");
            println!("Re-run `t9 install claude` after upgrading t9, then `claude plugin marketplace update t9`.");
        }
        InstallCmd::AgentsMd { file, print } => {
            if print {
                print!("{AGENTS_MD}");
                return Ok(());
            }
            let file = file.unwrap_or_else(|| expand_tilde("~/.codex/AGENTS.md"));
            let existing = fs::read_to_string(&file).unwrap_or_default();
            let updated = upsert_section(&existing, AGENTS_MD);
            if updated == existing {
                println!("{} already has the current t9 section", display_path(&file));
            } else {
                write(&file, &updated)?;
                println!("Updated the t9 section in {}", display_path(&file));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_is_appended_then_replaced() {
        let once = upsert_section("# Mine\n", AGENTS_MD);
        assert!(once.starts_with("# Mine\n\n<!-- t9:begin -->"));
        assert_eq!(upsert_section(&once, AGENTS_MD), once);
        let edited = once.replace("First thing in every session", "OLD");
        let fixed = upsert_section(&edited, AGENTS_MD);
        assert_eq!(fixed, once);
    }

    #[test]
    fn embedded_json_is_valid() {
        for j in [MARKETPLACE, PLUGIN, HOOKS] {
            serde_json::from_str::<serde_json::Value>(j).unwrap();
        }
    }
}
