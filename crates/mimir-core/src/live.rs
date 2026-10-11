//! Replies being written right now, so a UI can show them as they stream in. In-process only:
//! providers run in the host process (only tools run in the MCP subprocess).

use std::collections::HashMap;
use std::sync::Mutex;

static DRAFTS: Mutex<Option<HashMap<(i64, i64), String>>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut HashMap<(i64, i64), String>) -> R) -> R {
    f(DRAFTS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new))
}

/// Append streamed text for (bot, chat).
pub fn push(bot: i64, chat: i64, delta: &str) {
    with(|m| m.entry((bot, chat)).or_default().push_str(delta));
}

/// A new assistant message started (e.g. after a tool call): show only the current one.
pub fn reset(bot: i64, chat: i64) {
    with(|m| m.insert((bot, chat), String::new()));
}

pub fn get(bot: i64, chat: i64) -> Option<String> {
    with(|m| m.get(&(bot, chat)).cloned())
}

pub fn end(bot: i64, chat: i64) {
    with(|m| m.remove(&(bot, chat)));
    set_step(bot, chat, None);
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
}
