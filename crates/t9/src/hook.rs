//! Hook entry points for agent harnesses. They read the harness's JSON payload from stdin and must never break
//! the agent's session: failures are reported on stderr and the hook still exits 0.

use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::Result;
use clap::Subcommand;
use serde::Deserialize;
use t9_core::{Caller, Store, context};

#[derive(Subcommand)]
pub enum HookCmd {
    /// Claude Code: `session-start` (prints the brief), `prompt` (recalls decisions for the user's message) or
    /// `session-end`.
    Claude {
        event: String,
        #[arg(long, default_value_t = 1500)]
        budget: usize,
    },
}

#[derive(Deserialize, Default)]
struct ClaudePayload {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<PathBuf>,
    /// SessionStart: startup | resume | clear | compact
    #[serde(default)]
    source: Option<String>,
    /// UserPromptSubmit: the user's message.
    #[serde(default)]
    prompt: Option<String>,
}

const AGENT_CLAUDE: &str = "claude-code";

pub fn run(store: &mut Store, caller: &Caller, cmd: HookCmd) -> Result<()> {
    if let Err(e) = run_inner(store, caller, cmd) {
        eprintln!("t9 hook: {e:#}");
    }
    Ok(())
}

fn run_inner(store: &mut Store, caller: &Caller, cmd: HookCmd) -> Result<()> {
    let HookCmd::Claude { event, budget } = cmd;
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let payload: ClaudePayload =
        if raw.trim().is_empty() { ClaudePayload::default() } else { serde_json::from_str(&raw)? };
    let caller = Caller {
        cwd: payload.cwd.clone().unwrap_or_else(|| caller.cwd.clone()),
        agent: Some(AGENT_CLAUDE.into()),
        ..caller.clone()
    };
    match event.as_str() {
        "session-start" => {
            let session = store.start_session(&caller, payload.session_id.as_deref())?;
            // Later Bash tool calls in this session see these variables (Claude Code's CLAUDE_ENV_FILE).
            if let Some(env_file) = std::env::var_os("CLAUDE_ENV_FILE") {
                let mut f = std::fs::OpenOptions::new().create(true).append(true).open(env_file)?;
                writeln!(f, "export T9_SESSION={}\nexport T9_AGENT={AGENT_CLAUDE}", session.meta.id)?;
            }
            let caller = Caller { session: Some(session.meta.id.clone()), ..caller };
            let brief = context::build(store, &caller)?;
            let mut out = String::new();
            if payload.source.as_deref() == Some("compact") {
                out.push_str("(The conversation was compacted. This is the current t9 brief for this project.)\n\n");
            }
            out.push_str(&context::render(&brief, budget));
            // What the brief already listed is not repeated by the message hook today.
            let harness = payload.session_id.clone().unwrap_or_else(|| "default".into());
            for a in brief.attention.reminders() {
                store.first_time_for(&reminder_key(a), &harness)?;
            }
            out.push_str(
                "\n---\nt9 is this project's shared memory. As you work: `t9 focus ID` (or `t9 add task \"…\" --focus`) \
                 when starting a unit of work; `t9 log \"…\"` after meaningful steps; `t9 add bug|regression|revisit|idea \"…\"` \
                 for problems, loose ends and ideas. Decisions: before proposing an approach or asking the user to choose, \
                 run `t9 check \"topic\"`; when the user decides or rules something out, record it at once with \
                 `t9 decide \"…\" --because \"…\" [-r \"alternative: why not\"] [--area A] [--for ID]` or `t9 reject \"…\"`. \
                 Close work with its outcome: `t9 done ID -n \"what came out of it\"`. If \"Needs attention\" lists overdue, \
                 due, ripe or waiting items, mention the ones that concern the user in your first reply. \
                 `t9 handoff \"…\"` before you stop. Use the t9 skill for details.\n",
            );
            print!("{out}");
        }
        "prompt" => {
            // Stdout of a UserPromptSubmit hook is added to the agent's context; print nothing unless useful.
            if store.resolve(&caller)?.project.is_none() {
                // Once per session: make sure the user hears that this folder has no project yet.
                let session = payload.session_id.clone().unwrap_or_else(|| "default".into());
                if store.first_time_for("unlinked", &session)? {
                    store.note_unlinked(&caller.cwd, AGENT_CLAUDE)?;
                    let (name, key) = store.suggested_project(&caller.cwd)?;
                    println!(
                        "t9: this folder is not linked to a t9 project, so notes go to the inbox. If you have not \
                         told the user yet, tell them now and suggest `{}` (or `t9 project link KEY` for an \
                         existing project); run it once they agree.",
                        t9_core::unlinked::init_command(&name, &key)
                    );
                }
            }
            // Timely reminders: something became due, overdue, ripe or has waited too long since it was last
            // shown in this session (each item and reason once per session per day).
            if let Some(p) = store.resolve(&caller)?.project {
                let harness = payload.session_id.clone().unwrap_or_else(|| "default".into());
                let report = store.attention(&p.meta.key, None)?;
                let mut lines = vec![];
                for a in report.reminders() {
                    if lines.len() < 3 && store.first_time_for(&reminder_key(a), &harness)? {
                        lines.push(format!(
                            "- {} {}: {} — {}",
                            a.id,
                            a.detail,
                            t9_core::context::first_line(&a.title, 90),
                            a.hint
                        ));
                    }
                }
                if !lines.is_empty() {
                    println!(
                        "t9 reminder (new since you last looked; mention it to the user briefly if it concerns them, \
                         then carry on with their request):\n{}",
                        lines.join("\n")
                    );
                }
            }
            if let Some(text) =
                t9_core::decisions::prompt_context(store, &caller, payload.prompt.as_deref().unwrap_or(""))?
            {
                print!("{text}");
            }
        }
        "session-end" => {
            store.end_session(&caller, None)?;
        }
        other => anyhow::bail!("unknown Claude hook event '{other}' (expected session-start, prompt or session-end)"),
    }
    Ok(())
}

/// Once per item, reason and local day: a new day re-surfaces what is still overdue.
fn reminder_key(a: &t9_core::attention::Attention) -> String {
    format!("remind-{}-{}-{}", a.id, a.reason, chrono::Local::now().format("%Y%m%d"))
}
