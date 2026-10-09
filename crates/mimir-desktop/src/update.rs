//! Updates. Tauri's updater reads latest.json from the newest published GitHub release; each download is checked
//! against the public key in tauri.conf.json. A newer version downloads in the background, then waits: restarting stops
//! the bots mid-reply, so the person chooses when (Settings → General, or the tray).
use serde_json::{Value, json};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Default)]
pub struct Updates(Mutex<State>);

#[derive(Default)]
struct State {
    checking: bool,
    /// Downloaded and verified, waiting for a restart.
    ready: Option<(Update, Vec<u8>)>,
    error: Option<String>,
    checked_at: i64,
}

const FIRST_CHECK: Duration = Duration::from_secs(60);
const EVERY: Duration = Duration::from_secs(6 * 3600);

fn auto(app: &AppHandle) -> bool {
    app.state::<mimir_core::host::Host>().db.setting("auto_update").ok().flatten().as_deref() != Some("off")
}

/// The version waiting for a restart, if any (for the tray).
pub fn ready_version(app: &AppHandle) -> Option<String> {
    app.state::<Updates>().0.lock().unwrap().ready.as_ref().map(|(u, _)| u.version.clone())
}

pub fn status(app: &AppHandle) -> Value {
    let s = app.state::<Updates>();
    let s = s.0.lock().unwrap();
    json!({
        "current": app.package_info().version.to_string(),
        "checking": s.checking,
        "ready": s.ready.as_ref().map(|(u, _)| json!({"version": u.version, "notes": u.body})),
        "error": s.error,
        "checked_at": s.checked_at,
        "auto": auto(app),
    })
}

/// Look for a newer version and download it. Nothing to do if one is already waiting or a check is running.
pub async fn check(app: &AppHandle) {
    let updates = app.state::<Updates>();
    {
        let mut s = updates.0.lock().unwrap();
        if s.checking || s.ready.is_some() {
            return;
        }
        s.checking = true;
    }
    let found = async {
        match app.updater()?.check().await? {
            Some(u) => {
                let bytes = u.download(|_, _| {}, || {}).await?;
                Ok::<_, tauri_plugin_updater::Error>(Some((u, bytes)))
            }
            None => Ok(None),
        }
    }
    .await;
    let mut s = updates.0.lock().unwrap();
    s.checking = false;
    s.checked_at = mimir_core::db::now();
    match found {
        Ok(ready) => {
            s.error = None;
            s.ready = ready;
        }
        Err(e) => s.error = Some(format!("{e}")),
    }
    drop(s);
    crate::refresh_tray(app);
}

/// Install the waiting version and start it. Windows' installer closes the app itself.
pub fn install(app: &AppHandle) -> Result<(), String> {
    if app.state::<crate::meeting::Meeting>().status().recording {
        return Err("Stop transcribing the meeting first: restarting would cut it off.".into());
    }
    let Some((u, bytes)) = app.state::<Updates>().0.lock().unwrap().ready.take() else { return Err("No update is ready.".into()) };
    u.install(bytes).map_err(|e| format!("Couldn't install the update: {e}"))?;
    app.restart()
}

/// A minute after launch, then every 6 hours, unless switched off. MIMIR_UPDATE_TEST=1 (the update end-to-end test):
/// check after 3 s and install what it finds straight away, as if Restart had been pressed.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    let test = std::env::var("MIMIR_UPDATE_TEST").is_ok();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(if test { Duration::from_secs(3) } else { FIRST_CHECK }).await;
        loop {
            if auto(&app) || test {
                check(&app).await;
                eprintln!("update check: {}", status(&app));
                if test
                    && ready_version(&app).is_some()
                    && let Err(e) = install(&app)
                {
                    eprintln!("update install: {e}");
                }
            }
            tokio::time::sleep(EVERY).await;
        }
    });
}
