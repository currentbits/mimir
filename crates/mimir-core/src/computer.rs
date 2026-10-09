//! The bots' own computer, on a self-hosted Linux server (the Docker image has what it needs): a virtual screen
//! (Xvfb) with a window manager and a visible Chromium. Bots see it in screenshots and use it with the mouse and
//! keyboard; the browser tools drive the same Chromium; you can watch and take over in the web app.
//! It runs on its own: with docker compose it's a separate service (`mimir computer`, its own user, none of Mimir's
//! data) that Mimir drives over a small control channel and Chromium's DevTools port; a plain `docker run` keeps it in
//! Mimir's container. Never on your own machine.
// ponytail: one computer per server, shared by its bots (like Grok Bot's one VM per person); per-bot screens later

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

pub const DISPLAY: &str = ":99";
pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 800;
/// Chromium's DevTools port, so the browser tools (Playwright) drive the Chromium you see on the screen.
pub const CDP_PORT: u16 = 9222;
/// The computer service passes this port on to Chromium's DevTools (which only listen inside its own container).
pub const CDP_SHARED_PORT: u16 = 9223;
/// A tool result that starts with `[[image:<path>]]` hands the model a picture (a screenshot) along with its text.
pub const IMAGE_MARK: &str = "[[image:";

/// The screen, window manager and browser processes this server started.
static PROCS: Mutex<Vec<Child>> = Mutex::new(Vec::new());

fn which(bin: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
}

fn chromium() -> Option<&'static str> {
    ["chromium", "chromium-browser", "google-chrome"].into_iter().find(|b| which(b))
}

/// Can this machine run the computer? (A computer service, or Linux with Xvfb, xdotool, scrot and Chromium.)
pub fn available() -> bool {
    remote().is_some() || (cfg!(target_os = "linux") && ["Xvfb", "xdotool", "scrot"].iter().all(|b| which(b)) && chromium().is_some())
}

/// The computer as a separate service (`MIMIR_COMPUTER=computer:7300`, docker compose); unset, it runs here.
fn remote() -> Option<String> {
    std::env::var("MIMIR_COMPUTER").ok().filter(|v| !v.trim().is_empty())
}

/// The secret both sides share for the control channel.
fn token() -> Result<String> {
    std::env::var("MIMIR_COMPUTER_TOKEN").ok().filter(|t| t.len() >= 24).context("set MIMIR_COMPUTER_TOKEN (24+ characters, the same for Mimir and the computer)")
}

fn resolve(addr: &str) -> Result<std::net::SocketAddr> {
    use std::net::ToSocketAddrs;
    addr.to_socket_addrs()?.next().with_context(|| format!("can't find the computer at {addr}"))
}

/// One request to the computer service: a line of JSON there, a line back.
fn ask(addr: &str, mut req: serde_json::Value) -> Result<serde_json::Value> {
    use std::io::{BufRead, Read, Write};
    req["token"] = token()?.into();
    let mut sock = std::net::TcpStream::connect_timeout(&resolve(addr)?, Duration::from_secs(3)).with_context(|| format!("can't reach the computer at {addr}"))?;
    // Typing 2000 characters takes ~30 s.
    sock.set_read_timeout(Some(Duration::from_secs(60)))?;
    writeln!(sock, "{req}")?;
    let mut line = String::new();
    std::io::BufReader::new(sock.take(32 << 20)).read_line(&mut line)?;
    let v: serde_json::Value = serde_json::from_str(&line).context("the computer didn't answer")?;
    if let Some(e) = v["error"].as_str() {
        bail!("{e}");
    }
    Ok(v)
}

/// Chromium's DevTools, for Playwright and the page lookups. The computer service is asked by IP: Chromium refuses
/// a Host header that isn't an IP or localhost.
pub fn cdp_endpoint() -> Result<String> {
    Ok(match remote() {
        Some(a) => format!("http://{}", std::net::SocketAddr::new(resolve(&a)?.ip(), CDP_SHARED_PORT)),
        None => format!("http://127.0.0.1:{CDP_PORT}"),
    })
}

/// Switched on in Settings (setting `computer` = on).
pub fn enabled(db: &crate::db::Db) -> bool {
    db.setting("computer").ok().flatten().as_deref() == Some("on")
}

/// Bots get the computer tools: switched on, and this machine can run it.
pub fn usable(db: &crate::db::Db) -> bool {
    enabled(db) && available()
}

fn xdotool(args: &[&str]) -> Result<String> {
    let out = Command::new("xdotool").args(args).env("DISPLAY", DISPLAY).stdin(Stdio::null()).output().context("xdotool isn't installed")?;
    if !out.status.success() {
        bail!("the computer didn't respond: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Is the screen up?
pub fn running() -> bool {
    if let Some(a) = remote() {
        return ask(&a, serde_json::json!({"op": "status"})).is_ok_and(|v| v["running"] == true);
    }
    available() && xdotool(&["getdisplaygeometry"]).is_ok()
}

fn browser_up() -> bool {
    std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], CDP_PORT).into(), Duration::from_millis(300)).is_ok()
}

/// The browser on the bots' computer: none of Mimir's environment (API keys, tokens) and a home of its own, so the
/// AI CLIs' logins under Mimir's home are out of its reach (the Docker image also blocks file:// pages by policy).
fn browser(home: &Path) -> Result<Command> {
    let own = home.join("computer").join("home");
    std::fs::create_dir_all(&own)?;
    let mut c = Command::new(chromium().context("Chromium isn't installed")?);
    c.env_clear().env("HOME", &own).env("PATH", std::env::var_os("PATH").unwrap_or_default()).env("LANG", std::env::var_os("LANG").unwrap_or("C.UTF-8".into()));
    Ok(c)
}

fn spawn(cmd: &mut Command) -> Result<()> {
    let child = cmd.env("DISPLAY", DISPLAY).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    PROCS.lock().unwrap_or_else(|e| e.into_inner()).push(child);
    Ok(())
}

/// Start the screen, window manager and browser if they aren't running (again after a crash). Idempotent.
pub fn start(home: &Path) -> Result<()> {
    if let Some(a) = remote() {
        return ask(&a, serde_json::json!({"op": "start"})).map(drop);
    }
    if !available() {
        bail!("the computer needs a Linux server with Xvfb, xdotool, scrot and Chromium (the Mimir Docker image has them)");
    }
    if !running() {
        spawn(Command::new("Xvfb").args([DISPLAY, "-screen", "0", &format!("{WIDTH}x{HEIGHT}x24"), "-nolisten", "tcp"])).context("couldn't start the screen (Xvfb)")?;
        for _ in 0..50 {
            if running() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if !running() {
            bail!("the screen (Xvfb) didn't come up");
        }
        if which("openbox") {
            spawn(&mut Command::new("openbox"))?;
        }
    }
    if !browser_up() {
        let profile = home.join("computer").join("chromium");
        std::fs::create_dir_all(&profile)?;
        // --no-sandbox: Chromium's own sandbox needs user namespaces, which containers usually don't allow; the
        // container is the sandbox here.
        spawn(browser(home)?.args([
            "--no-sandbox",
            // Hides the "unsupported command-line flag" bar --no-sandbox brings, which shifts every page down.
            "--test-type",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-dev-shm-usage",
            &format!("--remote-debugging-port={CDP_PORT}"),
            "--remote-debugging-address=127.0.0.1",
            &format!("--user-data-dir={}", profile.display()),
            "--window-position=0,0",
            &format!("--window-size={WIDTH},{HEIGHT}"),
            "--start-maximized",
            "about:blank",
        ]))
        .context("couldn't start Chromium")?;
        for _ in 0..80 {
            if browser_up() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    Ok(())
}

/// Stop everything this server started.
pub fn stop() {
    if let Some(a) = remote() {
        let _ = ask(&a, serde_json::json!({"op": "stop"}));
        return;
    }
    for mut c in PROCS.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// The screen as a JPEG (WIDTH×HEIGHT, so its pixels are the click coordinates).
pub fn screenshot_jpeg() -> Result<Vec<u8>> {
    use base64::Engine;
    if let Some(a) = remote() {
        let v = ask(&a, serde_json::json!({"op": "screenshot"}))?;
        return Ok(base64::engine::general_purpose::STANDARD.decode(v["image"].as_str().unwrap_or(""))?);
    }
    if !running() {
        bail!("the computer isn't running (Settings → Computer)");
    }
    let png = std::env::temp_dir().join(format!("mimir-screen-{}.png", crate::random_hex(6)?));
    let out = Command::new("scrot").args(["-o", "-z"]).arg(&png).env("DISPLAY", DISPLAY).stdin(Stdio::null()).output().context("scrot isn't installed")?;
    let img = image::open(&png);
    let _ = std::fs::remove_file(&png);
    if !out.status.success() {
        bail!("couldn't take a screenshot: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let mut jpg = std::io::Cursor::new(vec![]);
    image::DynamicImage::ImageRgb8(img.context("couldn't read the screenshot")?.to_rgb8()).write_to(&mut jpg, image::ImageFormat::Jpeg)?;
    Ok(jpg.into_inner())
}

/// A screenshot saved for a tool result: `[[image:<path>]]` + `text`. The image travels to the model, then goes.
pub fn with_screenshot(home: &Path, text: &str) -> Result<String> {
    // Give the page a moment to react to the last action.
    std::thread::sleep(Duration::from_millis(600));
    let dir = home.join("media");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("screen-{}-{}.jpg", crate::db::now(), crate::random_hex(4)?));
    std::fs::write(&path, screenshot_jpeg()?)?;
    Ok(format!("{IMAGE_MARK}{}]]\n{text}", path.display()))
}

/// Split a tool result into (image path, text). Only paths inside Mimir's media folder count.
pub fn split_image(result: &str, home: &Path) -> (Option<PathBuf>, String) {
    if let Some(rest) = result.strip_prefix(IMAGE_MARK)
        && let Some((path, text)) = rest.split_once("]]")
    {
        let p = PathBuf::from(path);
        if p.starts_with(home.join("media")) && !path.contains("..") {
            return (Some(p), text.trim_start_matches('\n').to_string());
        }
    }
    (None, result.to_string())
}

/// Only the computer's own tools hand over screenshots: any other tool's result (a tool server's, which anyone could
/// write) that looks like an image marker stays plain text.
pub fn tool_image(tool: &str, result: &str, home: &Path) -> (Option<PathBuf>, String) {
    if tool.starts_with("computer_") { split_image(result, home) } else { (None, result.to_string()) }
}

/// Read the image a result points at, as base64 JPEG, and delete the file.
pub fn take_image(path: &Path) -> Option<String> {
    use base64::Engine;
    let bytes = std::fs::read(path).ok()?;
    let _ = std::fs::remove_file(path);
    Some(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn coord(v: i64, max: u32) -> Result<String> {
    if v < 0 || v >= max as i64 {
        bail!("that's off the screen (it's {WIDTH}×{HEIGHT})");
    }
    Ok(v.to_string())
}

pub fn click(x: i64, y: i64, button: &str, double: bool) -> Result<()> {
    let b = match button {
        "right" => "3",
        "middle" => "2",
        _ => "1",
    };
    let (sx, sy) = (coord(x, WIDTH)?, coord(y, HEIGHT)?);
    if let Some(a) = remote() {
        return ask(&a, serde_json::json!({"op": "click", "x": x, "y": y, "button": button, "double": double})).map(drop);
    }
    let mut args = vec!["mousemove", "--sync", sx.as_str(), sy.as_str(), "click"];
    if double {
        args.extend(["--repeat", "2"]);
    }
    args.push(b);
    xdotool(&args).map(drop)
}

pub fn type_text(text: &str) -> Result<()> {
    if text.chars().count() > 2000 {
        bail!("that's a lot to type at once; send at most 2000 characters per call");
    }
    if let Some(a) = remote() {
        return ask(&a, serde_json::json!({"op": "type", "text": text})).map(drop);
    }
    xdotool(&["type", "--delay", "15", "--", text]).map(drop)
}

/// Key presses like "Return", "Tab", "ctrl+l", "ctrl+shift+t"; several separated by spaces.
pub fn key(keys: &str) -> Result<()> {
    let keys: Vec<&str> = keys.split_whitespace().collect();
    if keys.is_empty() || keys.iter().any(|k| !k.chars().all(|c| c.is_ascii_alphanumeric() || "+_-".contains(c))) {
        bail!("keys look like Return, Tab, ctrl+l or ctrl+shift+t");
    }
    if let Some(a) = remote() {
        return ask(&a, serde_json::json!({"op": "key", "keys": keys.join(" ")})).map(drop);
    }
    let mut args = vec!["key", "--"];
    args.extend(keys);
    xdotool(&args).map(drop)
}

pub fn scroll(x: i64, y: i64, down: bool, amount: i64) -> Result<()> {
    let (sx, sy) = (coord(x, WIDTH)?, coord(y, HEIGHT)?);
    let n = amount.clamp(1, 20).to_string();
    if let Some(a) = remote() {
        return ask(&a, serde_json::json!({"op": "scroll", "x": x, "y": y, "down": down, "amount": amount})).map(drop);
    }
    xdotool(&["mousemove", "--sync", &sx, &sy, "click", "--repeat", &n, if down { "5" } else { "4" }]).map(drop)
}

/// Open a web address in the computer's Chromium (a new tab of the one on screen).
pub fn open_url(home: &Path, url: &str) -> Result<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        bail!("open takes a web address starting with https://");
    }
    if let Some(a) = remote() {
        return ask(&a, serde_json::json!({"op": "open", "url": url})).map(drop);
    }
    start(home)?;
    let profile = home.join("computer").join("chromium");
    browser(home)?
        .args(["--no-sandbox", &format!("--user-data-dir={}", profile.display()), url])
        .env("DISPLAY", DISPLAY)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("couldn't open Chromium")?;
    Ok(())
}

// ---------- the computer as a service of its own ----------

/// `mimir computer --listen 0.0.0.0:7300`: run the screen and browser here and take Mimir's requests (screenshots,
/// clicks, typing, keys, opening pages) over a control channel that needs MIMIR_COMPUTER_TOKEN. Chromium's DevTools
/// are passed on at CDP_SHARED_PORT on the same address. Meant for a container with none of Mimir's data in it.
pub fn serve(listen: &str) -> Result<()> {
    if remote().is_some() {
        bail!("MIMIR_COMPUTER is set: that's for Mimir, pointing at this service");
    }
    let token = token()?;
    if !available() {
        bail!("the computer needs Xvfb, xdotool, scrot and Chromium (the Mimir Docker image has them)");
    }
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME isn't set")?);
    let listen: std::net::SocketAddr = listen.parse().context("--listen takes an address like 0.0.0.0:7300")?;
    let cdp = std::net::TcpListener::bind((listen.ip(), CDP_SHARED_PORT))?;
    std::thread::spawn(move || {
        for c in cdp.incoming().flatten() {
            std::thread::spawn(move || pass_on(c));
        }
    });
    let control = std::net::TcpListener::bind(listen)?;
    eprintln!("the bots' computer: control on {listen}, DevTools on {}:{CDP_SHARED_PORT}", listen.ip());
    for c in control.incoming().flatten() {
        let (home, token) = (home.clone(), token.clone());
        std::thread::spawn(move || {
            use std::io::{BufRead, Read, Write};
            let _ = c.set_read_timeout(Some(Duration::from_secs(10)));
            let mut line = String::new();
            let Ok(r) = c.try_clone() else { return };
            if std::io::BufReader::new(r.take(64 << 10)).read_line(&mut line).is_err() {
                return;
            }
            let req: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
            let given = req["token"].as_str().unwrap_or("");
            // Constant time: no hints about the token from how long a refusal takes.
            let same = given.len() == token.len() && given.bytes().zip(token.bytes()).fold(0, |d, (a, b)| d | (a ^ b)) == 0;
            let out = if same { act(&home, &req).unwrap_or_else(|e| serde_json::json!({"error": format!("{e:#}")})) } else { serde_json::json!({"error": "the computer refused: wrong MIMIR_COMPUTER_TOKEN"}) };
            let _ = writeln!(&c, "{out}");
        });
    }
    Ok(())
}

/// One request from Mimir, carried out here.
fn act(home: &Path, v: &serde_json::Value) -> Result<serde_json::Value> {
    use base64::Engine;
    let n = |k: &str| v[k].as_i64().unwrap_or(0);
    let s = |k: &str| v[k].as_str().unwrap_or("");
    match s("op") {
        "status" => return Ok(serde_json::json!({"running": running()})),
        "screenshot" => return Ok(serde_json::json!({"image": base64::engine::general_purpose::STANDARD.encode(screenshot_jpeg()?)})),
        "start" => start(home)?,
        "stop" => stop(),
        "click" => click(n("x"), n("y"), s("button"), v["double"] == true)?,
        "type" => type_text(s("text"))?,
        "key" => key(s("keys"))?,
        "scroll" => scroll(n("x"), n("y"), v["down"] == true, n("amount"))?,
        "open" => open_url(home, s("url"))?,
        op => bail!("unknown request {op}"),
    }
    Ok(serde_json::json!({}))
}

/// Pass a DevTools connection on to Chromium, which only listens inside this container.
fn pass_on(c: std::net::TcpStream) {
    let Ok(up) = std::net::TcpStream::connect(("127.0.0.1", CDP_PORT)) else { return };
    let (Ok(mut c2), Ok(mut up2)) = (c.try_clone(), up.try_clone()) else { return };
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut c2, &mut up2);
        let _ = up2.shutdown(std::net::Shutdown::Write);
    });
    let (mut c, mut up) = (c, up);
    let _ = std::io::copy(&mut up, &mut c);
    let _ = c.shutdown(std::net::Shutdown::Write);
}

// ---------- teaching by showing ----------

/// How long one showing may run, and how many steps it keeps.
pub const TEACH_SECS: u64 = 600;
const TEACH_STEPS: usize = 200;
/// Click screenshots kept while recording; the draft gets a few of them, spread out.
const TEACH_SHOTS: usize = 30;

/// One thing you did while showing. Typing collects into the step that started it.
#[derive(Default)]
struct Step {
    secs: u64,
    page: String,
    what: String,
    typed: Option<String>,
    /// Typing into a password field, or a field it couldn't check: the text isn't kept.
    hidden: bool,
}

struct Recording {
    started: std::time::Instant,
    steps: Vec<Step>,
    shots: Vec<PathBuf>,
    /// The step that typing currently goes into.
    typing: Option<usize>,
    /// Which recording this is, so a late page lookup from an earlier one can't write into it.
    id: u64,
}

static RECORDING: Mutex<Option<Recording>> = Mutex::new(None);
static RECORDINGS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn recording() -> std::sync::MutexGuard<'static, Option<Recording>> {
    RECORDING.lock().unwrap_or_else(|e| e.into_inner())
}

/// What you showed: the steps as lines, and the click screenshots to look at.
pub struct Shown {
    pub steps: Vec<String>,
    pub shots: Vec<PathBuf>,
}

/// Start recording what you do on the computer (a new start drops an earlier recording).
pub fn teach_start() {
    if let Some(old) = recording().replace(Recording {
        started: std::time::Instant::now(),
        steps: vec![],
        shots: vec![],
        typing: None,
        id: RECORDINGS.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    }) {
        old.shots.iter().for_each(|p| drop(std::fs::remove_file(p)));
    }
}

/// While recording: (seconds so far, steps so far).
pub fn teaching() -> Option<(u64, usize)> {
    recording().as_ref().map(|r| (r.started.elapsed().as_secs().min(TEACH_SECS), r.steps.len()))
}

/// Stop recording and hand over what was shown, at most `keep` screenshots (the rest are deleted).
pub fn teach_stop(keep: usize) -> Option<Shown> {
    let r = recording().take()?;
    let mut page = String::new();
    let mut steps = vec![];
    for s in r.steps.iter().filter(|s| !s.what.is_empty()) {
        if !s.page.is_empty() && s.page != page {
            page = s.page.clone();
            steps.push(format!("On {page}:"));
        }
        let what = match (&s.typed, s.hidden) {
            (Some(_), true) => format!("{} (what was typed isn't kept)", s.what),
            (Some(t), false) => format!("{} \"{t}\"", s.what),
            (None, _) => s.what.clone(),
        };
        steps.push(format!("  [{}:{:02}] {what}", s.secs / 60, s.secs % 60));
    }
    // An even spread of the click screenshots.
    let n = r.shots.len();
    let pick: Vec<usize> = if n <= keep { (0..n).collect() } else { (0..keep).map(|i| i * (n - 1) / (keep - 1).max(1)).collect() };
    let mut shots = vec![];
    for (i, p) in r.shots.into_iter().enumerate() {
        if pick.contains(&i) { shots.push(p) } else { let _ = std::fs::remove_file(p); }
    }
    Some(Shown { steps, shots })
}

/// Note one of your inputs while recording, before it's carried out (a click can leave the page). The step's place is
/// taken at once, so quick keystrokes stay in order while the page is looked up.
pub async fn teach_note(home: &Path, a: &serde_json::Value) {
    let n = |k: &str| a.get(k).and_then(serde_json::Value::as_i64).unwrap_or(0);
    let s = |k: &str| a.get(k).and_then(serde_json::Value::as_str).unwrap_or("").to_string();
    let kind = s("kind");
    let (idx, look, id) = {
        let mut g = recording();
        let Some(r) = g.as_mut() else { return };
        let id = r.id;
        let secs = r.started.elapsed().as_secs();
        if secs > TEACH_SECS || r.steps.len() >= TEACH_STEPS {
            return;
        }
        match kind.as_str() {
            "type" => {
                if let Some(t) = r.typing.and_then(|i| r.steps.get_mut(i)) {
                    t.typed.get_or_insert_default().push_str(&s("text"));
                    return;
                }
                r.steps.push(Step { secs, typed: Some(s("text")), hidden: true, ..Default::default() });
                r.typing = Some(r.steps.len() - 1);
                (r.steps.len() - 1, Some(None), id)
            }
            "key" if s("keys") == "BackSpace" && r.typing.is_some() => {
                if let Some(t) = r.typing.and_then(|i| r.steps.get_mut(i)) {
                    t.typed.get_or_insert_default().pop();
                }
                return;
            }
            "scroll" if r.steps.last().is_some_and(|l| l.what.starts_with("scrolled")) => return,
            _ => {
                r.typing = None;
                let what = match kind.as_str() {
                    "key" => format!("pressed {}", s("keys")),
                    "scroll" => format!("scrolled {}", s("direction")),
                    _ => String::new(),
                };
                let page = r.steps.last().map(|l| l.page.clone()).unwrap_or_default();
                r.steps.push(Step { secs, what, page, ..Default::default() });
                (r.steps.len() - 1, (kind == "click").then_some(Some((n("x"), n("y")))), id)
            }
        }
    };
    let Some(at) = look else { return };
    let info = page_info(at).await;
    let shot = match at {
        Some((x, y)) if teaching().is_some() => tokio::task::spawn_blocking({
            let home = home.to_path_buf();
            move || marked_shot(&home, x, y)
        })
        .await
        .ok()
        .and_then(Result::ok),
        _ => None,
    };
    let mut g = recording();
    let Some(r) = g.as_mut().filter(|r| r.id == id) else {
        if let Some(p) = shot {
            let _ = std::fs::remove_file(p);
        }
        return;
    };
    if let Some(p) = shot {
        if r.shots.len() < TEACH_SHOTS { r.shots.push(p) } else { let _ = std::fs::remove_file(p); }
    }
    let Some(step) = r.steps.get_mut(idx) else { return };
    if let Some(v) = &info {
        step.page = page_label(v);
    }
    match at {
        Some((x, y)) => {
            let how = match (s("button").as_str(), a.get("double").and_then(serde_json::Value::as_bool).unwrap_or(false)) {
                ("right", _) => "right-clicked",
                (_, true) => "double-clicked",
                _ => "clicked",
            };
            step.what = format!("{how} {}", info.as_ref().map_or_else(|| format!("at ({x}, {y})"), element_label));
        }
        None => {
            // Kept only when a plain text field in the focused page is sure to have it; anything else (a password,
            // a field inside a frame, the address bar or a browser dialog) stays hidden.
            step.hidden = !info.as_ref().is_some_and(|v| v["editable"].as_bool() == Some(true) && v["password"].as_bool() == Some(false));
            step.what = match &info {
                Some(v) => format!("typed into {}", element_label(v)),
                None => "typed outside the page (the address bar or a dialog)".into(),
            };
        }
    }
}

/// "Padel Club – Book (https://club.example/book)": the page's title and address, without the query (it can hold
/// tokens).
fn page_label(v: &serde_json::Value) -> String {
    let url = v["url"].as_str().unwrap_or("");
    let url = match url::Url::parse(url) {
        Ok(mut u) => {
            u.set_query(None);
            u.set_fragment(None);
            u.to_string()
        }
        Err(_) => url.to_string(),
    };
    match v["title"].as_str().unwrap_or("") {
        "" => url,
        t => format!("{t} ({url})"),
    }
}

/// `the "Book now" button`, `the "Email" field`…
fn element_label(v: &serde_json::Value) -> String {
    let (tag, text, role) = (v["tag"].as_str().unwrap_or(""), v["text"].as_str().unwrap_or(""), v["role"].as_str().unwrap_or(""));
    if v["password"].as_bool() == Some(true) {
        return "the password field".into();
    }
    if tag.is_empty() {
        return "the browser's tab or address bar".into();
    }
    if tag == "iframe" {
        return "a frame inside the page (an embedded form)".into();
    }
    let kind = match (tag, role) {
        (_, "button") | ("button", _) | ("summary", _) => "button",
        ("a", _) | (_, "link") => "link",
        ("input" | "textarea", _) => "field",
        ("select", _) => "list",
        (_, "") => "",
        (_, r) => r,
    };
    match (text.is_empty(), kind.is_empty()) {
        (false, false) => format!("the \"{text}\" {kind}"),
        (false, true) => format!("\"{text}\""),
        (true, false) => format!("a {kind}"),
        (true, true) => format!("the page (a {tag})"),
    }
}

/// What's under a screen point (or focused, for None) in the Chromium on screen, through its DevTools port.
async fn page_info(at: Option<(i64, i64)>) -> Option<serde_json::Value> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let (x, y) = at.unwrap_or((-1, -1));
    let js = format!(
        "(() => {{ if (document.visibilityState !== 'visible') return null; const X = {x}, Y = {y}; let e;
          if (X < 0 && !document.hasFocus()) return null;
          const a = document.activeElement;
          const editable = !!a && !a.disabled && (a.tagName === 'TEXTAREA' || a.isContentEditable
            || (a.tagName === 'INPUT' && /^(text|search|email|url|tel|number|date|time|datetime-local|month|week)$/.test(a.type)));
          if (X >= 0) {{ const ox = screenX + (outerWidth - innerWidth) / 2, oy = screenY + outerHeight - innerHeight;
            e = Y >= oy ? document.elementFromPoint(X - ox, Y - oy) : null; }}
          else e = document.activeElement;
          e = e && (e.closest('a,button,input,select,textarea,label,summary,[role=button],[role=link],[role=tab],[role=menuitem],[role=option],[role=checkbox]') || e);
          const t = s => (s || '').replace(/\\s+/g, ' ').trim().slice(0, 80);
          const field = e && (e.tagName === 'INPUT' || e.tagName === 'TEXTAREA');
          return {{ url: location.href, title: t(document.title), editable, tag: e ? e.tagName.toLowerCase() : '',
            role: e ? e.getAttribute('role') || '' : '', password: !!(e && e.type === 'password'),
            text: e ? t(e.getAttribute('aria-label') || (field ? (e.labels && e.labels[0] && e.labels[0].innerText) || e.placeholder || e.name : e.innerText) || e.title || e.alt) : '' }};
        }})()"
    );
    // Each tab is asked in turn; only the one on screen (visible) answers.
    let ask_tab = |ws: String| {
        let js = js.clone();
        async move {
            let (mut sock, _) = tokio_tungstenite::connect_async(ws).await.ok()?;
            // One DevTools call at a time: send, then read until its answer.
            let mut id = 0;
            let mut call = async |method: &str, params: serde_json::Value| -> Option<serde_json::Value> {
                id += 1;
                sock.send(Message::text(serde_json::json!({"id": id, "method": method, "params": params}).to_string())).await.ok()?;
                while let Some(Ok(m)) = sock.next().await {
                    let v: serde_json::Value = serde_json::from_str(m.to_text().ok()?).ok()?;
                    if v["id"] == id {
                        return Some(v["result"].clone());
                    }
                }
                None
            };
            // In an isolated world: it sees the page's elements, but not its scripts, so a page can't change what the
            // lookup reports (e.g. make a password field read as plain text).
            let frame = call("Page.getFrameTree", serde_json::json!({})).await?["frameTree"]["frame"]["id"].as_str()?.to_string();
            let ctx = call("Page.createIsolatedWorld", serde_json::json!({"frameId": frame, "worldName": "mimir"})).await?["executionContextId"].as_i64()?;
            let r = call("Runtime.evaluate", serde_json::json!({"expression": js, "returnByValue": true, "contextId": ctx})).await?;
            Some(r["result"]["value"].clone()).filter(|v| v.is_object())
        }
    };
    let get = async {
        let pages: serde_json::Value = reqwest::Client::new().get(format!("{}/json/list", cdp_endpoint().ok()?)).send().await.ok()?.json().await.ok()?;
        // Chromium's own pages (the address bar's popup is one) aren't the page you're on.
        let real = |p: &&serde_json::Value| p["type"] == "page" && !p["url"].as_str().unwrap_or("").starts_with("chrome");
        for ws in pages.as_array()?.iter().filter(real).filter_map(|p| p["webSocketDebuggerUrl"].as_str()) {
            if let Some(v) = ask_tab(ws.to_string()).await {
                return Some(v);
            }
        }
        None
    };
    tokio::time::timeout(Duration::from_secs(2), get).await.ok().flatten()
}

/// A screenshot with a red ring where you clicked, saved in the media folder (where the model may read it).
fn marked_shot(home: &Path, x: i64, y: i64) -> Result<PathBuf> {
    let mut img = image::load_from_memory(&screenshot_jpeg()?)?.to_rgb8();
    for dy in -22i64..=22 {
        for dx in -22i64..=22 {
            let d = ((dx * dx + dy * dy) as f64).sqrt();
            let (px, py) = (x + dx, y + dy);
            if (15.0..=20.0).contains(&d) && px >= 0 && py >= 0 && (px as u32) < img.width() && (py as u32) < img.height() {
                img.put_pixel(px as u32, py as u32, image::Rgb([230, 30, 30]));
            }
        }
    }
    let dir = home.join("media");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("teach-{}-{}.jpg", crate::db::now(), crate::random_hex(4)?));
    img.save_with_format(&path, image::ImageFormat::Jpeg)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    #[test]
    fn showing_reads_as_steps() {
        use serde_json::json;
        assert_eq!(super::element_label(&json!({"tag": "button", "text": "Book now"})), "the \"Book now\" button");
        assert_eq!(super::element_label(&json!({"tag": "input", "text": "Email"})), "the \"Email\" field");
        assert_eq!(super::element_label(&json!({"tag": "input", "text": "Password", "password": true})), "the password field");
        assert_eq!(super::element_label(&json!({"tag": ""})), "the browser's tab or address bar");
        assert_eq!(super::page_label(&json!({"url": "https://club.example/book?token=abc#x", "title": "Book"})), "Book (https://club.example/book)");
        // Recorded in order; typing collects into one step, and a password field's text isn't kept.
        super::teach_start();
        {
            let mut g = super::recording();
            let r = g.as_mut().unwrap();
            r.steps.push(super::Step { page: "Club (https://club.example/)".into(), what: "clicked the \"Log in\" button".into(), ..Default::default() });
            r.steps.push(super::Step { page: "Club (https://club.example/)".into(), what: "typed into the \"Email\" field".into(), typed: Some("me@x.com".into()), ..Default::default() });
            r.steps.push(super::Step { page: "Club (https://club.example/)".into(), what: "typed into the password field".into(), typed: Some("hunter2".into()), hidden: true, ..Default::default() });
        }
        let shown = super::teach_stop(6).unwrap();
        assert_eq!(shown.steps[0], "On Club (https://club.example/):");
        assert!(shown.steps[2].ends_with("typed into the \"Email\" field \"me@x.com\""), "{}", shown.steps[2]);
        assert!(!shown.steps.join("\n").contains("hunter2"));
        assert!(super::teaching().is_none());
    }


    #[test]
    fn image_results_only_point_into_the_media_folder() {
        let home = std::path::Path::new("/data");
        let (img, text) = super::split_image("[[image:/data/media/screen-1.jpg]]\nClicked.", home);
        assert_eq!((img.as_deref(), text.as_str()), (Some(std::path::Path::new("/data/media/screen-1.jpg")), "Clicked."));
        for sneaky in ["[[image:/etc/passwd]]\nx", "[[image:/data/media/../mimir.db]]\nx", "plain text"] {
            assert_eq!(super::split_image(sneaky, home).0, None, "{sneaky}");
        }
        assert!(super::key("ctrl+l Return").is_err() || super::available(), "no xdotool here, but the keys are valid");
        assert!(super::key("ctrl+l; rm -rf /").is_err(), "only key names");
        assert!(super::click(5000, 10, "left", false).is_err(), "off screen");
    }
}
