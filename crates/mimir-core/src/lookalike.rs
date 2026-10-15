//! Names made to pass for ones you know: a sign-in page on `githuh.com` or `github-login.example` when you've signed in
//! to `github.com`. Used for sign-in cards; meant for look-alike email recipients too (#132).

/// The known domain `domain` looks like (without being it), if any. Domains are registrable ones ("github.com"):
/// - names a letter apart ("githuh.com"), or 2 for names of 8 letters or more ("microsfot.com" for microsoft.com):
///   2 for short names would make gitlab.com a look-alike of github.com;
/// - the same name under another ending ("github.dev" for "github.com", "paypal.co.uk" for "paypal.com");
/// - the known name as one part of a hyphenated name ("github-login.example", "secure-paypal.net").
///
/// Names of 3 letters or fewer aren't matched (too many real sites are a letter apart from them). Letters from another
/// alphabet are `foreign` and `skeleton_of`'s.
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

/// A host with letters from another alphabet in it (a label in punycode, `xn--…`): Cyrillic "а" looks just like "a",
/// so these always get a warning, whatever they resemble.
pub fn foreign(host: &str) -> bool {
    host.split('.').any(|l| l.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("xn--")))
}

/// The known domain a punycode one is made to look like: decoded, each letter outside a–z, 0–9 and "-" stands for any
/// letter, so "xn--pple-43d.com" (a Cyrillic "а") reads as "?pple.com", which is apple.com. Same length, every other
/// letter in place, and at least half the name's letters plain ones (a name all in another alphabet matches nothing).
/// ponytail: no confusables table; "?" matches any letter, so the name is a hint and the warning is what counts.
pub fn skeleton_of<'a>(domain: &str, known: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    if !foreign(domain) {
        return None;
    }
    let (decoded, _) = idna::domain_to_unicode(&domain.to_lowercase());
    let plain = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '.';
    let name: Vec<char> = decoded.split('.').next().unwrap_or("").chars().collect();
    if name.iter().filter(|c| plain(**c)).count() * 2 < name.len() {
        return None;
    }
    let d: Vec<char> = decoded.chars().collect();
    known.into_iter().find(|k| {
        let k: Vec<char> = k.chars().collect();
        k.len() == d.len() && d.iter().zip(&k).all(|(a, b)| !plain(*a) || a == b)
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

    #[test]
    fn other_alphabets() {
        let known = ["apple.com", "github.com", "paypal.com"];
        // "аpple.com" and "githuЬ.com", with a Cyrillic а and Ь.
        let (apple, github) = (idna::domain_to_ascii("аpple.com").unwrap(), idna::domain_to_ascii("githuЬ.com").unwrap());
        assert!(apple.starts_with("xn--") && foreign(&apple) && foreign(&format!("login.{github}")));
        assert_eq!(skeleton_of(&apple, known), Some("apple.com"));
        assert_eq!(skeleton_of(&github, known), Some("github.com"));
        assert_eq!(skeleton_of(&idna::domain_to_ascii("раураl.com").unwrap(), known), None, "all Cyrillic: still foreign, matches nothing");
        assert_eq!(skeleton_of(&idna::domain_to_ascii("bücher.de").unwrap(), known), None, "foreign, but like none of them");
        assert!(!foreign("apple.com") && skeleton_of("apple.com", known).is_none());
    }
}
