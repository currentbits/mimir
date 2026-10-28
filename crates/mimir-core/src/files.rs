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

/// The machine the workspaces are on, for a bot (or the app) to tell the person: its name, and in Docker that the path
/// is the container's (on the host, it's wherever the data volume is mounted).
pub fn machine() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        let name = std::process::Command::new("hostname").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|n| !n.is_empty());
        let name = name.map(|n| format!("the computer {n}")).unwrap_or_else(|| "this computer".into());
        if Path::new("/.dockerenv").exists() {
            format!("{name}, a Docker container (on its host, the folder is inside the volume mounted for Mimir's data)")
        } else {
            name
        }
    })
}

/// Where a workspace really is: "<absolute path> on <machine>".
pub fn whereabouts(root: &Path) -> String {
    format!("{} on {}", root.display(), machine())
}

/// Open a folder in Finder, Explorer or the file manager (the desktop app only).
pub fn reveal(dir: &Path) -> Result<()> {
    let opener = if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer" } else { "xdg-open" };
    std::process::Command::new(opener).arg(dir).spawn().with_context(|| format!("couldn't open {}", dir.display()))?;
    Ok(())
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

/// A workspace folder as one zip (its files, at most 200, under the folder's name), named after it; None if `rel`
/// isn't a folder.
pub fn zip_folder(db: &crate::db::Db, chat: i64, rel: &str) -> Result<Option<(String, Vec<u8>)>> {
    let root = root(db, chat);
    if !resolve(&root, rel)?.is_dir() {
        return Ok(None);
    }
    let files = list(db, chat, rel)?;
    if files.is_empty() {
        bail!("{rel} is an empty folder");
    }
    let base = rel.trim().trim_start_matches("./").trim_end_matches('/');
    let name = base.rsplit('/').next().unwrap_or(base);
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
    for (path, _) in &files {
        let inner = path.strip_prefix(base).unwrap_or(path).trim_start_matches('/');
        z.start_file(format!("{name}/{inner}"), zip::write::SimpleFileOptions::default())?;
        std::io::Write::write_all(&mut z, &std::fs::read(root.join(path))?)?;
    }
    Ok(Some((format!("{name}.zip"), z.finish()?.into_inner())))
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

/// A file a bot sends to its chat: saved under <home>/media (pruned with the photos after 60 days) as
/// `sent-<chat>-<random>-<name>`, and named at the end of the reply by a `[[file:…]]` line. The chat apps turn the
/// line into an upload, the app shows it in place. The random part means a name can't be guessed from another chat.
pub const SENT_MARK: &str = "[[file:";

/// Save a file to send with this run's reply.
pub fn send(db: &crate::db::Db, chat: i64, run: i64, name: &str, bytes: &[u8]) -> Result<String> {
    let name: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).take(80).collect();
    let saved = format!("sent-{chat}-{}-{}", crate::random_hex(8)?, name.trim_start_matches('.'));
    let dir = db.dir().join("media");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(&saved), bytes)?;
    let key = format!("outbox:{run}");
    let mut all: Vec<String> = db.setting(&key)?.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
    if all.len() >= 5 {
        bail!("at most 5 files with one reply");
    }
    all.push(saved);
    db.set_setting(&key, &serde_json::to_string(&all)?)?;
    Ok(name)
}

/// The `[[file:…]]` lines for the files this run sent.
pub fn sent_lines(db: &crate::db::Db, run: i64) -> String {
    let all: Vec<String> = db.setting(&format!("outbox:{run}")).ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
    all.iter().map(|n| format!("\n{SENT_MARK}{n}]]")).collect()
}

/// A reply's text without its `[[file:…]]` lines, and the files they name.
pub fn split_sent(text: &str) -> (String, Vec<String>) {
    let mut files = vec![];
    let kept: Vec<&str> = text
        .lines()
        .filter(|l| match l.trim().strip_prefix(SENT_MARK).and_then(|r| r.strip_suffix("]]")) {
            Some(n) => {
                files.push(n.to_string());
                false
            }
            None => true,
        })
        .collect();
    (kept.join("\n").trim_end().to_string(), files)
}

/// A sent file on disk, if `name` is one this chat sent (and nothing else: no paths, no other chat's files), with the
/// name it had before it was saved.
pub fn sent_path(home: &Path, chat: i64, name: &str) -> Option<(PathBuf, String)> {
    let rest = name.strip_prefix(&format!("sent-{chat}-"))?;
    let (_, shown) = rest.split_once('-')?;
    if name.contains(['/', '\\']) || name.contains("..") || shown.is_empty() {
        return None;
    }
    let p = home.join("media").join(name);
    p.is_file().then(|| (p, shown.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sent_files_go_with_the_reply_of_their_chat_only() {
        let home = std::env::temp_dir().join(format!("mimir-sent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let db = crate::db::Db::open(&home.join("mimir.db")).unwrap();
        assert_eq!(send(&db, 3, 7, "my report (v2).md", b"hi").unwrap(), "my_report__v2_.md");
        let lines = sent_lines(&db, 7);
        let (text, files) = split_sent(&format!("Here it is.{lines}"));
        assert_eq!(text, "Here it is.");
        assert_eq!(files.len(), 1);
        assert_eq!(sent_path(&home, 3, &files[0]).unwrap().1, "my_report__v2_.md");
        assert!(sent_path(&home, 4, &files[0]).is_none(), "another chat's file");
        assert!(sent_path(&home, 3, "sent-3-x-../../mimir.db").is_none());
        assert_eq!(sent_lines(&db, 8), "", "another run");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_folder_goes_as_one_zip() {
        let home = std::env::temp_dir().join(format!("mimir-zip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let db = crate::db::Db::open(&home.join("mimir.db")).unwrap();
        for f in ["skills/seo/SKILL.md", "skills/seo/refs/a.md", "skills/seo/refs/b.md", "other.md"] {
            write(&db, 3, f, f, "create", false).unwrap();
        }
        assert!(zip_folder(&db, 3, "other.md").unwrap().is_none(), "a file isn't a folder");
        let (name, bytes) = zip_folder(&db, 3, "skills/seo/").unwrap().unwrap();
        assert_eq!(name, "seo.zip");
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut names: Vec<String> = z.file_names().map(String::from).collect();
        names.sort();
        assert_eq!(names, ["seo/SKILL.md", "seo/refs/a.md", "seo/refs/b.md"]);
        let mut text = String::new();
        std::io::Read::read_to_string(&mut z.by_name("seo/refs/b.md").unwrap(), &mut text).unwrap();
        assert_eq!(text, "skills/seo/refs/b.md");
        assert!(zip_folder(&db, 3, "../x").is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

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
