//! The bots' own computer, on a self-hosted Linux server (the Docker image has what it needs): a virtual screen
//! (Xvfb) with a window manager and a visible Chromium. Bots see it in screenshots and use it with the mouse and
//! keyboard; the browser tools drive the same Chromium; you can watch and take over in the web app.
//! It runs on its own: with docker compose it's a separate service (`mimir computer`, its own user, none of Mimir's
//! data) that Mimir drives over a small control channel and Chromium's DevTools port; a plain `docker run` keeps it in
//! Mimir's container. Never on your own machine.
//! Each bot has its own Chromium profile, so a site you sign in to for one bot isn't signed in for the others: when
//! another bot uses the computer, Chromium restarts with that bot's profile.
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

/// Where the computer service is and its token: an AI CLI must hand these on to the `mimir mcp` it starts, or that
/// process runs a computer of its own, inside Mimir's container.
pub const ENV: [&str; 2] = ["MIMIR_COMPUTER", "MIMIR_COMPUTER_TOKEN"];

/// The computer as a separate service (`MIMIR_COMPUTER=computer:7300`, docker compose); unset, it runs here.
fn remote() -> Option<String> {
    std::env::var(ENV[0]).ok().filter(|v| !v.trim().is_empty())
}

/// The secret both sides share for the control channel.
fn token() -> Result<String> {
    std::env::var(ENV[1]).ok().filter(|t| t.len() >= 24).context("set MIMIR_COMPUTER_TOKEN (24+ characters, the same for Mimir and the computer)")
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

/// The Authorization header a computer service wants on DevTools (its token); None when the computer runs here.
pub fn cdp_auth() -> Result<Option<String>> {
    remote().map(|_| token().map(|t| format!("Bearer {t}"))).transpose()
}

/// Constant time: no hints about the token from how long a refusal takes.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |d, (x, y)| d | (x ^ y)) == 0
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

/// For `mimir doctor`: None without a computer service, else whether it answers (with the right token).
pub fn reach() -> Option<Result<()>> {
    let a = remote()?;
    Some(ask(&a, serde_json::json!({"op": "status"})).map(drop))
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

fn launch(cmd: &mut Command) -> Result<Child> {
    Ok(cmd.env("DISPLAY", DISPLAY).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?)
}

fn spawn(cmd: &mut Command) -> Result<()> {
    let child = launch(cmd)?;
    PROCS.lock().unwrap_or_else(|e| e.into_inner()).push(child);
    Ok(())
}

/// A bot's own Chromium profile (its cookies and sign-ins).
fn profile(home: &Path, bot: i64) -> Result<PathBuf> {
    let dir = home.join("computer");
    let p = dir.join(format!("chromium-{bot}"));
    // Before profiles were per bot, all bots shared one: the first bot to use the computer since keeps its sign-ins.
    let shared = dir.join("chromium");
    if bot > 0 && !p.exists() && shared.is_dir() {
        std::fs::rename(&shared, &p)?;
    }
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

/// Which bot's Chromium is on screen: (bot, pid), recorded in `computer/browser` by whichever process started it (the
/// server, or a CLI bot's own `mimir mcp`, so not something one process can keep in memory), and only while that pid
/// is still Chromium on that bot's profile.
fn on_screen(home: &Path) -> Option<(i64, u32)> {
    let s = std::fs::read_to_string(home.join("computer").join("browser")).ok()?;
    let (bot, pid) = s.trim().split_once(' ')?;
    let (bot, pid): (i64, u32) = (bot.parse().ok()?, pid.parse().ok()?);
    let dir = format!("--user-data-dir={}", home.join("computer").join(format!("chromium-{bot}")).display());
    let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    cmd.split(|b| *b == 0).any(|a| a == dir.as_bytes()).then_some((bot, pid))
}

/// What start does about the browser: keep it, close the one open (if any) and start the bot's, or leave alone a
/// Chromium on the DevTools port that no bot's record accounts for (driving it could mean anybody's sign-ins).
#[derive(Debug, PartialEq)]
enum Next {
    Keep,
    Switch(Option<u32>),
    Unknown,
}

fn next(on: Option<(i64, u32)>, want: i64, up: bool) -> Next {
    match on {
        Some((b, _)) if b == want && up => Next::Keep,
        Some((_, pid)) => Next::Switch(Some(pid)),
        None if up => Next::Unknown,
        None => Next::Switch(None),
    }
}

/// One process at a time decides which Chromium is on screen.
fn browser_lock(home: &Path) -> Result<std::fs::File> {
    let dir = home.join("computer");
    std::fs::create_dir_all(&dir)?;
    let f = std::fs::File::create(dir.join("browser.lock"))?;
    f.lock()?;
    Ok(f)
}

/// Close Chromium the way a person would, so it writes its cookies out (killed if it hangs), and wait for its port.
fn quit(pid: u32) {
    // A zombie (exited, not yet collected by its parent) has an empty command line.
    let alive = || std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|c| !c.is_empty());
    let _ = Command::new("kill").args(["-TERM", &pid.to_string()]).status();
    for _ in 0..50 {
        PROCS.lock().unwrap_or_else(|e| e.into_inner()).retain_mut(|c| matches!(c.try_wait(), Ok(None)));
        if !alive() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if alive() {
        let _ = Command::new("kill").args(["-KILL", &pid.to_string()]).status();
    }
    for _ in 0..30 {
        if !browser_up() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Start the screen, window manager and browser if they aren't running (again after a crash). Idempotent. With a
/// bot, the browser runs that bot's profile (restarting it if another bot's is open); without, it keeps the one open.
pub fn start(home: &Path, bot: Option<i64>) -> Result<()> {
    if let Some(a) = remote() {
        return ask(&a, serde_json::json!({"op": "start", "bot": bot})).map(drop);
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
    let _lock = browser_lock(home)?;
    let on = on_screen(home);
    let bot = bot.or(on.map(|(b, _)| b)).unwrap_or(0);
    let close = match next(on, bot, browser_up()) {
        Next::Keep => return Ok(()),
        Next::Unknown => bail!("another Chromium has the computer's DevTools port; turn the computer off and on in Settings"),
        Next::Switch(pid) => pid,
    };
    if let Some(pid) = close {
        quit(pid);
    }
    if browser_up() {
        bail!("the open Chromium didn't close; turn the computer off and on in Settings");
    }
    let profile = profile(home, bot)?;
    // --no-sandbox: Chromium's own sandbox needs user namespaces, which containers usually don't allow; the
    // container is the sandbox here.
    let child = launch(browser(home)?.args([
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
    std::fs::write(home.join("computer").join("browser"), format!("{bot} {}", child.id()))?;
    PROCS.lock().unwrap_or_else(|e| e.into_inner()).push(child);
    for _ in 0..80 {
        if browser_up() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

/// Stop everything this server started, and the browser on screen.
pub fn stop(home: &Path) {
    if let Some(a) = remote() {
        let _ = ask(&a, serde_json::json!({"op": "stop"}));
        return;
    }
    if let Ok(_lock) = browser_lock(home)
        && let Some((_, pid)) = on_screen(home)
    {
        quit(pid);
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

/// Open a web address in the computer's Chromium (a new tab of the one on screen), in the bot's profile.
pub fn open_url(home: &Path, bot: i64, url: &str) -> Result<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        bail!("open takes a web address starting with https://");
    }
    if let Some(a) = remote() {
        return ask(&a, serde_json::json!({"op": "open", "bot": bot, "url": url})).map(drop);
    }
    start(home, Some(bot))?;
    let profile = profile(home, bot)?;
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

/// Whose browser profile is on the screen now (the record `start` keeps, checked against the running Chromium), or
/// None when that can't be told: then nobody's login may be typed there.
pub fn screen_bot(home: &Path) -> Option<i64> {
    match remote() {
        Some(a) => ask(&a, serde_json::json!({"op": "status"})).ok()?["bot"].as_i64(),
        None => on_screen(home).map(|(b, _)| b),
    }
}

/// Sign a bot out of every site on the computer: its profile is emptied (closing the browser if it's that bot's).
pub fn forget(home: &Path, bot: i64) -> Result<()> {
    if let Some(a) = remote() {
        return ask(&a, serde_json::json!({"op": "forget", "bot": bot})).map(drop);
    }
    let _lock = browser_lock(home)?;
    if let Some((b, pid)) = on_screen(home)
        && b == bot
    {
        quit(pid);
    }
    let p = profile(home, bot)?;
    std::fs::remove_dir_all(&p)?;
    // Left empty, so the shared profile from before can't move in.
    std::fs::create_dir_all(&p)?;
    Ok(())
}

// ---------- the computer as a service of its own ----------

/// `mimir computer --listen 0.0.0.0:7300`: run the screen and browser here and take Mimir's requests (screenshots,
/// clicks, typing, keys, opening pages) over a control channel that needs MIMIR_COMPUTER_TOKEN. Chromium's DevTools
/// are passed on at CDP_SHARED_PORT on the same address, behind the same token. Meant for a container with none of Mimir's data in it.
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
    let t = token.clone();
    std::thread::spawn(move || {
        for c in cdp.incoming().flatten() {
            let t = t.clone();
            std::thread::spawn(move || pass_on(c, &t));
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
            let out = if same(req["token"].as_str().unwrap_or(""), &token) { act(&home, &req).unwrap_or_else(|e| serde_json::json!({"error": format!("{e:#}")})) } else { serde_json::json!({"error": "the computer refused: wrong MIMIR_COMPUTER_TOKEN"}) };
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
        "status" => return Ok(serde_json::json!({"running": running(), "bot": on_screen(home).map(|(b, _)| b)})),
        "screenshot" => return Ok(serde_json::json!({"image": base64::engine::general_purpose::STANDARD.encode(screenshot_jpeg()?)})),
        "start" => start(home, v["bot"].as_i64())?,
        "stop" => stop(home),
        "click" => click(n("x"), n("y"), s("button"), v["double"] == true)?,
        "type" => type_text(s("text"))?,
        "key" => key(s("keys"))?,
        "scroll" => scroll(n("x"), n("y"), v["down"] == true, n("amount"))?,
        "open" => open_url(home, n("bot"), s("url"))?,
        "forget" => forget(home, n("bot"))?,
        op => bail!("unknown request {op}"),
    }
    Ok(serde_json::json!({}))
}

/// Pass a DevTools connection on to Chromium, which only listens inside this container, once its first request
/// carries the token (`Authorization: Bearer …`): DevTools is the whole browser, saved logins and all.
fn pass_on(mut c: std::net::TcpStream, token: &str) {
    use std::io::{Read, Write};
    // The request's head: at most 16 KB, within 10 s.
    let start = std::time::Instant::now();
    let _ = c.set_read_timeout(Some(Duration::from_secs(10)));
    let (mut head, mut buf) = (vec![], [0u8; 4096]);
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        match c.read(&mut buf) {
            Ok(n) if n > 0 && head.len() + n <= 16 << 10 && start.elapsed() < Duration::from_secs(10) => head.extend_from_slice(&buf[..n]),
            _ => return,
        }
    }
    if !authorized(&head, token) {
        let _ = c.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    }
    let _ = c.set_read_timeout(None);
    let Ok(mut up) = std::net::TcpStream::connect(("127.0.0.1", CDP_PORT)) else { return };
    if up.write_all(&head).is_err() {
        return;
    }
    let (Ok(mut c2), Ok(mut up2)) = (c.try_clone(), up.try_clone()) else { return };
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut c2, &mut up2);
        let _ = up2.shutdown(std::net::Shutdown::Write);
    });
    let _ = std::io::copy(&mut up, &mut c);
    let _ = c.shutdown(std::net::Shutdown::Write);
}

/// An HTTP request head with `Authorization: Bearer <token>`.
fn authorized(head: &[u8], token: &str) -> bool {
    let head = String::from_utf8_lossy(head);
    let head = head.split("\r\n\r\n").next().unwrap_or("");
    head.lines().skip(1).filter_map(|l| l.split_once(':')).any(|(k, v)| {
        k.trim().eq_ignore_ascii_case("authorization") && v.trim().strip_prefix("Bearer ").is_some_and(|t| same(t.trim(), token))
    })
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
    let get = async { screen_tab().await?.eval(&js).await.filter(|v| v.is_object()) };
    tokio::time::timeout(Duration::from_secs(2), get).await.ok().flatten()
}

/// A DevTools session on the tab on screen, in an isolated world of its page: it sees the page's elements, but not its
/// scripts, so a page can't change what Mimir reads there (e.g. make a password field read as plain text).
struct Tab {
    sock: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    id: i64,
    world: i64,
}

impl Tab {
    async fn open(ws: &str) -> Option<Tab> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut req = ws.into_client_request().ok()?;
        if let Some(h) = cdp_auth().ok()? {
            req.headers_mut().insert("authorization", h.parse().ok()?);
        }
        let (sock, _) = tokio_tungstenite::connect_async(req).await.ok()?;
        let mut tab = Tab { sock, id: 0, world: 0 };
        let frame = tab.call("Page.getFrameTree", serde_json::json!({})).await?["frameTree"]["frame"]["id"].as_str()?.to_string();
        tab.world = tab.call("Page.createIsolatedWorld", serde_json::json!({"frameId": frame, "worldName": "mimir"})).await?["executionContextId"].as_i64()?;
        Some(tab)
    }

    /// One DevTools call at a time: send, then read until its answer.
    async fn call(&mut self, method: &str, params: serde_json::Value) -> Option<serde_json::Value> {
        use futures_util::{SinkExt, StreamExt};
        self.id += 1;
        let msg = serde_json::json!({"id": self.id, "method": method, "params": params}).to_string();
        self.sock.send(tokio_tungstenite::tungstenite::Message::text(msg)).await.ok()?;
        while let Some(Ok(m)) = self.sock.next().await {
            let v: serde_json::Value = serde_json::from_str(m.to_text().ok()?).ok()?;
            if v["id"] == self.id {
                return v.get("error").is_none().then(|| v["result"].clone());
            }
        }
        None
    }

    async fn eval(&mut self, js: &str) -> Option<serde_json::Value> {
        let r = self.call("Runtime.evaluate", serde_json::json!({"expression": js, "returnByValue": true, "contextId": self.world})).await?;
        Some(r["result"]["value"].clone())
    }
}

/// The tab on screen in the computer's Chromium: each page tab is asked in turn; only the visible one is it.
async fn screen_tab() -> Option<Tab> {
    let mut list = reqwest::Client::new().get(format!("{}/json/list", cdp_endpoint().ok()?));
    if let Some(h) = cdp_auth().ok()? {
        list = list.header("Authorization", h);
    }
    let pages: serde_json::Value = list.send().await.ok()?.json().await.ok()?;
    // Chromium's own pages (the address bar's popup is one) aren't the page you're on.
    let real = |p: &&serde_json::Value| p["type"] == "page" && !p["url"].as_str().unwrap_or("").starts_with("chrome");
    for ws in pages.as_array()?.iter().filter(real).filter_map(|p| p["webSocketDebuggerUrl"].as_str()) {
        if let Some(mut tab) = Tab::open(ws).await
            && tab.eval("document.visibilityState === 'visible'").await == Some(serde_json::Value::Bool(true))
        {
            return Some(tab);
        }
    }
    None
}

/// The sign-in fields on the page (main frame only: an embedded form from another site isn't the page's own), found
/// the way password managers do. `F` is the expression's result: {url, user, password, code}, each field or undefined.
const FIELDS: &str = "(() => {
  const shown = e => !e.disabled && !e.readOnly && e.getClientRects().length > 0 && getComputedStyle(e).visibility !== 'hidden';
  const ins = [...document.querySelectorAll('input')].filter(shown);
  const hint = e => [e.name, e.id, e.autocomplete, e.placeholder, e.getAttribute('aria-label')].join(' ');
  const code = ins.find(e => e.autocomplete === 'one-time-code' || (/^(text|tel|number|password)$/.test(e.type)
    && /otp|totp|2fa|mfa|one.?time|passcode|verif|security.?code|auth.?code|\\bcode\\b/i.test(hint(e))));
  const password = ins.find(e => e.type === 'password' && e !== code);
  const user = ins.find(e => e !== code && (/^(email|tel)$/.test(e.type) || (e.type === 'text' && /user|e-?mail|login|account|identifier|phone/i.test(hint(e)))));
  return { url: location.href, user, password, code };
})()";

/// Which sign-in fields the page on screen has, and its address: {url, user, password, code} (true/false).
pub async fn sign_in_form() -> Option<serde_json::Value> {
    let js = format!("(() => {{ const f = {FIELDS}; return {{ url: f.url, user: !!f.user, password: !!f.password, code: !!f.code }} }})()");
    let get = async { screen_tab().await?.eval(&js).await.filter(|v| v.is_object()) };
    tokio::time::timeout(Duration::from_secs(3), get).await.ok().flatten()
}

/// What Mimir types into a sign-in for you. Empty fields are left alone.
#[derive(Default)]
pub struct Login {
    pub user: String,
    pub password: String,
    /// A code you typed in, or worked out from the login's authenticator key at the moment it's typed.
    pub code: String,
    pub totp: Option<crate::totp::Totp>,
}

/// Type a login into the page on screen, only while that page is on one of `domains` (checked by the same script that
/// fills each field), and press Enter, step by step
/// (a site that asks the email first, then the password, then a code). Each field is typed once: a password field
/// back again after Mimir typed the password means it was wrong, and isn't tried again (which could lock the
/// account). True when signed in: no password or code field left on the page.
/// ponytail: "signed in" is read from the fields going away; a site that keeps a password field on its home page
/// reads as failed.
/// Only into `bot`'s own browser profile: checked before the first field and before each later step, so a login is never
/// typed into another bot's Chromium (one that took the computer while this bot waited for an OK, say).
pub async fn sign_in(home: &Path, bot: i64, login: &Login, domains: &[String]) -> Result<bool> {
    let home = home.to_path_buf();
    let mine = move || {
        let home = home.clone();
        async move {
            match tokio::task::spawn_blocking(move || screen_bot(&home)).await? {
                Some(b) if b == bot => Ok(()),
                _ => bail!("the computer isn't on this bot's browser now (another bot used it meanwhile); nothing was typed. Call computer_signin again"),
            }
        }
    };
    fill(login, domains, mine).await
}

async fn fill<F: std::future::Future<Output = Result<()>>>(login: &Login, domains: &[String], mine: impl Fn() -> F) -> Result<bool> {
    let mut typed: Vec<&str> = vec![];
    loop {
        mine().await?;
        let mut tab = screen_tab().await.context("couldn't reach the browser on the computer")?;
        let js = format!("(() => {{ const f = {FIELDS}; return {{ url: f.url, user: !!f.user, password: !!f.password, code: !!f.code }} }})()");
        let seen = tab.eval(&js).await.filter(|v| v.is_object()).context("couldn't read the page on the computer")?;
        let url = seen["url"].as_str().unwrap_or("");
        if !crate::ext::host_allowed(url, domains) {
            bail!("the computer is on {} now, not {}", url.split(['?', '#']).next().unwrap_or(""), domains.join(", "));
        }
        let on = |k: &str| seen[k] == true;
        if typed.iter().any(|k| *k != "user" && on(k)) {
            return Ok(false);
        }
        let code = if login.code.is_empty() { login.totp.as_ref().map(|t| t.now()).unwrap_or_default() } else { login.code.clone() };
        let mut now = 0;
        for (k, v) in [("user", login.user.as_str()), ("password", login.password.as_str()), ("code", code.as_str())] {
            if v.is_empty() || !on(k) || typed.contains(&k) {
                continue;
            }
            // The site check and the typing are one script in the page's isolated world, run without a break in
            // between: a page that has just gone to another site can't get the value. Set the way a framework (React,
            // Vue…) still notices: the input's own value setter, then input and change events.
            let fill = format!(
                "(() => {{ const D = {}, h = location.hostname.toLowerCase();
                  if (!/^https?:$/.test(location.protocol) || !D.some(d => h === d || h.endsWith('.' + d))) return 'elsewhere';
                  const e = {FIELDS}.{k}; if (!e) return 'gone';
                  e.focus(); Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(e, {});
                  e.dispatchEvent(new Event('input', {{ bubbles: true }})); e.dispatchEvent(new Event('change', {{ bubbles: true }}));
                  return 'ok' }})()",
                serde_json::to_string(domains)?,
                serde_json::to_string(v)?
            );
            match tab.eval(&fill).await.as_ref().and_then(|r| r.as_str()) {
                Some("ok") => {}
                Some("elsewhere") => bail!("the page went to another site while signing in"),
                _ => bail!("couldn't type into the page"),
            }
            typed.push(k);
            now += 1;
        }
        if now == 0 {
            return Ok(!typed.is_empty() && !on("password") && !on("code"));
        }
        // Enter sends the form, as a person would.
        for (t, text) in [("keyDown", "\r"), ("keyUp", "")] {
            tab.call("Input.dispatchKeyEvent", serde_json::json!({"type": t, "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13, "text": text})).await;
        }
        drop(tab);
        tokio::time::sleep(Duration::from_secs(4)).await;
    }
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
    /// Opt-in, on a Chromium already showing a sign-in page: `MIMIR_SIGNIN_TEST=http://127.0.0.1:8765/login.html`
    /// with `--remote-debugging-port=9222`, where login.html posts to a code page, then to a page without fields.
    #[tokio::test]
    #[ignore]
    async fn signs_in_on_a_real_page() {
        let url = std::env::var("MIMIR_SIGNIN_TEST").unwrap();
        let host = url::Url::parse(&url).unwrap().host_str().unwrap().to_string();
        use super::*;
        let form = sign_in_form().await.unwrap();
        assert_eq!((form["user"].as_bool(), form["password"].as_bool(), form["code"].as_bool()), (Some(true), Some(true), Some(false)), "{form}");
        let login = Login { user: "me@example.com".into(), password: "hunter2!".into(), totp: crate::totp::parse("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ").ok(), ..Default::default() };
        let yes = || async { Ok::<(), anyhow::Error>(()) };
        assert!(fill(&login, &["elsewhere.example".into()], yes).await.is_err(), "not on another site");
        assert!(fill(&login, &[host], yes).await.unwrap(), "email and password, then the code");
        let typed = screen_tab().await.unwrap().eval("document.getElementById('x').textContent").await.unwrap();
        let code = crate::totp::parse("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ").unwrap();
        let now = crate::db::now() as u64;
        assert!([code.at(now), code.at(now - 30)].iter().any(|c| typed == format!("me@example.com|hunter2!|{c}")), "{typed}");
    }

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
    fn devtools_need_the_token() {
        let t = "0123456789abcdef01234567";
        assert!(super::authorized(format!("GET /json/list HTTP/1.1\r\nHost: 10.0.0.2:9223\r\nauthorization: Bearer {t}\r\n\r\n").as_bytes(), t));
        assert!(!super::authorized(b"GET /json/list HTTP/1.1\r\nHost: 10.0.0.2:9223\r\n\r\n", t));
        assert!(!super::authorized(b"GET /json/list HTTP/1.1\r\nAuthorization: Bearer 0123456789abcdef01234568\r\n\r\n", t));
        assert!(!super::authorized(format!("POST / HTTP/1.1\r\nHost: x\r\n\r\nAuthorization: Bearer {t}\r\n").as_bytes(), t), "not in the body");
    }


    #[test]
    fn each_bot_has_its_own_profile() {
        let home = std::env::temp_dir().join(format!("mimir-profiles-{}", std::process::id()));
        let shared = home.join("computer").join("chromium");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("Cookies"), "signed in").unwrap();
        // No bot's browser doesn't take the shared profile; the first bot to use the computer does.
        assert_eq!(super::profile(&home, 0).unwrap(), home.join("computer").join("chromium-0"));
        assert!(shared.is_dir());
        let first = super::profile(&home, 3).unwrap();
        assert_eq!(first, home.join("computer").join("chromium-3"));
        assert!(first.join("Cookies").is_file() && !shared.exists());
        let second = super::profile(&home, 4).unwrap();
        assert!(second.is_dir() && !second.join("Cookies").exists(), "another bot starts signed out");
        // Signing a bot out empties its profile, and only its.
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("Cookies"), "old").unwrap();
        if super::remote().is_none() {
            super::forget(&home, 3).unwrap();
            assert!(!first.join("Cookies").exists() && first.is_dir());
            assert!(super::profile(&home, 3).unwrap().read_dir().unwrap().next().is_none(), "the old shared one doesn't move back in");
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_browser_switches_to_the_bot_using_it() {
        use super::{Next, next};
        assert_eq!(next(Some((3, 40)), 3, true), Next::Keep);
        assert_eq!(next(Some((3, 40)), 4, true), Next::Switch(Some(40)), "another bot's: closed first");
        assert_eq!(next(Some((3, 40)), 3, false), Next::Switch(Some(40)), "not answering: started again");
        assert_eq!(next(None, 3, false), Next::Switch(None));
        // A Chromium nobody recorded (another process's, or one that outlived its record) isn't driven.
        assert_eq!(next(None, 3, true), Next::Unknown);
        // Only a record whose pid is still Chromium on that bot's profile counts.
        let home = std::env::temp_dir().join(format!("mimir-onscreen-{}", std::process::id()));
        std::fs::create_dir_all(home.join("computer")).unwrap();
        std::fs::write(home.join("computer").join("browser"), format!("3 {}", std::process::id())).unwrap();
        assert_eq!(super::on_screen(&home), None, "this test isn't Chromium on bot 3's profile");
        let _ = std::fs::remove_dir_all(&home);
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
