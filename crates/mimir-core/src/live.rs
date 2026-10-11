//! Replies being written right now, so a UI can show them as they stream in. In-process only:
//! providers run in the host process (only tools run in the MCP subprocess).

use std::collections::HashMap;
use std::sync::Mutex;

static DRAFTS: Mutex<Option<HashMap<(i64, i64), String>>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut HashMap<(i64, i64), String>) -> R) -> R {
    f(DRAFTS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new))
}

/// Said at the end of a draft whose reply changed course, until the new answer's first words.
const TURNING: &str = "_Changing course…_";

/// Append streamed text for (bot, chat).
pub fn push(bot: i64, chat: i64, delta: &str) {
    with(|m| {
        let d = m.entry((bot, chat)).or_default();
        if d.ends_with(TURNING) {
            d.clear();
        }
        d.push_str(delta)
    });
}

/// A new assistant message started (e.g. after a tool call): show only the current one (or that it's changing course).
pub fn reset(bot: i64, chat: i64) {
    with(|m| {
        let d = m.entry((bot, chat)).or_default();
        *d = if d.ends_with(TURNING) { TURNING.to_string() } else { String::new() };
    });
}

pub fn get(bot: i64, chat: i64) -> Option<String> {
    with(|m| m.get(&(bot, chat)).cloned())
}

pub fn end(bot: i64, chat: i64) {
    with(|m| m.remove(&(bot, chat)));
    set_step(bot, chat, None);
    inbox(|m| m.remove(&(bot, chat)));
}

/// The reply changed course (a message sent while it was being written): what it had so far, saying so, until the
/// new answer starts.
pub fn turned(bot: i64, chat: i64) {
    with(|m| {
        let d = m.entry((bot, chat)).or_default();
        *d = format!("{}\n\n{TURNING}", d.trim_end()).trim_start().to_string();
    });
}

/// Messages sent while a reply is being written, for the run writing it (steering).
#[derive(Default)]
struct Inbox {
    open: bool,
    waiting: Vec<String>,
    /// Taken in by the run: logged with the message it answers.
    taken: Vec<String>,
    bell: std::sync::Arc<tokio::sync::Notify>,
}

static INBOX: Mutex<Option<HashMap<(i64, i64), Inbox>>> = Mutex::new(None);

fn inbox<R>(f: impl FnOnce(&mut HashMap<(i64, i64), Inbox>) -> R) -> R {
    f(INBOX.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new))
}

/// A reply to a person started: messages sent before it ends can steer it.
pub fn listen(bot: i64, chat: i64) {
    inbox(|m| m.insert((bot, chat), Inbox { open: true, ..Default::default() }));
}

/// Rung when a message arrives for the reply being written; None when nothing there takes one.
pub fn bell(bot: i64, chat: i64) -> Option<std::sync::Arc<tokio::sync::Notify>> {
    inbox(|m| m.get(&(bot, chat)).filter(|i| i.open).map(|i| i.bell.clone()))
}

/// A message sent while a reply is being written here: true if that reply takes it in, false if it should wait its
/// turn as a message of its own (nothing being written, or it's just finishing).
pub fn steer(bot: i64, chat: i64, text: &str) -> bool {
    inbox(|m| match m.get_mut(&(bot, chat)).filter(|i| i.open) {
        Some(i) => {
            i.waiting.push(text.to_string());
            i.bell.notify_one();
            true
        }
        None => false,
    })
}

/// The messages waiting for the run to take in (and so taken).
pub fn steers(bot: i64, chat: i64) -> Vec<String> {
    inbox(|m| {
        let Some(i) = m.get_mut(&(bot, chat)) else { return vec![] };
        let w = std::mem::take(&mut i.waiting);
        i.taken.extend(w.iter().cloned());
        w
    })
}

/// The run has its answer: any message that just arrived, to take in first; with none, the inbox closes (one sent
/// from now on is a message of its own).
pub fn last_call(bot: i64, chat: i64) -> Vec<String> {
    let w = steers(bot, chat);
    if w.is_empty() {
        inbox(|m| m.get_mut(&(bot, chat)).map(|i| i.open = false));
    }
    w
}

/// Every message that steered this reply, in order.
pub fn steered(bot: i64, chat: i64) -> Vec<String> {
    inbox(|m| m.get(&(bot, chat)).map(|i| i.taken.iter().chain(&i.waiting).cloned().collect()).unwrap_or_default())
}

/// What the bot is doing right now ("Searching memory"), while a reply is in progress.
static STEPS: Mutex<Option<HashMap<(i64, i64), String>>> = Mutex::new(None);

pub fn set_step(bot: i64, chat: i64, step: Option<String>) {
    let mut g = STEPS.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    match step {
        Some(s) => m.insert((bot, chat), s),
        None => m.remove(&(bot, chat)),
    };
}

pub fn step(bot: i64, chat: i64) -> Option<String> {
    STEPS.lock().unwrap_or_else(|e| e.into_inner()).as_ref()?.get(&(bot, chat)).cloned()
}

/// A tool name in words, for "what it's doing now". Claude's names carry an `mcp__mimir__` prefix.
pub fn step_label(tool: &str) -> String {
    let t = tool.strip_prefix("mcp__mimir__").unwrap_or(tool);
    match t {
        "memory_search" => "Searching memory".into(),
        "memory_store" => "Saving to memory".into(),
        "memory_forget" => "Forgetting a memory".into(),
        "routine_create" => "Scheduling".into(),
        "routine_list" => "Looking at routines".into(),
        "routine_update" => "Changing a routine".into(),
        "routine_delete" => "Deleting a routine".into(),
        "web_fetch" => "Reading a web page".into(),
        "meeting_search" | "meeting_read" => "Looking at your meetings".into(),
        "chat_search" => "Looking through earlier chats".into(),
        "calendar_list" => "Looking at your calendar".into(),
        "calendar_add" => "Making a calendar link".into(),
        "calendar_create" => "Adding to your calendar".into(),
        "email_search" => "Looking through your email".into(),
        "email_read" => "Reading an email".into(),
        "email_draft" => "Writing an email draft".into(),
        "email_send" => "Sending an email".into(),
        "file_list" | "file_read" => "Looking at the workspace".into(),
        "file_write" => "Writing a file".into(),
        "file_delete" => "Deleting a file".into(),
        "send_file" => "Sending a file".into(),
        "skill_save" => "Writing it down as a skill".into(),
        "skill_view" => "Reading a skill".into(),
        "plan_propose" => "Waiting for your OK on a plan".into(),
        "ask_bot" => "Asking another bot".into(),
        "ask_choice" => "Asking you a question".into(),
        "bot_create" => "Creating a bot".into(),
        "WebSearch" | "web_search" => "Searching the web".into(),
        _ if t.starts_with("computer_") => "Using the computer".into(),
        _ => match t.split_once("__") {
            Some(("browser", _)) => "Using the browser".into(),
            Some((server, _)) => format!("Using {server}"),
            None => format!("Using {}", t.replace('_', " ")),
        },
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn drafts() {
        super::push(7, 1, "Hel");
        super::push(7, 1, "lo");
        assert_eq!(super::get(7, 1).as_deref(), Some("Hello"));
        super::reset(7, 1);
        assert_eq!(super::get(7, 1).as_deref(), Some(""));
        super::end(7, 1);
        assert_eq!(super::get(7, 1), None);
    }

    #[test]
    fn steering() {
        use super::*;
        assert!(!steer(8, 1, "nobody's writing"), "no reply in progress: a message of its own");
        listen(8, 1);
        push(8, 1, "Here's the month");
        assert!(steer(8, 1, "only this week"));
        turned(8, 1);
        assert_eq!(get(8, 1).as_deref(), Some("Here's the month\n\n_Changing course…_"));
        reset(8, 1);
        assert_eq!(get(8, 1).as_deref(), Some("_Changing course…_"), "said until the new answer starts");
        push(8, 1, "This week");
        assert_eq!(get(8, 1).as_deref(), Some("This week"));
        assert_eq!(steers(8, 1), ["only this week"]);
        assert!(steers(8, 1).is_empty(), "taken once");
        assert!(steer(8, 1, "and in French"));
        assert_eq!(last_call(8, 1), ["and in French"], "one that arrived as it finished is still taken in");
        assert!(last_call(8, 1).is_empty());
        assert!(!steer(8, 1, "too late"), "closed: waits its turn");
        assert_eq!(steered(8, 1), ["only this week", "and in French"]);
        end(8, 1);
        assert!(steered(8, 1).is_empty() && bell(8, 1).is_none());
    }
}
