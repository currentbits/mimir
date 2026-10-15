//! Authenticator codes (RFC 6238 TOTP) for saved logins. Mimir works them out itself from the key you pasted once,
//! so the key never reaches the AI, and a code is typed only on its login's sites.

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Algo {
    Sha1,
    Sha256,
    Sha512,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Totp {
    key: Vec<u8>,
    algo: Algo,
    digits: u32,
    period: u64,
}

/// A setup key as sites show it ("JBSW Y3DP EHPK 3PXP", any case), or the otpauth://totp/… link in the QR code.
pub fn parse(s: &str) -> Result<Totp> {
    let s = s.trim();
    if !s.get(..10).is_some_and(|p| p.eq_ignore_ascii_case("otpauth://")) {
        return Ok(Totp { key: base32(s)?, algo: Algo::Sha1, digits: 6, period: 30 });
    }
    let u = url::Url::parse(s).context("that otpauth:// link doesn't read")?;
    if u.host_str() != Some("totp") {
        bail!("only time-based codes (otpauth://totp/…) work here");
    }
    let q = |k: &str| u.query_pairs().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.into_owned());
    let algo = match q("algorithm").unwrap_or_default().to_ascii_uppercase().as_str() {
        "" | "SHA1" => Algo::Sha1,
        "SHA256" => Algo::Sha256,
        "SHA512" => Algo::Sha512,
        a => bail!("unknown algorithm {a}"),
    };
    let digits = q("digits").map(|d| d.parse()).transpose().context("digits isn't a number")?.unwrap_or(6);
    let period = q("period").map(|p| p.parse()).transpose().context("period isn't a number")?.unwrap_or(30);
    if !(6..=8).contains(&digits) || !(1..=300).contains(&period) {
        bail!("codes have 6 to 8 digits and change at most every 5 minutes");
    }
    Ok(Totp { key: base32(&q("secret").context("the link has no secret")?)?, algo, digits, period })
}

/// RFC 4648 base32, forgiving about case, spaces, dashes and padding.
fn base32(s: &str) -> Result<Vec<u8>> {
    let (mut out, mut buf, mut bits) = (vec![], 0u32, 0);
    for c in s.chars().filter(|c| !c.is_whitespace() && !matches!(c, '-' | '=')) {
        let v = match c.to_ascii_uppercase() {
            c @ 'A'..='Z' => c as u32 - 'A' as u32,
            c @ '2'..='7' => c as u32 - '2' as u32 + 26,
            _ => bail!("a setup key has only the letters A to Z and the digits 2 to 7"),
        };
        buf = ((buf << 5) | v) & 0xfff;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    // 80 bits is the least sites hand out; shorter is a typo (or a 6-digit code pasted by mistake).
    if out.len() < 10 {
        bail!("that setup key is too short; paste the whole key, or the otpauth:// link");
    }
    Ok(out)
}

fn hash(algo: Algo, parts: &[&[u8]]) -> Vec<u8> {
    match algo {
        Algo::Sha1 => {
            use sha1::Digest;
            let mut h = sha1::Sha1::new();
            parts.iter().for_each(|p| h.update(p));
            h.finalize().to_vec()
        }
        Algo::Sha256 => {
            use sha2::Digest;
            let mut h = sha2::Sha256::new();
            parts.iter().for_each(|p| h.update(p));
            h.finalize().to_vec()
        }
        Algo::Sha512 => {
            use sha2::Digest;
            let mut h = sha2::Sha512::new();
            parts.iter().for_each(|p| h.update(p));
            h.finalize().to_vec()
        }
    }
}

/// RFC 2104.
fn hmac(algo: Algo, key: &[u8], msg: &[u8]) -> Vec<u8> {
    let block = if algo == Algo::Sha512 { 128 } else { 64 };
    let mut k = if key.len() > block { hash(algo, &[key]) } else { key.to_vec() };
    k.resize(block, 0);
    let pad = |b: u8| k.iter().map(|x| x ^ b).collect::<Vec<u8>>();
    let inner = hash(algo, &[&pad(0x36), msg]);
    hash(algo, &[&pad(0x5c), &inner])
}

impl Totp {
    /// The code at a unix time.
    pub fn at(&self, unix: u64) -> String {
        let mac = hmac(self.algo, &self.key, &(unix / self.period).to_be_bytes());
        let o = (mac[mac.len() - 1] & 0xf) as usize;
        let n = u32::from_be_bytes([mac[o], mac[o + 1], mac[o + 2], mac[o + 3]]) & 0x7fff_ffff;
        format!("{:0w$}", n % 10u32.pow(self.digits), w = self.digits as usize)
    }

    pub fn now(&self) -> String {
        self.at(crate::db::now() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6238_vectors() {
        let key = |n: usize| b"1234567890".iter().cycle().take(n).copied().collect::<Vec<u8>>();
        let t = |algo, n| Totp { key: key(n), algo, digits: 8, period: 30 };
        let (sha1, sha256, sha512) = (t(Algo::Sha1, 20), t(Algo::Sha256, 32), t(Algo::Sha512, 64));
        for (time, a, b, c) in [
            (59, "94287082", "46119246", "90693936"),
            (1111111109, "07081804", "68084774", "25091201"),
            (1111111111, "14050471", "67062674", "99943326"),
            (1234567890, "89005924", "91819424", "93441116"),
            (2000000000, "69279037", "90698825", "38618901"),
            (20000000000, "65353130", "77737706", "47863826"),
        ] {
            assert_eq!((sha1.at(time).as_str(), sha256.at(time).as_str(), sha512.at(time).as_str()), (a, b, c), "at {time}");
        }
    }

    #[test]
    fn setup_keys_and_links() {
        // "12345678901234567890" in base32, as a site would show it.
        let plain = parse("gezd gnbv gy3t qojq gezd gnbv gy3t qojq").unwrap();
        assert_eq!(plain.at(59), "287082", "6 digits by default: the vector's last six");
        let link = parse("otpauth://totp/Example:me@example.com?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Example&digits=8").unwrap();
        assert_eq!(link.at(59), "94287082");
        let sha256 = parse("otpauth://totp/x?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZA====&algorithm=SHA256&digits=8&period=30").unwrap();
        assert_eq!(sha256.at(1111111109), "68084774");
        assert!(parse("123456").is_err(), "a code isn't a key");
        assert!(parse("JBSWY3DP").is_err(), "too short");
        assert!(parse("otpauth://hotp/x?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&counter=1").is_err(), "counter-based codes aren't supported");
        assert!(parse("otpauth://totp/x?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&digits=12").is_err());
    }
}
