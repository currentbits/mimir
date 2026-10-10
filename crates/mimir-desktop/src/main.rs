#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Mimir desktop: runs your bots locally (or points at your self-hosted `mimir server`)
//! and serves the UI. Also doubles as the MCP server binary (`<app> mcp --bot B --chat N`)
//! that `claude -p` / `codex exec` spawn, since providers re-exec `current_exe()`.

mod meeting;
mod update;

use mimir_core::db::Db;
use mimir_core::host::{Host, LOCAL_CHAT, SecretStore};
use mimir_core::mcp;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_notification::NotificationExt;

/// Quick-ask shortcut: ⌥Space on macOS (⌥⇧Space if Alfred, ChatGPT… already own it). Elsewhere
/// Alt+Space is the window menu (Windows, GNOME, KDE), so Ctrl+Alt+Space there.
#[cfg(target_os = "macos")]
const QUICK_KEYS: [(&str, &str); 2] = [("alt+space", "⌥ Space"), ("alt+shift+space", "⌥ ⇧ Space")];
#[cfg(not(target_os = "macos"))]
const QUICK_KEYS: [(&str, &str); 2] = [("ctrl+alt+space", "Ctrl+Alt+Space"), ("ctrl+alt+k", "Ctrl+Alt+K")];
/// The shortcut that registered, for Settings.
struct QuickKey(std::sync::Mutex<Option<&'static str>>);

/// Keeps the computer from falling asleep when idle while Mimir runs, so routines fire on time. The screen
/// can still turn off, and closing a laptop's lid or choosing Sleep still sleeps it.
struct Awake(std::sync::Mutex<Option<Inhibitor>>);

#[cfg(not(windows))]
type Inhibitor = std::process::Child;
/// Windows: a thread holds the request; dropping the sender ends it.
#[cfg(windows)]
type Inhibitor = std::sync::mpsc::Sender<()>;

#[cfg(target_os = "macos")]
fn stay_awake() -> Result<Inhibitor, String> {
    // -w: ends by itself if Mimir exits.
    std::process::Command::new("caffeinate").args(["-i", "-w", &std::process::id().to_string()]).spawn().map_err(|e| format!("couldn't keep awake: {e}"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn stay_awake() -> Result<Inhibitor, String> {
    let pid = std::process::id().to_string();
    std::process::Command::new("systemd-inhibit")
        .args(["--what=idle", "--who=Mimir", "--why=Your bots are running", "tail", "--pid", &pid, "-f", "/dev/null"])
        .spawn()
        .map_err(|e| format!("keeping awake needs systemd-inhibit (systemd): {e}"))
}

#[cfg(windows)]
fn stay_awake() -> Result<Inhibitor, String> {
    unsafe extern "system" {
        fn SetThreadExecutionState(flags: u32) -> u32;
    }
    const ES_CONTINUOUS: u32 = 0x8000_0000;
    const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        // SAFETY: plain Win32 call; the request belongs to this thread until it's reset.
        unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
        let _ = rx.recv();
        unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
    });
    Ok(tx)
}

fn set_awake(app: &AppHandle, on: bool) -> Result<(), String> {
    let awake = app.state::<Awake>();
    let mut held = awake.0.lock().unwrap();
    match (on, held.take()) {
        (true, Some(i)) => *held = Some(i),
        (true, None) => *held = Some(stay_awake()?),
        #[cfg(not(windows))]
        (false, Some(mut child)) => {
            let _ = child.kill();
            let _ = child.wait();
        }
        (false, _) => {}
    }
    Ok(())
}

const KEYCHAIN: &str = "app.heymimir";

struct Keychain;

impl SecretStore for Keychain {
    fn get(&self, name: &str) -> Option<String> {
        let read = |service| keyring::Entry::new(service, name).ok()?.get_password().ok().filter(|s: &String| !s.is_empty());
        // ponytail: keys saved before the rename to Mimir stay under the old service; drop the fallback once none predate it
        read(KEYCHAIN).or_else(|| read("dev.botpoc.app"))
    }
    fn set(&self, name: &str, value: &str) -> anyhow::Result<()> {
        // Saving or clearing replaces any pre-rename copy, so the fallback in get never resurrects it.
        if let Ok(old) = keyring::Entry::new("dev.botpoc.app", name) {
            let _ = old.delete_credential();
        }
        let entry = keyring::Entry::new(KEYCHAIN, name)?;
        if value.is_empty() {
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(err) => Err(err.into()),
            }
        } else {
            Ok(entry.set_password(value)?)
        }
    }
}

/// Every UI command goes through here. `local_bots` is desktop-only: turn the in-app
/// runtime off while the UI is pointed at a server (two runtimes on one Telegram token conflict).
#[tauri::command]
async fn api(app: AppHandle, host: State<'_, Host>, cmd: String, args: Value) -> Result<Value, String> {
    // Smoke test of the real window + IPC (MIMIR_SELFTEST=1): the UI reports what it rendered, then we exit.
    let selftest = std::env::var("MIMIR_SELFTEST").is_ok();
    match cmd.as_str() {
        "selftest" => return Ok(Value::Bool(selftest)),
        "selftest_report" if selftest => {
            println!("{args}");
            // The tray menu as built for the approvals waiting now.
            let pending = app.state::<Host>().db.approvals_pending().unwrap_or_default();
            if let Ok(menu) = tray_menu(&app, &pending, "header") {
                for it in menu.items().unwrap_or_default() {
                    match it {
                        tauri::menu::MenuItemKind::Submenu(sub) => {
                            let inner: Vec<String> = sub.items().unwrap_or_default().iter().filter_map(|i| i.as_menuitem().map(|m| format!("{}{}", m.text().unwrap_or_default(), if m.is_enabled().unwrap_or(true) { "" } else { " (off)" }))).collect();
                            println!("tray: {} -> {}", sub.text().unwrap_or_default(), inner.join(" | "));
                        }
                        tauri::menu::MenuItemKind::MenuItem(m) => println!("tray: {}", m.text().unwrap_or_default()),
                        _ => {}
                    }
                }
            }
            mimir_core::transcribe::unload();
            std::process::exit(if args["errors"].as_array().is_some_and(|e| e.is_empty()) { 0 } else { 1 });
        }
        _ => {}
    }
    // Meeting transcription runs on this computer even when the app is connected to a server.
    if cmd == "meeting" {
        let m = app.state::<meeting::Meeting>();
        let (home, model) = (host.db.dir(), mimir_core::transcribe::model_name(&host.db));
        let action = args["action"].as_str().unwrap_or("status");
        if matches!(action, "start" | "stop") {
            // The tray's Start/Stop item follows.
            let app = app.clone();
            tauri::async_runtime::spawn(async move { refresh_tray(&app) });
        }
        return match action {
            "start" => m.start(host.db.clone(), model).map(|s| json!(s)),
            "stop" => match m.stop(&host.db, model).await {
                Some(st) => {
                    after_meeting(&app, &st);
                    Ok(json!(st))
                }
                None => Ok(json!(m.status())), // already stopped (the tray or a second click)
            },
            // Which microphone is "you" (default: the system's), and what happens when a meeting ends.
            // The calendar's private iCal link (in the keychain): link (checked first), unlink, or say if linked.
            "calendar" => {
                use mimir_core::calendar;
                match args["url"].as_str() {
                    Some("") => {
                        mimir_core::host::secret_set(calendar::SECRET, "").map_err(|e| format!("{e:#}"))?;
                        Ok(json!({"linked": false}))
                    }
                    Some(url) => {
                        let url = calendar::normalise(url).ok_or("paste the calendar's https:// (or webcal://) iCal link")?;
                        let count = calendar::check(&url).await.map_err(|e| format!("{e:#}"))?;
                        mimir_core::host::secret_set(calendar::SECRET, &url).map_err(|e| format!("{e:#}"))?;
                        Ok(json!({"linked": true, "this_week": count}))
                    }
                    None => Ok(json!({"linked": mimir_core::host::secret_get(calendar::SECRET).is_some()})),
                }
            }
            "mics" => Ok(json!({"mics": meeting::microphones(), "chosen": host.db.setting("meeting_mic").ok().flatten()})),
            "set" => {
                for key in ["meeting_mic", "meeting_notes"] {
                    if let Some(v) = args.get(key) {
                        host.db.set_setting(key, &v.as_str().map(str::to_string).unwrap_or_else(|| if v.is_null() { String::new() } else { v.to_string() })).map_err(|e| format!("{e:#}"))?;
                    }
                }
                Ok(json!({"meeting_mic": host.db.setting("meeting_mic").ok().flatten(), "meeting_notes": host.db.setting("meeting_notes").ok().flatten()}))
            }
            // Meetings live in this computer's Mimir (they're recorded here), even when the app shows a server.
            "list" => host.db.meetings().map(|v| json!(v)).map_err(|e| format!("{e:#}")),
            "search" => host.db.meeting_search(args["query"].as_str().unwrap_or(""), 50)
                .map(|v| json!(v.into_iter().map(|(id, title, started_at, ts, who, text)| json!({"id": id, "title": title, "started_at": started_at, "ts": ts, "who": who, "text": text})).collect::<Vec<_>>()))
                .map_err(|e| format!("{e:#}")),
            "get" | "rename" | "notes" | "speaker" | "summary" | "delete" => {
                let cmd = match action {
                    "get" => "meeting",
                    "rename" => "meeting_rename",
                    "notes" => "meeting_notes",
                    "speaker" => "meeting_speaker",
                    "summary" => "meeting_summary",
                    _ => "meeting_delete",
                };
                host.call(cmd, &args).await.map_err(|e| format!("{e:#}"))
            }
            // Offer to transcribe when another app starts using the microphone (on unless switched off).
            "offer" => {
                if let Some(on) = args["on"].as_bool() {
                    host.db.set_setting("meeting_offer", if on { "on" } else { "off" }).map_err(|e| format!("{e:#}"))?;
                }
                Ok(json!(host.db.setting("meeting_offer").ok().flatten().as_deref() != Some("off")))
            }
            "engine" => Ok(json!({"engine": mimir_core::transcribe::engine(), "whisper": mimir_core::transcribe::whisper_bin().is_some(),
                "model": model, "model_ready": mimir_core::transcribe::model_path(&home, &mimir_core::transcribe::model_name(&host.db)).is_file()})),
            _ => Ok(json!(m.status())),
        };
    }
    if cmd == "quick_shortcut" {
        return Ok(json!(*app.state::<QuickKey>().0.lock().unwrap()));
    }
    if cmd == "keep_awake" {
        if let Some(on) = args["enabled"].as_bool() {
            set_awake(&app, on)?;
            app.state::<Host>().db.set_setting("keep_awake", if on { "on" } else { "off" }).map_err(|e| format!("{e:#}"))?;
        }
        return Ok(json!(app.state::<Awake>().0.lock().unwrap().is_some()));
    }
    if cmd == "show_main" {
        if let Some(q) = app.get_webview_window("quick") {
            let _ = q.hide();
        }
        show(&app);
        return Ok(Value::Null);
    }
    if cmd == "open_at_login" {
        let auto = app.autolaunch();
        if let Some(on) = args["enabled"].as_bool() {
            let r = if on { auto.enable() } else { auto.disable() };
            r.map_err(|e| format!("{e}"))?;
        }
        return Ok(json!(auto.is_enabled().unwrap_or(false)));
    }
    // The webview's clipboard refuses writes after any wait (WebKit's user-activation rule): the app copies instead.
    if cmd == "copy_text" {
        let text = args["text"].as_str().unwrap_or("");
        arboard::Clipboard::new().and_then(|mut c| c.set_text(text)).map_err(|e| format!("couldn't copy: {e}"))?;
        return Ok(Value::Null);
    }
    // A file to keep (an export): straight into Downloads, as a browser would; the webview itself downloads nothing.
    if cmd == "save_download" {
        let name = std::path::Path::new(args["name"].as_str().unwrap_or("mimir.txt")).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "mimir.txt".into());
        let dir = dirs::download_dir().or_else(dirs::home_dir).ok_or("no Downloads folder")?;
        let (stem, ext) = name.rsplit_once('.').map(|(s, e)| (s.to_string(), format!(".{e}"))).unwrap_or((name.clone(), String::new()));
        let mut path = dir.join(&name);
        let mut n = 1;
        while path.exists() {
            path = dir.join(format!("{stem} ({n}){ext}"));
            n += 1;
        }
        std::fs::write(&path, args["text"].as_str().unwrap_or("")).map_err(|e| format!("couldn't save: {e}"))?;
        return Ok(json!(path.to_string_lossy()));
    }
    // Links in replies open in the user's browser, never inside the app window.
    if cmd == "open_url" {
        let url = args["url"].as_str().unwrap_or("");
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err("only web links can be opened".into());
        }
        #[cfg(target_os = "macos")]
        let opener = ("open", vec![url]);
        // Explorer splits its argument at commas (and quotes), so those are percent-encoded: the link stays the same
        // address and can't turn into a path to open.
        #[cfg(target_os = "windows")]
        let safe = url.replace(',', "%2C").replace('"', "%22");
        #[cfg(target_os = "windows")]
        let opener = ("explorer", vec![safe.as_str()]);
        #[cfg(all(unix, not(target_os = "macos")))]
        let opener = ("xdg-open", vec![url]);
        std::process::Command::new(opener.0).args(opener.1).spawn().map_err(|e| format!("couldn't open the link: {e}"))?;
        return Ok(Value::Null);
    }
    // The admin token of the server this app is connected to: in the keychain, like every other secret.
    if cmd == "remote_token" {
        if let Some(t) = args["set"].as_str() {
            mimir_core::host::secret_set("remote_server_token", t).map_err(|e| format!("{e:#}"))?;
        }
        return Ok(json!(mimir_core::host::secret_get("remote_server_token")));
    }
    // Updates (desktop only): what's waiting, look now, install and restart, check automatically or not.
    match cmd.as_str() {
        "update_status" => return Ok(update::status(&app)),
        "update_check" => {
            update::check(&app).await;
            return Ok(update::status(&app));
        }
        "update_install" => return update::install(&app).map(|_| Value::Null),
        "update_auto" => {
            let on = args["on"].as_bool().unwrap_or(true);
            host.db.set_setting("auto_update", if on { "on" } else { "off" }).map_err(|e| format!("{e:#}"))?;
            return Ok(update::status(&app));
        }
        _ => {}
    }
    if cmd == "local_bots" {
        let on = args["enabled"].as_bool().unwrap_or(true);
        host.db.set_setting("local_bots", if on { "on" } else { "off" }).map_err(|e| format!("{e:#}"))?;
        if on { host.restart_all() } else { host.stop_all() }
        return Ok(Value::Null);
    }
    host.call(&cmd, &args).await.map_err(|e| format!("{e:#}"))
}

/// The quick-ask panel: summoned from anywhere with the shortcut, hides when it loses focus.
fn toggle_quick(app: &AppHandle) {
    let Some(q) = app.get_webview_window("quick") else { return };
    if q.is_visible().unwrap_or(false) {
        let _ = q.hide();
    } else {
        let _ = q.center();
        let _ = q.show();
        let _ = q.set_focus();
    }
}

/// Bring the window up on a section ("inbox", "settings"…), like the macOS menu bar does.
fn show_at(app: &AppHandle, view: &str) {
    show(app);
    let _ = app.emit("menu", view.to_string());
}

/// How many approvals wait (beside the icon on macOS, in the tooltip everywhere), and the tray menu that
/// lists them. Plain "your rules say ask" requests can be allowed or denied right there; ones from a
/// conversation that read something untrusted, or that would type a saved login, open the full card.
fn update_tray(app: &AppHandle, pending: &[mimir_core::db::Approval]) {
    let Some(tray) = app.tray_by_id("main") else { return };
    let recording = app.state::<meeting::Meeting>().status().recording;
    let text = match pending.len() {
        0 => "No approvals waiting".to_string(),
        1 => "1 approval waiting".to_string(),
        n => format!("{n} approvals waiting"),
    };
    let tip = if pending.is_empty() { "Mimir".to_string() } else { format!("Mimir · {text}") };
    let _ = tray.set_tooltip(Some(if recording { format!("{tip} · transcribing") } else { tip }));
    #[cfg(target_os = "macos")]
    let _ = tray.set_title(if pending.is_empty() { None } else { Some(pending.len().to_string()) });
    match tray_menu(app, pending, &text) {
        Ok(menu) => drop(tray.set_menu(Some(menu))),
        Err(e) => eprintln!("tray menu: {e}"),
    }
}

/// At most this many approvals get their own submenu; the rest are one "more" line.
const TRAY_APPROVALS: usize = 5;

fn tray_menu(app: &AppHandle, pending: &[mimir_core::db::Approval], header: &str) -> tauri::Result<Menu<tauri::Wry>> {
    let host = app.state::<Host>();
    let item = |id: String, label: &str, on: bool| MenuItem::with_id(app, id, label, on, None::<&str>);
    let sep = || PredefinedMenuItem::separator(app);
    let menu = Menu::new(app)?;
    menu.append(&item("inbox".into(), header, !pending.is_empty())?)?;
    for a in pending.iter().take(TRAY_APPROVALS) {
        let bot = host.db.bot(a.bot_id).map(|b| b.name).unwrap_or_else(|_| "Your bot".into());
        let short: String = a.summary.chars().take(46).collect();
        let whole = short.len() == a.summary.len();
        let short = if whole { short } else { format!("{}…", short.trim_end()) };
        let sub = Submenu::new(app, format!("{bot}: {short}"), true)?;
        // Allow from the menu only when the menu shows all of it; otherwise it's decided in the app, where it's whole.
        if a.kind == "rule" && whole {
            sub.append(&item(format!("ap:{}:y", a.id), if a.parked { "Allow (runs it now)" } else { "Allow" }, true)?)?;
        } else {
            let why = match a.kind.as_str() {
                "rule" => "Too long to show here: review it in Mimir",
                "login" => "Uses a saved login",
                "send" => "Sends an email: review it in Mimir",
                "bot" => "Makes a new bot: review it in Mimir",
                _ => "After reading something untrusted",
            };
            sub.append(&item(format!("info:{}", a.id), why, false)?)?;
        }
        sub.append(&item(format!("ap:{}:n", a.id), "Deny", true)?)?;
        sub.append(&sep()?)?;
        sub.append(&item("inbox".into(), "Review in Mimir…", true)?)?;
        menu.append(&sub)?;
    }
    if pending.len() > TRAY_APPROVALS {
        menu.append(&item("inbox".into(), &format!("{} more in Mimir…", pending.len() - TRAY_APPROVALS), true)?)?;
    }
    let quick_key = *app.state::<QuickKey>().0.lock().unwrap();
    let quick = match quick_key { Some(k) => format!("Quick ask    {k}"), None => "Quick ask".into() };
    menu.append(&sep()?)?;
    let recording = app.state::<meeting::Meeting>().status().recording;
    menu.append(&item("meeting".into(), if recording { "● Stop transcribing" } else { "Transcribe a meeting" }, true)?)?;
    menu.append(&item("quick".into(), &quick, true)?)?;
    menu.append(&item("open".into(), "Open Mimir", true)?)?;
    menu.append(&sep()?)?;
    if let Some(v) = update::ready_version(app) {
        menu.append(&item("update".into(), &format!("Restart to update to {v}"), true)?)?;
    }
    menu.append(&item("quit".into(), "Quit Mimir (stops your bots)", true)?)?;
    Ok(menu)
}

/// A meeting ended: if the person chose a bot for notes (setting meeting_notes = bot id), it writes them in the
/// built-in chat, and a notification says they're ready.
fn after_meeting(app: &AppHandle, st: &meeting::Status) {
    let host = app.state::<Host>();
    let (Some(id), Some(bot)) = (st.meeting_id, host.db.setting("meeting_notes").ok().flatten().and_then(|b| b.parse::<i64>().ok()).filter(|b| *b > 0)) else { return };
    let Ok(prompt) = mimir_core::tools::meeting_notes_prompt(&host.db, id) else { return };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // The notes name speakers once they're told apart.
        app.state::<meeting::Meeting>().speakers_done().await;
        let host = app.state::<Host>();
        let (title, body) = match host.call("send_local", &json!({"bot_id": bot, "text": prompt})).await {
            Ok(reply) => {
                // Kept on the meeting too, so they're there when you open it (not only in the chat).
                let _ = host.db.meeting_summary(id, reply.as_str().unwrap_or(""));
                ("Your meeting notes are ready".to_string(), mimir_core::tools::truncate(&reply.as_str().unwrap_or("").replace("**", ""), 180))
            }
            Err(e) => ("Couldn't write the meeting notes".to_string(), format!("{e:#}")),
        };
        let _ = app.notification().builder().title(title).body(body).show();
    });
}

fn refresh_tray(app: &AppHandle) {
    update_tray(app, &app.state::<Host>().db.approvals_pending().unwrap_or_default());
}

/// Tray "Transcribe a meeting" / "Stop transcribing". Starting doesn't need the window; a problem (no whisper,
/// no permission) or a finished transcript opens Meetings.
fn tray_meeting(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let (m, host) = (app.state::<meeting::Meeting>(), app.state::<Host>());
        let model = mimir_core::transcribe::model_name(&host.db);
        if m.status().recording {
            if let Some(st) = m.stop(&host.db, model).await {
                after_meeting(&app, &st);
            }
            show_at(&app, "meetings");
        } else if m.start(host.db.clone(), model).is_err() {
            show_at(&app, "meetings");
        }
        refresh_tray(&app);
    });
}

/// A tray "Allow"/"Deny": the same decision as the card in the app (a parked routine step runs).
fn tray_decide(app: &AppHandle, id: &str) {
    let Some((id, d)) = mimir_core::runtime::parse_decision(id) else { return };
    // The tray offers Allow and Deny only.
    let approve = d != mimir_core::runtime::Decision::Deny;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let host = app.state::<Host>();
        if let Err(e) = host.call("decide", &json!({ "id": id, "approve": approve })).await {
            eprintln!("tray decision: {e:#}");
        }
        update_tray(&app, &host.db.approvals_pending().unwrap_or_default());
    });
}

fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Native notifications while the window isn't in front: new approvals, and quiet-check / routine
/// results in the built-in chat (which has no messaging app to deliver them).
async fn notifier(app: AppHandle) {
    let mut seen_approvals: HashSet<i64> = HashSet::new();
    let mut since = mimir_core::db::now();
    let mut shown = vec![-1];
    // A call in progress (another app on the microphone): since when, and whether we've offered to transcribe it.
    let (mut call_since, mut offered) = (None::<i64>, false);
    // Calendar meetings already offered (title, start), checked about once a minute.
    let (mut offered_events, mut tick, mut calendar_offer_at) = (HashSet::<(String, i64)>::new(), 0u32, 0i64);
    loop {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let host = app.state::<Host>();
        tick += 1;
        if tick % 20 == 1
            && !app.state::<meeting::Meeting>().status().recording
            && host.db.setting("meeting_offer").ok().flatten().as_deref() != Some("off")
            && mimir_core::host::secret_get(mimir_core::calendar::SECRET).is_some()
        {
            let t = mimir_core::db::now();
            // A meeting with others, starting in the coming minute (or just started).
            for ev in mimir_core::calendar::starting(t - 60, t + 60).await.into_iter().filter(|e| e.attendees.len() >= 2) {
                if offered_events.insert((ev.title.clone(), ev.start)) {
                    let place = if cfg!(target_os = "macos") { "menu bar" } else { "tray" };
                    let body = format!("{} at {}. To transcribe it: Mimir icon in the {place} → Transcribe a meeting.", ev.title, mimir_core::calendar::clock(ev.start));
                    if let Err(e) = app.notification().builder().title("Meeting starting").body(body).show() {
                        eprintln!("notification failed: {e}");
                    }
                    calendar_offer_at = t; // the call that follows needn't be offered again
                }
            }
        }
        if app.state::<meeting::Meeting>().status().recording {
            offered = true; // that call's being transcribed; don't offer it again when it stops
        } else if host.db.setting("meeting_offer").ok().flatten().as_deref() != Some("off") {
            match tokio::task::spawn_blocking(meeting::mic_in_use).await.ok().flatten() {
                Some(who) => {
                    let since = *call_since.get_or_insert(mimir_core::db::now());
                    // 20 s: dictation and voice notes are shorter than calls.
                    if !offered && mimir_core::db::now() - since >= 20 && mimir_core::db::now() - calendar_offer_at > 600 {
                        offered = true;
                        let who = if who.is_empty() { "An app".to_string() } else { who };
                        let place = if cfg!(target_os = "macos") { "menu bar" } else { "tray" };
                        if let Err(e) = app.notification().builder().title("In a call?").body(format!("{who} is using your microphone. To transcribe the call: Mimir icon in the {place} → Transcribe a meeting.")).show() {
                            eprintln!("notification failed: {e}");
                        }
                    }
                }
                None => (call_since, offered) = (None, false),
            }
        }
        let pending = host.db.approvals_pending().unwrap_or_default();
        let ids: Vec<i64> = pending.iter().map(|a| a.id).collect();
        if ids != shown {
            shown = ids;
            update_tray(&app, &pending);
        }
        let focused = app.get_webview_window("main").and_then(|w| w.is_focused().ok()).unwrap_or(false);
        let name = |id: i64| host.db.bot(id).map(|b| b.name).unwrap_or_else(|_| "Your bot".into());
        let notify = |title: String, body: String| {
            if !focused && let Err(e) = app.notification().builder().title(title).body(body).show() {
                eprintln!("notification failed: {e}");
            }
        };
        let quiet = mimir_core::runtime::quiet(&host.db);
        for a in host.db.approvals_pending().unwrap_or_default() {
            // A routine's ask can wait out quiet hours; it's announced when they end.
            if !(quiet && a.parked) && seen_approvals.insert(a.id) {
                let hint = if a.kind != "rule" { "Review it in Mimir." } else if cfg!(target_os = "macos") { "Allow or deny it from the Mimir icon in the menu bar." } else { "Allow or deny it from the Mimir icon in the tray." };
                notify(format!("{} needs your approval", name(a.bot_id)), format!("{}\n{hint}", a.summary));
            }
        }
        let msgs = host.db.messages(LOCAL_CHAT, 20).unwrap_or_default();
        for pair in msgs.windows(2) {
            let (ask, reply) = (&pair[0], &pair[1]);
            if reply.ts > since && reply.role == "assistant" && ask.content.starts_with("[Scheduled") {
                notify(name(reply.bot_id), mimir_core::tools::truncate(&reply.content, 180));
            }
        }
        // The daily digest arrives on its own (no scheduled prompt before it).
        for m in msgs.iter().filter(|m| m.ts > since && m.role == "assistant" && m.content.starts_with("☀️")) {
            notify(format!("{}: your digest", name(m.bot_id)), mimir_core::tools::truncate(&m.content.replace("**", ""), 180));
        }
        since = msgs.last().map_or(since, |m| m.ts.max(since));
    }
}

/// Apps started from Finder or a Linux desktop launcher get a bare PATH (/usr/bin:/bin:…), so
/// `claude`, `codex` and `npx` would look "not installed". Take the login shell's PATH, plus the usual
/// install locations. Windows apps inherit the user's PATH already.
#[cfg(windows)]
fn fix_path() {}

#[cfg(unix)]
fn fix_path() {
    use std::path::PathBuf;
    let shell = std::env::var("SHELL").unwrap_or_else(|_| if cfg!(target_os = "macos") { "/bin/zsh".into() } else { "/bin/sh".into() });
    let from_shell = std::process::Command::new(&shell)
        .args(["-ilc", "printf %s \"$PATH\""])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let home = mimir_core::user_home().unwrap_or_default();
    let extra = [home.join(".local/bin"), "/opt/homebrew/bin".into(), "/usr/local/bin".into(), home.join(".npm-global/bin")];
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut parts: Vec<PathBuf> = vec![];
    for p in std::env::split_paths(&from_shell).chain(std::env::split_paths(&current)).chain(extra) {
        if !p.as_os_str().is_empty() && !parts.contains(&p) {
            parts.push(p);
        }
    }
    if let Ok(joined) = std::env::join_paths(parts) {
        // SAFETY: called at startup before any other thread exists.
        unsafe { std::env::set_var("PATH", joined) };
    }
}

/// The macOS menu bar: makes the shortcuts discoverable, and gives the webview the standard Edit menu
/// (copy/paste). Windows and Linux keep the window chrome clean; the same shortcuts work there via the UI.
#[cfg(target_os = "macos")]
fn app_menu(app: &mut tauri::App) -> tauri::Result<()> {
    use tauri::menu::{MenuBuilder, SubmenuBuilder};
    let item = |id: &str, label: &str, keys: Option<&str>| MenuItem::with_id(app, id, label, true, keys);
    let mimir = SubmenuBuilder::new(app, "Mimir")
        .about(None)
        .separator()
        .item(&item("settings", "Settings…", Some("CmdOrCtrl+,"))?)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;
    let file = SubmenuBuilder::new(app, "File")
        .item(&item("new_bot", "New Bot", Some("CmdOrCtrl+N"))?)
        .item(&item("menu_quick", "Quick Ask", None)?)
        .build()?;
    let edit = SubmenuBuilder::new(app, "Edit").undo().redo().separator().cut().copy().paste().select_all().build()?;
    let view = SubmenuBuilder::new(app, "View")
        .item(&item("inbox", "Needs You", Some("CmdOrCtrl+0"))?)
        .separator()
        .item(&item("chat", "Chat", Some("CmdOrCtrl+1"))?)
        .item(&item("memory", "Memory", Some("CmdOrCtrl+2"))?)
        .item(&item("skills", "Skills", Some("CmdOrCtrl+3"))?)
        .item(&item("routines", "Routines", Some("CmdOrCtrl+4"))?)
        .item(&item("activity", "Activity", Some("CmdOrCtrl+5"))?)
        .separator()
        .item(&item("search", "Search Memory", Some("CmdOrCtrl+F"))?)
        .build()?;
    let window = SubmenuBuilder::new(app, "Window").minimize().maximize().separator().close_window().build()?;
    app.set_menu(MenuBuilder::new(app).items(&[&mimir, &file, &edit, &view, &window]).build()?)?;
    app.on_menu_event(|app, e| match e.id.as_ref() {
        "menu_quick" => toggle_quick(app),
        id @ ("settings" | "new_bot" | "inbox" | "chat" | "memory" | "skills" | "routines" | "activity" | "search") => show_at(app, id),
        _ => {}
    });
    Ok(())
}

fn main() {
    fix_path();
    // The PDF helper (a separate process for reading one PDF): nothing else, no data folder.
    if std::env::args().nth(1).as_deref() == Some("read-pdf") {
        mimir_core::pdf_helper_main();
    }
    mimir_core::use_pdf_helper();
    let home = mimir_core::data_dir().expect("data dir");
    let db = match Db::open(&home.join("mimir.db")) {
        Ok(db) => db,
        Err(e) => {
            // The tools process has no window: its error goes to the app that started it.
            if std::env::args().nth(1).as_deref() != Some("mcp") {
                rfd::MessageDialog::new()
                    .set_level(rfd::MessageLevel::Error)
                    .set_title("Mimir can't open its data")
                    .set_description(format!("{e:#}\n\nYour data is in {}", home.display()))
                    .show();
            }
            eprintln!("open database: {e:#}");
            std::process::exit(1);
        }
    };
    let rt = tokio::runtime::Runtime::new().expect("tokio");

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("mcp") {
        mimir_core::host::install_secrets(Arc::new(Keychain));
        if let Err(err) = mcp::ctx_from_args(db, &args).and_then(|ctx| rt.block_on(mcp::serve_stdio(ctx))) {
            eprintln!("mcp: {err:#}");
            std::process::exit(1);
        }
        return;
    }

    // One tokio runtime for Tauri and the bots.
    tauri::async_runtime::set(rt.handle().clone());
    let host = Host::new(db, home, Arc::new(Keychain), rt.handle().clone(), "desktop");
    if host.db.setting("local_bots").ok().flatten().as_deref() != Some("off") {
        host.restart_all();
    }
    let mut builder = tauri::Builder::default();
    // A second launch (on the usual data folder) shows the running app instead of starting the same bots, with the
    // same tokens and database, a second time. A custom MIMIR_HOME (a test, a second profile) may run alongside.
    if std::env::var_os("MIMIR_HOME").is_none() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }));
    }
    builder
        // The windows only ever show the app itself: an outside page can't be loaded into them (it would look like
        // part of Mimir). Links open in the browser through open_url instead.
        .plugin(
            tauri::plugin::Builder::<tauri::Wry>::new("stay-home")
                .on_navigation(|_, url| {
                    matches!(url.scheme(), "tauri" | "asset" | "ipc")
                        || matches!(url.host_str(), Some("tauri.localhost" | "localhost" | "127.0.0.1" | "ipc.localhost"))
                        || url.as_str() == "about:blank"
                })
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(update::Updates::default())
        // Reopen where you left it (size, position); the quick panel always appears centred.
        .plugin(tauri_plugin_window_state::Builder::default().with_denylist(&["quick"]).build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_quick(app);
                    }
                })
                .build(),
        )
        .manage(QuickKey(Default::default()))
        .manage(Awake(Default::default()))
        .manage(meeting::Meeting::default())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .manage(host)
        .invoke_handler(tauri::generate_handler![api])
        .setup(|app| {
            tauri::async_runtime::spawn(notifier(app.handle().clone()));
            update::start(app.handle());
            // The others' audio a meeting left if the app quit mid-call: nothing is recording yet, so it can go.
            let _ = std::fs::remove_dir_all(app.state::<Host>().db.dir().join("meeting-audio"));
            // Test of the real recorder (MIMIR_MEETING_TEST=<seconds>): record, stop, print what was saved, exit.
            if let Some(secs) = std::env::var("MIMIR_MEETING_TEST").ok().and_then(|s| s.parse::<u64>().ok()) {
                let app = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let (m, host) = (app.state::<meeting::Meeting>(), app.state::<Host>());
                    let model = mimir_core::transcribe::model_name(&host.db);
                    println!("mic in use before: {:?}", meeting::mic_in_use());
                    let started = m.start(host.db.clone(), model.clone());
                    println!("meeting start: {}", serde_json::to_string(&started.as_ref().map(|s| &s.sources)).unwrap_or_default());
                    // Before Stop, lines should already be there (chunks end at pauses, not every 30 s).
                    tokio::time::sleep(std::time::Duration::from_secs(secs * 4 / 5)).await;
                    println!("meeting before stop: {} lines", m.status().lines.len());
                    tokio::time::sleep(std::time::Duration::from_secs(secs - secs * 4 / 5)).await;
                    // MIMIR_MEETING_TEST_QUIT: quit mid-call instead of Stop (the meeting should still be finished and saved).
                    if std::env::var("MIMIR_MEETING_TEST_QUIT").is_ok() {
                        println!("meeting quitting mid-call");
                        app.exit(0);
                        return;
                    }
                    let st = m.stop(&host.db, model).await.unwrap_or_else(|| m.status());
                    m.speakers_done().await;
                    let saved = st.meeting_id.map(|id| host.db.meeting_lines(id).unwrap_or_default());
                    let title = host.db.meetings().ok().and_then(|v| v.into_iter().find(|m| Some(m["id"].as_i64().unwrap_or(0)) == st.meeting_id)).map(|m| m["title"].clone());
                    println!("meeting result: {}", json!({"status": st, "saved": saved, "title": title}));
                    mimir_core::transcribe::unload();
                    std::process::exit(if started.is_ok() && st.error.is_none() { 0 } else { 1 });
                });
            }
            if app.state::<Host>().db.setting("keep_awake").ok().flatten().as_deref() == Some("on")
                && let Err(e) = set_awake(app.handle(), true)
            {
                eprintln!("{e}");
            }
            // First free quick-ask shortcut wins; none free just means no shortcut (the app still works).
            let registered = QUICK_KEYS.iter().find(|(keys, _)| app.global_shortcut().register(*keys).is_ok()).map(|(_, label)| *label);
            if registered.is_none() {
                eprintln!("quick-ask shortcut unavailable: {} are taken", QUICK_KEYS.map(|(_, l)| l).join(" and "));
            }
            *app.state::<QuickKey>().0.lock().unwrap() = registered;
            // Menu-bar / tray icon: the app keeps running (and your bots with it) when the window is closed.
            // It shows how many approvals are waiting (a count beside the icon on macOS; tooltip and menu everywhere).
            TrayIconBuilder::with_id("main")
                // macOS: a monochrome template image it tints for light/dark menu bars. Windows and
                // Linux taskbars are often dark and don't tint, so they get the full-colour icon.
                .icon(tauri::image::Image::from_bytes(if cfg!(target_os = "macos") { include_bytes!("../icons/tray.png") } else { include_bytes!("../icons/32x32.png") })?)
                .icon_as_template(cfg!(target_os = "macos"))
                .tooltip("Mimir")
                .menu(&tray_menu(app.handle(), &[], "No approvals waiting")?)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "open" => show(app),
                    "inbox" => show_at(app, "inbox"),
                    "quick" => toggle_quick(app),
                    "meeting" => tray_meeting(app),
                    "quit" => app.exit(0),
                    "update" => {
                        if let Err(e) = update::install(app) {
                            let _ = app.notification().builder().title("Mimir").body(e).show();
                        }
                    }
                    id => tray_decide(app, id),
                })
                .build(app)?;
            #[cfg(target_os = "macos")]
            app_menu(app)?;
            Ok(())
        })
        .on_window_event(|w, e| match e {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = w.hide();
            }
            // The quick panel behaves like Spotlight: click elsewhere and it goes away.
            WindowEvent::Focused(false) if w.label() == "quick" => {
                let _ = w.hide();
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("error while building Mimir")
        .run(|app, e| {
            // Quitting mid-call: the meeting is finished first (the last chunk transcribed and saved), for up to 20 s,
            // then the app quits for real (the second request goes through).
            static FINISHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if let tauri::RunEvent::ExitRequested { api, .. } = &e
                && app.state::<meeting::Meeting>().status().recording
                && !FINISHING.swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                api.prevent_exit();
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let (m, host) = (app.state::<meeting::Meeting>(), app.state::<Host>());
                    let model = mimir_core::transcribe::model_name(&host.db);
                    let _ = tokio::time::timeout(Duration::from_secs(20), m.stop(&host.db, model)).await;
                    app.exit(0);
                });
            }
            // The speech model off the GPU before the process ends (Metal aborts on exit otherwise).
            if let tauri::RunEvent::Exit = e {
                mimir_core::transcribe::unload();
            }
            // Clicking the Dock icon brings the hidden window back.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = e {
                show(app);
            }
            let _ = (app, e);
        });
}
