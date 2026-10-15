//! Names made to pass for ones you know: a sign-in page on `githuh.com` or `github-login.example` when you've signed in
//! to `github.com`. Used for sign-in cards; meant for look-alike email recipients too (#132).

/// The known domain `domain` looks like (without being it), if any. Domains are registrable ones ("github.com"):
/// - names a letter apart ("githuh.com"), or 2 for names of 8 letters or more ("microsfot.com" for microsoft.com):
///   2 for short names would make gitlab.com a look-alike of github.com;
/// - the same name under another ending ("github.dev" for "github.com", "paypal.co.uk" for "paypal.com");
/// - the known name as one part of a hyphenated name ("github-login.example", "secure-paypal.net").
///
/// Names of 3 letters or fewer aren't matched (too many real sites are a letter apart from them).
/// ponytail: letters only; a different alphabet's look-alike letters (Cyrillic "а") show as xn--… and are missed.
pub fn of<'a>(domain: &str, known: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let domain = domain.to_lowercase();
    let label = |d: &str| d.split('.').next().unwrap_or("").to_string();
    let name = label(&domain);
    known.into_iter().find(|k| {
        let k_name = label(k);
        *k != domain
            && k_name.chars().count() > 3
            && (distance(&name, &k_name) <= if k_name.chars().count() >= 8 { 2 } else { 1 } || name == k_name || name.split('-').filter(|p| !p.is_empty()).count() > 1 && name.split('-').any(|p| p == k_name))
    })
}

/// Levenshtein distance, by letters.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let next = (row[j + 1] + 1).min(row[j] + 1).min(prev + usize::from(ca != *cb));
            prev = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn look_alikes() {
        let known = ["github.com", "paypal.com", "bbc.co.uk", "ing.nl"];
        assert_eq!(of("githuh.com", known), Some("github.com"));
        assert_eq!(of("github.co", known), Some("github.com"));
        assert_eq!(of("github.dev", known), Some("github.com"), "same name, another ending");
        assert_eq!(of("github-login.example", known), Some("github.com"));
        assert_eq!(of("secure-paypal.net", known), Some("paypal.com"));
        assert_eq!(of("paypal.co.uk", known), Some("paypal.com"));
        assert_eq!(of("github.com", known), None, "the site itself isn't a look-alike");
        assert_eq!(of("GitHub.com", known), None);
        assert_eq!(of("gitlab.com", known), None, "2 letters apart is another site for a short name");
        assert_eq!(of("microsfot.com", ["microsoft.com"]), Some("microsoft.com"));
        assert_eq!(of("ingx.nl", known), None, "short names aren't matched");
        assert_eq!(of("bbd.co.uk", known), None);
        assert_eq!(of("githubusercontent.com", known), None);
        assert_eq!(distance("kitten", "sitting"), 3);
    }
}
