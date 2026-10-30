//! Commands a bot runs in its working folder (shell_run): off unless the person turns it on for that bot, every one
//! asked for on a card showing exactly what runs, and run in a sandbox: no network, no writing outside the folder, no
//! reading the rest of the home folder (keys, other projects), a clean environment (no secrets), 60 seconds at most.
//! macOS: sandbox-exec. Linux: bubblewrap (bwrap), refused without it. Elsewhere: not offered.

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::time::Duration;

const LIMIT: Duration = Duration::from_secs(60);
const MAX_OUT: usize = 20_000;

/// Whether commands can be run sandboxed here at all.
pub fn available() -> bool {
    static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OK.get_or_init(|| {
        if cfg!(target_os = "macos") {
            Path::new("/usr/bin/sandbox-exec").exists()
        } else if cfg!(target_os = "linux") {
            // Installed isn't enough: some systems (Ubuntu 24.04's AppArmor, Docker) don't let it make its sandbox. Tried once.
            std::process::Command::new("bwrap")
                .args(["--ro-bind", "/", "/", "--dev", "/dev", "--unshare-net", "--unshare-pid", "--die-with-parent", "/bin/true"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        } else {
            false
        }
    })
}

/// The sandbox's rules on macOS, for a folder given as its real path (links resolved: /tmp is /private/tmp).
pub(crate) fn mac_profile(dir: &Path, home: &Path) -> String {
    let q = |p: &Path| p.display().to_string().replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "(version 1)(allow default)(deny network*)\
         (deny file-write* (require-not (subpath \"{d}\")))(allow file-write* (literal \"/dev/null\"))\
         (deny file-read* (require-all (subpath \"{h}\") (require-not (subpath \"{d}\"))))",
        d = q(dir),
        h = q(home)
    )
}

/// Run `command` with /bin/sh in `dir` (the bot's working folder, as stored: links resolved), sandboxed. Returns its
/// output (stdout then stderr, cut at 20,000 characters) and how it ended.
pub async fn run(dir: &Path, command: &str) -> Result<String> {
    if !available() {
        bail!("commands can't be run safely on this machine (macOS, or Linux with bubblewrap, only)");
    }
    let dir = dir.canonicalize().with_context(|| format!("the working folder {} isn't there", dir.display()))?;
    let home = crate::user_home().and_then(|h| h.canonicalize().ok()).unwrap_or_else(|| "/nonexistent".into());
    let tmp = dir.join(".mimir-tmp");
    std::fs::create_dir_all(&tmp)?;
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = tokio::process::Command::new("/usr/bin/sandbox-exec");
        c.args(["-p", &mac_profile(&dir, &home), "/bin/sh", "-c", command]);
        c
    } else {
        let d = dir.display().to_string();
        let mut c = tokio::process::Command::new("bwrap");
        c.args(["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--tmpfs", "/tmp"]);
        c.args(["--tmpfs", &home.display().to_string(), "--bind", &d, &d, "--unshare-net", "--unshare-pid", "--die-with-parent", "--chdir", &d]);
        c.args(["/bin/sh", "-c", command]);
        c
    };
    // Nothing of Mimir's environment (API keys, tokens) reaches it.
    cmd.current_dir(&dir).env_clear().env("PATH", "/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin").env("HOME", &dir).env("TMPDIR", &tmp).env("LANG", "en_US.UTF-8");
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    let mut child = cmd.spawn().context("couldn't start the command")?;
    let (mut so, mut se) = (child.stdout.take().context("no output")?, child.stderr.take().context("no output")?);
    // Kept only up to a bound while it runs (a command printing forever can't fill Mimir's memory); past it, stopped.
    let cap = (MAX_OUT * 4) as u64;
    let run = async {
        use tokio::io::AsyncReadExt;
        let (mut out, mut err, mut over) = (vec![], vec![], false);
        let (mut b1, mut b2) = ([0u8; 8192], [0u8; 8192]);
        let (mut o_done, mut e_done) = (false, false);
        while !(o_done && e_done) {
            tokio::select! {
                n = so.read(&mut b1), if !o_done => match n {
                    Ok(0) | Err(_) => o_done = true,
                    Ok(n) => out.extend_from_slice(&b1[..n]),
                },
                n = se.read(&mut b2), if !e_done => match n {
                    Ok(0) | Err(_) => e_done = true,
                    Ok(n) => err.extend_from_slice(&b2[..n]),
                },
            }
            if (out.len() + err.len()) as u64 > cap {
                over = true;
                let _ = child.start_kill();
                break;
            }
        }
        (out, err, over, child.wait().await)
    };
    let (out, err, over, status) = match tokio::time::timeout(LIMIT, run).await {
        Ok(r) => r,
        Err(_) => bail!("the command took more than a minute and was stopped"),
    };
    let mut text = String::from_utf8_lossy(&out).to_string();
    let err = String::from_utf8_lossy(&err);
    if !err.trim().is_empty() {
        text.push_str(&format!("\n[stderr]\n{err}"));
    }
    let code = if over { "stopped: too much output".to_string() } else { status.ok().and_then(|s| s.code()).map_or("stopped".to_string(), |c| c.to_string()) };
    Ok(format!("exit {code}\n{}", crate::tools::truncate(text.trim_end(), MAX_OUT)))
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn a_command_stays_in_its_folder() {
        if !super::available() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("mimir-shell-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let out = super::run(&dir, "mkdir -p sub && echo hi > sub/a.txt && cat sub/a.txt && echo $HOME").await.unwrap();
        assert!(out.starts_with("exit 0") && out.contains("hi") && out.contains(&dir.display().to_string()), "{out}");
        assert_eq!(std::fs::read_to_string(dir.join("sub/a.txt")).unwrap(), "hi\n");
        // Writing outside, the network, the home folder's other files, Mimir's secrets: none of them.
        let outside = std::env::temp_dir().join(format!("mimir-shell-outside-{}", std::process::id()));
        let out = super::run(&dir, &format!("echo x > {}", outside.display())).await.unwrap();
        assert!(!out.starts_with("exit 0") && !outside.exists(), "{out}");
        let home = crate::user_home().unwrap();
        let out = super::run(&dir, &format!("ls {}", home.display())).await.unwrap();
        assert!(!out.starts_with("exit 0"), "the home folder isn't readable: {out}");
        let out = super::run(&dir, "env").await.unwrap();
        assert!(!out.contains("API_KEY") && !out.contains("TOKEN"), "{out}");
        if std::process::Command::new("curl").arg("--version").output().is_ok() {
            let out = super::run(&dir, "curl -s -m 5 https://example.com").await.unwrap();
            assert!(!out.starts_with("exit 0"), "no network: {out}");
        }
        // Endless output: stopped at the bound, not kept in memory.
        let out = super::run(&dir, "yes").await.unwrap();
        assert!(out.starts_with("exit stopped: too much output"), "{}", &out[..60.min(out.len())]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
