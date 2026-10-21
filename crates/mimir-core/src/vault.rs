//! Memories kept as files too, in a folder you choose (open it in Obsidian, grep it, keep it in git): one Markdown
//! file per memory, a folder per chat. Mimir writes them; what you change there comes back as a suggestion in Needs
//! you, never applied on its own, since a file in a synced folder is someone else's words until you accept it.
//! Guests' chats and incognito ones stay out.

use crate::db::{Db, EntryRow, Suggestion, TYPES};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The folder (empty or missing: off).
pub const SETTING: &str = "memory_folder";
/// What each file held when Mimir last wrote or read it, so a change of yours is seen once.
const STATE: &str = "memory_folder_state";
/// Reason prefixes of the suggestions a change in the folder makes.
pub const EDITED: &str = "Edited in your memory folder";
pub const DELETED: &str = "Deleted from your memory folder";
const MAX_FILE: u64 = 100_000;

#[derive(Serialize, Deserialize, Clone)]
struct Seen {
    /// The memory it mirrors (None: a note you made there).
    entry: Option<i64>,
    scope: String,
    hash: String,
}

pub fn folder(db: &Db) -> Option<PathBuf> {
    db.setting(SETTING).ok().flatten().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).map(PathBuf::from)
}

/// A suggestion that came from the folder (it's yours to accept, not the tidy-up's).
pub fn from_folder(reason: &str) -> bool {
    reason.starts_with(EDITED) || reason.starts_with(DELETED)
}

/// Use `dir` from now on (None: stop). Files already there are yours: they're left alone, not suggested.
pub fn set(db: &Db, dir: Option<&str>) -> Result<()> {
    let dir = dir.map(str::trim).unwrap_or("");
    if !dir.is_empty() {
        if !Path::new(dir).is_absolute() {
            anyhow::bail!("give the folder's full path");
        }
        std::fs::create_dir_all(dir)?;
    }
    db.set_setting(SETTING, dir)?;
    db.set_setting(STATE, "")?;
    sync(db)?;
    Ok(())
}

fn hash(s: &str) -> String {
    format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(s.as_bytes()))
}

/// A name a file can have on macOS, Windows and in Obsidian (no # ^ [ ] | either: they break links).
fn file_name(s: &str) -> String {
    let n: String = s.chars().map(|c| if "/\\:*?\"<>|#^[]".contains(c) || c.is_control() { '-' } else { c }).take(80).collect();
    let n = n.trim().trim_matches('.').trim().to_string();
    if n.is_empty() { "memory".into() } else { n }
}

/// (id, type, title, body) of a memory file: frontmatter, then "# Title", then the text. No heading: the file's name.
fn parse(text: &str, stem: &str) -> (Option<i64>, Option<String>, String, String) {
    let text = text.replace("\r\n", "\n");
    let (front, rest) = match text.strip_prefix("---\n").and_then(|t| t.split_once("\n---")) {
        Some((f, r)) => (f.to_string(), r.trim_start_matches('-').to_string()),
        None => (String::new(), text.clone()),
    };
    let field = |k: &str| front.lines().find_map(|l| l.strip_prefix(k)?.strip_prefix(':').map(|v| v.trim().trim_matches('"').to_string()));
    let rest = rest.trim();
    let (title, body) = match rest.strip_prefix("# ") {
        Some(r) => r.split_once('\n').map(|(t, b)| (t.trim().to_string(), b.trim().to_string())).unwrap_or((r.trim().to_string(), String::new())),
        None => (stem.to_string(), rest.to_string()),
    };
    (field("id").and_then(|v| v.parse().ok()), field("type"), title, body)
}

fn render(e: &EntryRow, chat: &str, bots: &HashMap<i64, String>) -> String {
    let day = |t: i64| chrono::TimeZone::timestamp_opt(&chrono::Local, t, 0).single().map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default();
    let q = |s: &str| serde_json::to_string(s).unwrap_or_default();
    let mut f = format!("---\nid: {}\ntype: {}\nchat: {}\nsaved: {}\n", e.id, e.kind, q(chat), day(e.created_at));
    if e.for_bot != 0 {
        f += &format!("bot: {}\n", q(bots.get(&e.for_bot).map_or("one bot", String::as_str)));
    }
    if let Some(t) = e.valid_until {
        f += &format!("until: {}\n", day(t));
    }
    if e.pinned {
        f += "pinned: true\n";
    }
    if e.tainted {
        f += "unconfirmed: true\n";
    }
    format!("{f}---\n# {}\n\n{}\n", e.title.trim(), e.body.trim())
}

/// Every .md file under `dir` (not in hidden folders like .obsidian or .git; symlinks aren't followed).
fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for d in rd.flatten() {
        let name = d.file_name().to_string_lossy().to_string();
        let Ok(t) = d.file_type() else { continue };
        let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        if name.starts_with('.') {
            continue;
        }
        if t.is_dir() {
            walk(&d.path(), &r, out);
        } else if t.is_file() && name.ends_with(".md") {
            out.push(r);
        }
    }
}

/// One pass: your changes become suggestions, then the files catch up with memory. Returns suggestions made.
// ponytail: whole-folder pass every 30s; a file watcher if folders grow past a few thousand files
pub fn sync(db: &Db) -> Result<usize> {
    let Some(dir) = folder(db) else { return Ok(0) };
    if !dir.is_dir() {
        return Ok(0);
    }
    let raw = db.setting(STATE)?.unwrap_or_default();
    // The first pass in a folder only notes what's already there.
    let first = raw.is_empty();
    let mut state: BTreeMap<String, Seen> = serde_json::from_str(&raw).unwrap_or_default();

    // Your own chats and the household, a folder each (named after the chat).
    let bots: HashMap<i64, String> = db.bots()?.into_iter().map(|b| (b.id, b.name)).collect();
    let mut chats: Vec<(String, String)> = vec![];
    let mut list = db.chats()?;
    // The app's own chat has memories before it has a row.
    if !list.iter().any(|c| c.chat_id == crate::host::LOCAL_CHAT) {
        list.insert(0, crate::db::Chat { chat_id: crate::host::LOCAL_CHAT, title: "Mimir".into(), channel: "local".into(), memories: 0, guest: false, household: true, family: false, private: true });
    }
    for c in list.into_iter().filter(|c| !c.guest && c.chat_id != crate::INCOGNITO_CHAT) {
        let mut name = file_name(&c.title);
        if chats.iter().any(|(_, n)| n.eq_ignore_ascii_case(&name)) {
            name = format!("{name} ({})", c.chat_id);
        }
        chats.push((format!("chat:{}", c.chat_id), name));
    }
    let mut active: HashMap<i64, (String, String, EntryRow)> = HashMap::new();
    for (scope, name) in &chats {
        for e in db.entries(scope)?.into_iter().filter(|e| e.status == "active") {
            active.insert(e.id, (scope.clone(), name.clone(), e));
        }
    }
    let scope_of = |rel: &str| -> String {
        let top = rel.split_once('/').map(|(t, _)| t);
        let local = format!("chat:{}", crate::host::LOCAL_CHAT);
        top.and_then(|t| chats.iter().find(|(_, n)| n.eq_ignore_ascii_case(t))).map(|(s, _)| s.clone()).unwrap_or(local)
    };
    let mut made = 0;
    let mut suggest = |scope: &str, s: Suggestion| match db.suggestion_add(scope, &s) {
        Ok(Some(_)) => made += 1,
        Ok(None) => {}
        Err(e) => eprintln!("memory folder: {e:#}"),
    };

    // 1. What you changed or added.
    let mut files = vec![];
    walk(&dir, "", &mut files);
    for rel in &files {
        let path = dir.join(rel);
        if std::fs::metadata(&path).map_or(true, |m| m.len() > MAX_FILE) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let h = hash(&text);
        let seen = state.get(rel).cloned();
        if seen.as_ref().is_some_and(|s| s.hash == h) {
            continue;
        }
        let stem = Path::new(rel).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let (id, kind, title, body) = parse(&text, &stem);
        let entry = seen.as_ref().map_or(id, |s| s.entry).and_then(|id| active.get(&id));
        let scope = entry.map(|e| e.0.clone()).or_else(|| seen.as_ref().map(|s| s.scope.clone())).unwrap_or_else(|| scope_of(rel));
        let kind = kind.filter(|k| TYPES.contains(&k.as_str()));
        let reason = format!("{EDITED} ({rel}).");
        if !first && !title.is_empty() && !body.is_empty() {
            match entry {
                Some((_, _, e)) if e.title == title && e.body == body && kind.as_ref().is_none_or(|k| *k == e.kind) => {}
                Some((_, _, e)) => suggest(&scope, Suggestion { action: "update".into(), entry_id: Some(e.id), kind: kind.unwrap_or(e.kind.clone()), title, body, reason, ..sugg() }),
                None => suggest(&scope, Suggestion { action: "add".into(), kind: kind.unwrap_or("knowledge".into()), title, body, reason, ..sugg() }),
            }
        }
        state.insert(rel.clone(), Seen { entry: entry.map(|e| e.2.id), scope, hash: h });
    }

    // 2. Mimir's files you deleted: forget it? Your notes you deleted are just gone.
    let on_disk: HashSet<&String> = files.iter().collect();
    for (rel, s) in state.clone() {
        if on_disk.contains(&rel) {
            continue;
        }
        if let Some((scope, _, e)) = s.entry.and_then(|id| active.get(&id)) {
            suggest(scope, Suggestion { action: "forget".into(), entry_id: Some(e.id), kind: e.kind.clone(), reason: format!("{DELETED} ({rel})."), ..sugg() });
        }
        state.remove(&rel);
    }

    // A memory with a change of yours waiting keeps your file as it is until you decide.
    let mut waiting: HashSet<i64> = HashSet::new();
    for (scope, _) in &chats {
        waiting.extend(db.suggestions(scope)?.into_iter().filter(|s| from_folder(&s.reason)).filter_map(|s| s.entry_id));
    }

    // 3. Files of memories that were forgotten or replaced go (a newer version gets its own).
    for (rel, s) in state.clone() {
        if s.entry.is_some_and(|id| !active.contains_key(&id)) {
            let _ = std::fs::remove_file(dir.join(&rel));
            state.remove(&rel);
        }
    }

    // 4. Every current memory as a file.
    let mut by_entry: HashMap<i64, String> = state.iter().filter_map(|(r, s)| Some((s.entry?, r.clone()))).collect();
    let mut ids: Vec<&i64> = active.keys().collect();
    ids.sort();
    for id in ids {
        let (scope, chat, e) = &active[id];
        if waiting.contains(id) {
            continue;
        }
        let text = render(e, chat, &bots);
        let h = hash(&text);
        let rel = match by_entry.get(id) {
            Some(r) if state.get(r).is_some_and(|s| s.hash == h) => continue,
            Some(r) => r.clone(),
            None => {
                let base = format!("{chat}/{}", file_name(&e.title));
                let free = |r: &str| match state.get(r) {
                    // A note of yours saying just this (a suggestion you accepted) becomes its file.
                    None => !dir.join(r).exists(),
                    Some(s) => s.entry.is_none() && std::fs::read_to_string(dir.join(r)).is_ok_and(|t| {
                        let (_, _, title, body) = parse(&t, Path::new(r).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default().as_str());
                        title == e.title && body == e.body
                    }),
                };
                let plain = format!("{base}.md");
                if free(&plain) { plain } else { format!("{base} ({id}).md") }
            }
        };
        let path = dir.join(&rel);
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&path, &text)?;
        by_entry.insert(*id, rel.clone());
        state.insert(rel, Seen { entry: Some(*id), scope: scope.clone(), hash: h });
    }
    db.set_setting(STATE, &serde_json::to_string(&state)?)?;
    Ok(made)
}

fn sugg() -> Suggestion {
    Suggestion { id: 0, action: String::new(), entry_id: None, kind: "knowledge".into(), title: String::new(), body: String::new(), reason: String::new(), check: None, tainted: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_come_back_as_suggestions() {
        let t = std::env::temp_dir().join(format!("mimir-vault-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(&t).unwrap();
        let d = Db::open(&t.join("m.db")).unwrap();
        let s = format!("chat:{}", crate::host::LOCAL_CHAT);
        d.store_by(&s, "knowledge", "Dentist", "Dr. Smit, Tuesdays", "", None, &crate::db::Prov::by("you")).unwrap();
        let dir = t.join("vault");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Old note.md"), "already here").unwrap();
        set(&d, Some(dir.to_str().unwrap())).unwrap();
        let chat = "Mimir";
        let file = dir.join(file_name(chat)).join("Dentist.md");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("type: knowledge") && text.contains("# Dentist\n\nDr. Smit, Tuesdays"), "{text}");
        assert!(d.suggestions(&s).unwrap().is_empty(), "files already there are yours, not suggestions");
        assert_eq!(sync(&d).unwrap(), 0, "nothing changed");

        // An edit: a suggestion, the file left as you wrote it until you decide; dismissed, Mimir's version is back.
        std::fs::write(&file, text.replace("Tuesdays", "Wednesdays")).unwrap();
        assert_eq!(sync(&d).unwrap(), 1);
        let sg = d.suggestions(&s).unwrap();
        assert_eq!((sg[0].action.as_str(), sg[0].body.as_str()), ("update", "Dr. Smit, Wednesdays"));
        assert!(std::fs::read_to_string(&file).unwrap().contains("Wednesdays"));
        d.suggestion_decide(&s, sg[0].id, false).unwrap();
        sync(&d).unwrap();
        assert!(std::fs::read_to_string(&file).unwrap().contains("Tuesdays"));
        // Edited again after a dismissal: still asked. Accepted: the new version is the file.
        std::fs::write(&file, text.replace("Tuesdays", "Thursdays")).unwrap();
        assert_eq!(sync(&d).unwrap(), 1);
        let sg = d.suggestions(&s).unwrap();
        assert!(d.suggestion_decide(&s, sg[0].id, true).unwrap().contains("replaced"));
        sync(&d).unwrap();
        let now = std::fs::read_to_string(&file).unwrap();
        assert!(now.contains("Thursdays") && !now.contains(&format!("id: {}\n", sg[0].entry_id.unwrap())), "{now}");

        // A new note in the chat's folder: an "add"; accepted, the note becomes its file.
        let note = dir.join(file_name(chat)).join("Bins.md");
        std::fs::write(&note, "# Bins\n\nGreen bin on Mondays").unwrap();
        assert_eq!(sync(&d).unwrap(), 1);
        let sg = d.suggestions(&s).unwrap();
        assert_eq!(sg[0].action, "add");
        d.suggestion_decide(&s, sg[0].id, true).unwrap();
        sync(&d).unwrap();
        assert!(std::fs::read_to_string(&note).unwrap().starts_with("---\nid: "));
        assert!(!dir.join(file_name(chat)).read_dir().unwrap().any(|f| f.unwrap().file_name().to_string_lossy().contains('(')), "no second copy");

        // Deleted: a "forget" suggestion; accepted, it's gone from memory.
        std::fs::remove_file(&note).unwrap();
        assert_eq!(sync(&d).unwrap(), 1);
        let sg = d.suggestions(&s).unwrap();
        assert_eq!(sg[0].action, "forget");
        d.suggestion_decide(&s, sg[0].id, true).unwrap();
        sync(&d).unwrap();
        assert!(!note.exists() && dir.join("Old note.md").exists());
    }
}
