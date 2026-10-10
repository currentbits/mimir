//! A file workspace per chat, like memory (files from a private chat can't be read in a group): notes, lists, drafts
//! and research too long for a memory. Plain text only, inside the chat's folder (no "..", no absolute paths, no
//! symlinks), with size limits. Replacing a file keeps the previous version under .history/, so it can be undone.
//! A file written after reading outside content stays marked, and reading it marks the conversation again; a file
//! Mimir didn't write (dropped into the folder by hand) counts as outside content too.

use anyhow::{Context, Result, bail};
use std::path::{Component, Path, PathBuf};

const MAX_FILE: usize = 1_000_000;
const MAX_CHAT: u64 = 50_000_000;
const HISTORY: &str = ".history";

pub fn root(db: &crate::db::Db, chat: i64) -> PathBuf {
    db.dir().join("files").join(format!("chat-{chat}"))
}

/// A file's name for its mark: the path as the file system sees it (no ./ or //), in lower case, since macOS and
/// Windows treat Notes/A.md and notes/a.md as one file (on Linux two such files share a mark, the stricter way).
fn key(rel: &str) -> String {
    Path::new(rel.trim())
        .components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// When a file was last changed, in milliseconds (0 if unknown).
fn mtime(p: &Path) -> i64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64)
}

/// The file a relative path names inside `root`, or why it can't be used.
fn resolve(root: &Path, rel: &str) -> Result<PathBuf> {
    let rel = rel.trim().trim_start_matches("./");
    let p = Path::new(rel);
    if rel.is_empty() || rel.len() > 200 || p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        bail!("use a path inside the workspace, like notes/trip.md");
    }
    if p.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.')) {
        bail!("names starting with a dot are kept for Mimir");
    }
    let full = root.join(p);
    // Nothing on the way may be a symlink (it could point anywhere).
    let mut at = root.to_path_buf();
    for c in p.components() {
        at.push(c);
        if at.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
            bail!("{rel} is a link; only plain files and folders");
        }
    }
    Ok(full)
}

fn size(dir: &Path) -> u64 {
    std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| match e.metadata() {
        Ok(m) if m.is_dir() => size(&e.path()),
        Ok(m) => m.len(),
        Err(_) => 0,
    }).sum()
}

/// Files (relative paths and sizes) under `dir` in the workspace, folders first, at most 200.
pub fn list(db: &crate::db::Db, chat: i64, dir: &str) -> Result<Vec<(String, u64)>> {
    let root = root(db, chat);
    let base = if dir.trim().is_empty() { root.clone() } else { resolve(&root, dir)? };
    let mut out = vec![];
    fn walk(root: &Path, d: &Path, out: &mut Vec<(String, u64)>) {
        let mut entries: Vec<_> = std::fs::read_dir(d).into_iter().flatten().flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || out.len() >= 200 {
                continue;
            }
            let Ok(m) = e.metadata() else { continue };
            // Always with "/", also on Windows: the same path a bot (or you) passes back to read it.
            let rel = e
                .path()
                .strip_prefix(root)
                .map(|p| p.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"))
                .unwrap_or(name);
            if m.is_dir() {
                walk(root, &e.path(), out);
            } else if m.is_file() {
                out.push((rel, m.len()));
            }
        }
    }
    walk(&root, &base, &mut out);
    Ok(out)
}

/// A file's text, and whether it's outside content (written after reading some, or not written by Mimir at all).
pub fn read(db: &crate::db::Db, chat: i64, rel: &str) -> Result<(String, bool)> {
    let path = resolve(&root(db, chat), rel)?;
    // Only a plain file, and its size before reading it (a huge one dropped in, or a pipe, would hang or fill memory).
    let meta = std::fs::metadata(&path).with_context(|| format!("no file {rel}"))?;
    if !meta.is_file() {
        bail!("{rel} isn't a file");
    }
    if meta.len() > MAX_FILE as u64 {
        bail!("{rel} is over 1 MB; too big to read here");
    }
    let text = String::from_utf8(std::fs::read(&path)?).map_err(|_| anyhow::anyhow!("{rel} isn't a text file"))?;
    // Outside content unless Mimir wrote it from a clean conversation and nothing has changed it since.
    let outside = match db.file_tainted(chat, &key(rel))? {
        Some((tainted, at)) => tainted || at != mtime(&path),
        None => true,
    };
    Ok((text, outside))
}

/// Write a file: create, replace (the old version kept under .history/) or append. Returns what was done.
pub fn write(db: &crate::db::Db, chat: i64, rel: &str, content: &str, mode: &str, tainted: bool) -> Result<String> {
    let root = root(db, chat);
    let path = resolve(&root, rel)?;
    let exists = path.is_file();
    let new_len = content.len() + if mode == "append" && exists { std::fs::metadata(&path)?.len() as usize } else { 0 };
    if new_len > MAX_FILE {
        bail!("a file can be up to 1 MB");
    }
    if size(&root) + content.len() as u64 > MAX_CHAT {
        bail!("this chat's workspace is full (50 MB); remove some files first");
    }
    std::fs::create_dir_all(path.parent().context("no folder")?)?;
    let done = match (mode, exists) {
        ("create", true) => bail!("{rel} already exists; replace it, append to it, or pick another name"),
        ("append", true) => {
            use std::io::Write;
            std::fs::OpenOptions::new().append(true).open(&path)?.write_all(content.as_bytes())?;
            "appended to"
        }
        ("replace", true) => {
            let keep = root.join(HISTORY).join(format!("{}.{}", rel.replace('/', "__"), crate::db::now()));
            std::fs::create_dir_all(keep.parent().context("no folder")?)?;
            std::fs::copy(&path, &keep)?;
            std::fs::write(&path, content)?;
            "replaced (the previous version is kept)"
        }
        ("create" | "replace" | "append", false) => {
            std::fs::write(&path, content)?;
            "created"
        }
        _ => bail!("mode is create, replace or append"),
    };
    // Once marked, a file stays marked: appending clean text doesn't make outside content clean.
    let was = db.file_tainted(chat, &key(rel))?.map(|(t, _)| t).unwrap_or(exists);
    db.file_mark(chat, &key(rel), tainted || (mode == "append" && was), mtime(&path))?;
    Ok(format!("{done} {rel}"))
}

pub fn delete(db: &crate::db::Db, chat: i64, rel: &str) -> Result<()> {
    let path = resolve(&root(db, chat), rel)?;
    if !path.is_file() {
        bail!("no file {rel}");
    }
    std::fs::remove_file(&path)?;
    db.file_forget(chat, &key(rel))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_inside_and_files_keep_their_mark() {
        let home = std::env::temp_dir().join(format!("mimir-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let db = crate::db::Db::open(&home.join("mimir.db")).unwrap();
        for bad in ["../x", "/etc/passwd", "a/../../x", ".history/x", "", "notes/.secret"] {
            assert!(write(&db, 3, bad, "x", "create", false).is_err(), "{bad}");
        }
        assert_eq!(write(&db, 3, "notes/todo.md", "- milk\n", "create", false).unwrap(), "created notes/todo.md");
        assert!(write(&db, 3, "notes/todo.md", "x", "create", false).is_err(), "create won't overwrite");
        write(&db, 3, "notes/todo.md", "- eggs\n", "append", false).unwrap();
        assert_eq!(read(&db, 3, "notes/todo.md").unwrap(), ("- milk\n- eggs\n".into(), false));
        assert!(write(&db, 3, "notes/todo.md", "- bread\n", "replace", false).unwrap().contains("previous version"));
        assert_eq!(list(&db, 3, "").unwrap(), vec![("notes/todo.md".to_string(), 8)], "history isn't listed");
        assert!(read(&db, 4, "notes/todo.md").is_err(), "another chat's workspace");
        // Written after outside content: marked, and appending clean text keeps the mark.
        write(&db, 3, "page.md", "from a page", "create", true).unwrap();
        write(&db, 3, "page.md", " more", "append", false).unwrap();
        assert!(read(&db, 3, "page.md").unwrap().1);
        // Dropped in by hand: outside content.
        std::fs::write(root(&db, 3).join("dropped.txt"), "hello").unwrap();
        assert!(read(&db, 3, "dropped.txt").unwrap().1);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc", root(&db, 3).join("etc")).unwrap();
            assert!(read(&db, 3, "etc/hosts").is_err(), "no following links out");
        }
        // Another name for the same file shares its mark; a change made outside Mimir counts as outside content.
        write(&db, 3, "clean.md", "mine", "create", false).unwrap();
        write(&db, 3, "./Clean.md", "a page's words", "replace", true).unwrap();
        assert!(read(&db, 3, "clean.md").unwrap().1, "the alias's mark is the file's mark");
        write(&db, 3, "mine.md", "mine", "create", false).unwrap();
        assert!(!read(&db, 3, "mine.md").unwrap().1);
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(root(&db, 3).join("mine.md"), "edited by hand").unwrap();
        assert!(read(&db, 3, "mine.md").unwrap().1, "changed outside Mimir");
        delete(&db, 3, "notes/todo.md").unwrap();
        assert!(read(&db, 3, "notes/todo.md").is_err());
        let _ = std::fs::remove_dir_all(&home);
    }
}
