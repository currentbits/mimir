//! One-click import from other agents on this computer: Claude Code (~/.claude), Hermes (~/.hermes) and OpenClaw
//! (~/.openclaw). Their memory files go through the pasted-memory import (suggestions), SKILL.md files become skill
//! proposals, and scheduled jobs become routines that start paused (resuming one is accepting it). Nothing comes in
//! active, no secrets come in, and nothing is granted: a routine runs with the bot's own tools, marked as outside content.

use crate::db::Db;
use crate::provider::Provider;
use anyhow::{Result, bail};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// (id, label) of the agents we know how to read.
pub const SOURCES: [(&str, &str); 3] = [("claude_code", "Claude Code"), ("hermes", "Hermes"), ("openclaw", "OpenClaw")];

/// A scheduled job from another agent: its name, its cron and what it asks.
struct Job {
    name: String,
    cron: String,
    prompt: String,
}

#[derive(Default)]
struct Found {
    memory: Vec<PathBuf>,
    skills: Vec<PathBuf>,
    jobs: Vec<Job>,
}

/// An agent found on this computer, with how much it has of each.
#[derive(Serialize)]
pub struct Source {
    pub id: &'static str,
    pub label: &'static str,
    pub memories: usize,
    pub skills: usize,
    pub routines: usize,
}

/// What one import queued.
#[derive(Serialize, Default, Debug)]
pub struct Imported {
    pub memories: usize,
    pub skills: usize,
    pub routines: usize,
}

/// The agents on this computer that have something to import.
pub fn sources() -> Vec<Source> {
    let Some(home) = crate::user_home() else { return vec![] };
    SOURCES
        .iter()
        .map(|&(id, label)| {
            let f = scan(&home, id);
            Source { id, label, memories: f.memory.len(), skills: f.skills.len(), routines: f.jobs.len() }
        })
        .filter(|s| s.memories + s.skills + s.routines > 0)
        .collect()
}

/// Import everything `source` has, for `bot` in `chat`: all of it as suggestions, proposals or paused routines.
pub async fn run(db: &Db, provider: &Provider, bot: i64, chat: i64, source: &str) -> Result<Imported> {
    let Some(&(_, label)) = SOURCES.iter().find(|s| s.0 == source) else { bail!("unknown source {source}") };
    let Some(home) = crate::user_home() else { bail!("no home folder") };
    let found = scan(&home, source);
    let mut done = Imported::default();
    // The memory files first: it's the part that can fail (a model call), and a retry is harmless (all deduplicated).
    let text = memory_text(&home, &found.memory);
    if !text.is_empty() {
        done.memories = crate::tidy::import(db, provider, bot, chat, &text).await?;
    }
    let (skills, routines) = bring_in(db, bot, chat, label, &home, &found)?;
    (done.skills, done.routines) = (skills, routines);
    Ok(done)
}

/// Memory files as one text, each under its path. The import reads only the first 20,000 characters, so what's about
/// the person goes first: their own instructions (CLAUDE.md, USER.md), then Claude Code memories typed `user` or
/// `feedback`, then the rest (project notes), newest first within each.
fn memory_text(home: &Path, files: &[PathBuf]) -> String {
    let mut read: Vec<(&PathBuf, String)> = files.iter().filter_map(|p| Some((p, std::fs::read_to_string(p).ok()?))).filter(|(_, t)| !t.trim().is_empty()).collect();
    read.sort_by_key(|(p, t)| {
        if p.ends_with("CLAUDE.md") || p.ends_with("USER.md") {
            0
        } else if t.contains("type: user") || t.contains("type: feedback") {
            1
        } else {
            2
        }
    });
    read.into_iter()
        .map(|(p, t)| format!("## {}\n{}", shown(home, p), crate::tools::truncate(t.trim(), 4_000)))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The model-free part: skills as proposals and jobs as paused routines. Returns how many of each are new.
fn bring_in(db: &Db, bot: i64, chat: i64, label: &str, home: &Path, found: &Found) -> Result<(usize, usize)> {
    let scope = format!("chat:{chat}");
    let mut skills = 0;
    for p in &found.skills {
        let Some((name, description, body)) = std::fs::read_to_string(p).ok().as_deref().and_then(crate::tidy::parse_skill_md) else { continue };
        // Already proposed, dismissed, too long or holding a secret: skipped, the rest still come in.
        match crate::tidy::skill_md_propose(db, &scope, bot, &format!("{label}, {}", shown(home, p)), &name, &description, &body) {
            Ok(Some(_)) => skills += 1,
            Ok(None) => {}
            Err(e) => eprintln!("import: skipped {}: {e:#}", p.display()),
        }
    }
    let have: Vec<String> = db.routines(chat)?.into_iter().map(|r| r.prompt).collect();
    let mut routines = 0;
    for j in &found.jobs {
        let prompt = format!("{} (imported from {label})\n\n{}", j.name, j.prompt.trim());
        if have.contains(&prompt) || crate::tools::looks_secret(db, &prompt).is_some() {
            continue;
        }
        let r = match db.routine_create(bot, chat, &j.cron, &prompt, false) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("import: skipped job {}: {e:#}", j.name);
                continue;
            }
        };
        // Paused until the person resumes it; someone else's words, so every run counts as having read outside content.
        db.routine_update(chat, r.id, &r.cron, &r.prompt, false, true)?;
        db.routine_taint(r.id)?;
        routines += 1;
    }
    Ok((skills, routines))
}

/// `~/.hermes/memories/USER.md` rather than the full path.
fn shown(home: &Path, p: &Path) -> String {
    // `/` on Windows too: the same label everywhere.
    p.strip_prefix(home).map_or_else(|_| p.display().to_string(), |r| format!("~/{}", r.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")))
}

fn scan(home: &Path, source: &str) -> Found {
    let dir = |env: &str, name: &str| std::env::var_os(env).filter(|v| !v.is_empty()).map_or_else(|| home.join(name), PathBuf::from);
    let mut f = Found::default();
    match source {
        "claude_code" => {
            let c = home.join(".claude");
            f.memory.push(c.join("CLAUDE.md"));
            // Auto memory, per project: one fact per file, MEMORY.md being only their index.
            for project in children(&c.join("projects")) {
                f.memory.extend(children(&project.join("memory")).into_iter().filter(|p| p.extension().is_some_and(|e| e == "md") && !p.ends_with("MEMORY.md")));
            }
            skill_files(&c.join("skills"), &[], 2, &mut f.skills);
        }
        "hermes" => {
            let h = dir("HERMES_HOME", ".hermes");
            f.memory.extend([h.join("memories/USER.md"), h.join("memories/MEMORY.md")]);
            // Skills Hermes ships with are listed in its manifest: only the person's own come in.
            let bundled: Vec<String> =
                std::fs::read_to_string(h.join("skills/.bundled_manifest")).unwrap_or_default().lines().filter_map(|l| Some(l.split(':').next()?.trim().to_string())).collect();
            skill_files(&h.join("skills"), &bundled, 4, &mut f.skills);
            f.jobs = jobs(&h.join("cron/jobs.json"));
        }
        "openclaw" => {
            let o = dir("OPENCLAW_STATE_DIR", ".openclaw");
            let w = o.join("workspace");
            f.memory.extend([w.join("USER.md"), w.join("MEMORY.md")]);
            f.memory.extend(children(&w.join("memory")).into_iter().filter(|p| p.extension().is_some_and(|e| e == "md")));
            skill_files(&w.join("skills"), &[], 2, &mut f.skills);
            skill_files(&o.join("skills"), &[], 2, &mut f.skills);
            f.jobs = jobs(&o.join("cron/jobs.json"));
        }
        _ => {}
    }
    f.memory.retain(|p| p.is_file());
    f.memory.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).and_then(|m| m.modified()).ok()));
    f
}

fn children(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default()
}

/// Every `SKILL.md` under `dir` (skills may sit in category folders), skipping hidden folders and `skip`ped names.
fn skill_files(dir: &Path, skip: &[String], depth: usize, out: &mut Vec<PathBuf>) {
    if dir.join("SKILL.md").is_file() {
        let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !skip.iter().any(|s| s == name) {
            out.push(dir.join("SKILL.md"));
        }
        return;
    }
    if depth == 0 {
        return;
    }
    let mut kids = children(dir);
    kids.sort();
    for d in kids.iter().filter(|p| p.is_dir() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))) {
        skill_files(d, skip, depth - 1, out);
    }
}

/// Cron jobs from a Hermes or OpenClaw `jobs.json` ({"jobs": [...]}). Only cron schedules: one-offs and plain
/// intervals are left out.
// ponytail: a job's own time zone is ignored (the routine runs on this computer's clock); convert when someone needs it
fn jobs(path: &Path) -> Vec<Job> {
    let v: serde_json::Value = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    v["jobs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|j| j["schedule"]["kind"] == "cron")
        .filter_map(|j| {
            let cron = j["schedule"]["expr"].as_str()?.trim().to_string();
            // Hermes: `prompt`, after the delivery note it puts in front. OpenClaw: the payload's message or text.
            let prompt = j["prompt"].as_str().or(j["payload"]["message"].as_str()).or(j["payload"]["text"].as_str())?;
            let prompt = match prompt.trim_start().strip_prefix("[IMPORTANT:").and_then(|r| r.split_once("]\n")) {
                Some((_, rest)) => rest.trim(),
                None => prompt.trim(),
            };
            let name = j["name"].as_str().filter(|n| !n.trim().is_empty()).unwrap_or("A scheduled job").trim().to_string();
            (!prompt.is_empty()).then(|| Job { name, cron, prompt: prompt.to_string() })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn hermes_and_claude_code_come_in_as_proposals() {
        let home = std::env::temp_dir().join(format!("mimir-imports-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let h = home.join(".hermes");
        put(&h.join("memories/USER.md"), "Lives in Utrecht\n§\nPrefers short answers");
        let skill = "---\nname: news-roundup\ndescription: When asked for the news\n---\n1. Read three sources\n2. Five bullets";
        put(&h.join("skills/research/news-roundup/SKILL.md"), skill);
        put(&h.join("skills/github/github-auth/SKILL.md"), &skill.replace("news-roundup", "github-auth"));
        put(&h.join("skills/.bundled_manifest"), "github-auth:abc123\n");
        put(
            &h.join("cron/jobs.json"),
            r#"{"jobs":[
                {"name":"Thai news","prompt":"[IMPORTANT: You are running as a scheduled cron job. Say [SILENT] if nothing.]\n\nFind the latest Thai news.","schedule":{"kind":"cron","expr":"0 9 * * *"}},
                {"name":"Once","prompt":"Ping","schedule":{"kind":"once","run_at":"2030-01-01T00:00:00"}}
            ]}"#,
        );
        let f = scan(&home, "hermes");
        assert_eq!(f.memory.len(), 1);
        assert_eq!(f.skills.len(), 1, "the bundled skill stays out");
        assert_eq!(f.jobs.len(), 1, "only cron schedules");
        assert_eq!(f.jobs[0].prompt, "Find the latest Thai news.");

        let p = home.join("test.db");
        let db = Db::open(&p).unwrap();
        assert_eq!(bring_in(&db, 1, 7, "Hermes", &home, &f).unwrap(), (1, 1));
        let r = &db.routines(7).unwrap()[0];
        assert!(r.paused && r.tainted, "a proposal: paused, and someone else's words");
        assert_eq!(r.cron, "0 9 * * *");
        assert_eq!(bring_in(&db, 1, 7, "Hermes", &home, &f).unwrap(), (0, 0), "importing again adds nothing");

        put(&home.join(".claude/CLAUDE.md"), "Always answer in English");
        put(&home.join(".claude/projects/-x/memory/MEMORY.md"), "- [a](a.md)");
        put(&home.join(".claude/projects/-x/memory/a.md"), "---\nname: a\n---\nThe build uses cargo");
        put(&home.join(".claude/projects/-x/memory/b.md"), "---\nname: b\nmetadata:\n  type: feedback\n---\nNo emoji, ever");
        let c = scan(&home, "claude_code");
        assert_eq!(c.memory.len(), 3, "the MEMORY.md index stays out");
        let text = memory_text(&home, &c.memory);
        assert!(text.starts_with("## ~/.claude/CLAUDE.md\nAlways answer in English"), "{text}");
        assert!(text.find("No emoji").unwrap() < text.find("cargo").unwrap(), "what's about the person before project notes");
        let _ = std::fs::remove_dir_all(&home);
    }
}
