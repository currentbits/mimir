//! Saved logins brought in from a password manager's CSV export (Chrome, Edge, Safari / Apple Passwords, Firefox,
//! 1Password, Bitwarden, LastPass): first a preview with sites and user names (never the passwords), then the rows the
//! person picks become saved logins, one per site, each only for that site and asked for on every use like any other.

use anyhow::{Result, bail};
use serde::Serialize;

/// One login in the file.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    /// Its place in the file, to pick it by.
    pub i: usize,
    pub site: String,
    pub user: String,
    pub totp: bool,
    /// The name it would be saved under (MIMIR's NAME), and whether a saved login already has this site and user.
    pub name: String,
    pub exists: bool,
    #[serde(skip)]
    password: String,
    #[serde(skip)]
    otp: String,
}

const MAX_ROWS: usize = 5000;

/// Fields of a CSV (RFC 4180: quotes, "" inside quotes, line breaks inside quotes).
fn csv(text: &str) -> Vec<Vec<String>> {
    let (mut rows, mut row, mut field, mut quoted) = (vec![], vec![], String::new(), false);
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            ('"', true) => quoted = false,
            ('"', false) if field.is_empty() => quoted = true,
            (',', false) => row.push(std::mem::take(&mut field)),
            ('\n', false) => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            ('\r', false) => {}
            (c, _) => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.retain(|r| r.iter().any(|f| !f.trim().is_empty()));
    rows
}

/// The column holding one of these (by its header, in any case).
fn col(header: &[String], names: &[&str]) -> Option<usize> {
    header.iter().position(|h| names.contains(&h.trim().to_lowercase().as_str()))
}

/// What's in the file, without the passwords leaving here.
pub fn read(db: &crate::db::Db, text: &str) -> Result<Vec<Row>> {
    let rows = csv(text);
    let Some((header, body)) = rows.split_first() else { bail!("the file is empty") };
    let (Some(url), Some(pass)) = (col(header, &["url", "login_uri", "website", "web site", "login url"]), col(header, &["password", "login_password"])) else {
        bail!("that doesn't look like a password export: it needs a URL and a password column (export as CSV from your password manager)")
    };
    let user = col(header, &["username", "login_username", "user name", "user", "email", "login"]);
    let otp = col(header, &["otpauth", "totp", "login_totp", "otp", "one-time password"]);
    if body.len() > MAX_ROWS {
        bail!("more than {MAX_ROWS} logins in one file; export a folder or a part of them");
    }
    let saved = crate::ext::vault(db);
    // Each saved login's site and user name, read once (the keychain is slow to ask thousands of times).
    let known: Vec<(String, String)> = saved
        .iter()
        .flat_map(|e| {
            let u = if e.user { crate::host::secret_get(&format!("vault:{}:user", e.name)).unwrap_or_default() } else { String::new() };
            e.domains.iter().map(move |d| (d.clone(), u.clone()))
        })
        .collect();
    let mut out: Vec<Row> = vec![];
    for (i, r) in body.iter().enumerate() {
        let get = |c: Option<usize>| c.and_then(|c| r.get(c)).map(|v| v.trim().to_string()).unwrap_or_default();
        let (u, password) = (get(Some(url)), get(Some(pass)));
        let raw = if u.contains("://") { u.clone() } else { format!("https://{u}") };
        let Some(site) = crate::ext::site_of(&raw) else { continue };
        if password.is_empty() {
            continue;
        }
        let user = get(user);
        // The same login twice in one file (a site listed under two addresses): once.
        if out.iter().any(|o| o.site == site && o.user == user) {
            continue;
        }
        let base = site.to_uppercase().replace(|c: char| !c.is_ascii_alphanumeric(), "_");
        let taken = |n: &str| saved.iter().any(|e| e.name == n) || out.iter().any(|o| o.name == n);
        let name = (1..).map(|k| if k == 1 { base.clone() } else { format!("{base}_{k}") }).find(|n| !taken(n)).unwrap_or(base);
        let exists = known.iter().any(|(d, u)| *d == site && *u == user);
        let otp = get(otp);
        out.push(Row { i, site, user, totp: !otp.is_empty(), name, exists, password, otp });
    }
    if out.is_empty() {
        bail!("no logins with a website and a password in that file");
    }
    Ok(out)
}

/// Save the picked rows (by `i`) as saved logins: everyone's, or one bot's. Returns how many.
pub fn import(db: &crate::db::Db, text: &str, pick: &[usize], bot: Option<i64>) -> Result<usize> {
    let mut n = 0;
    for r in read(db, text)?.into_iter().filter(|r| pick.contains(&r.i) && !r.exists) {
        // An authenticator key that doesn't parse is left out, not the whole login.
        let otp = Some(r.otp.as_str()).filter(|o| !o.is_empty() && crate::totp::parse(o).is_ok());
        let entry = crate::ext::VaultEntry { name: r.name.clone(), domains: vec![r.site], bot, totp: false, user: !r.user.is_empty() };
        crate::ext::vault_save(db, entry, Some(&r.password), otp)?;
        if !r.user.is_empty() {
            crate::host::secret_set(&format!("vault:{}:user", r.name), &r.user)?;
        }
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    #[test]
    fn exports_are_read_whatever_manager_made_them() {
        // Chrome's columns, a quoted comma and an escaped quote, and a line break inside quotes.
        let rows = super::csv("name,url,username,password,note\r\nGitHub,https://github.com/login,dana,\"p,a\"\"ss\",\"two\nlines\"\r\n");
        assert_eq!(rows[1], ["GitHub", "https://github.com/login", "dana", "p,a\"ss", "two\nlines"]);
        // Bitwarden's and Safari's headers.
        let bw = super::csv("folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n");
        assert_eq!((super::col(&bw[0], &["url", "login_uri"]), super::col(&bw[0], &["password", "login_password"])), (Some(7), Some(9)));
        let safari = super::csv("Title,URL,Username,Password,Notes,OTPAuth\n");
        assert_eq!(super::col(&safari[0], &["otpauth"]), Some(5));
    }
}
