//! Scanned PDF pages read on this computer: a local document model behind Ollama (rednote's dots.ocr, or any vision
//! model you pick) turns each page into markdown, tables included, so the bot gets text instead of photos: less of the
//! AI plan per page, and the scan stays here. Off unless a model is set (setting `ocr_model`).

use anyhow::{Context, Result, bail};
use std::time::Duration;

/// Where Ollama listens unless `ocr_url` says otherwise.
pub const DEFAULT_URL: &str = "http://localhost:11434";
const PROMPT: &str = "Convert this scanned page to markdown. Keep tables as markdown tables. Output only the page's content, nothing else.";

/// The model and address, when reading scans here is switched on.
pub fn configured(db: &crate::db::Db) -> Option<(String, String)> {
    let model = db.setting("ocr_model").ok().flatten().map(|m| m.trim().to_string()).filter(|m| !m.is_empty())?;
    let url = db.setting("ocr_url").ok().flatten().map(|u| u.trim().trim_end_matches('/').to_string()).filter(|u| !u.is_empty()).unwrap_or_else(|| DEFAULT_URL.into());
    Some((model, url))
}

/// A scan's pages read by the local model: their text, page by page, and the pages it couldn't read (still sent as
/// photos; the text says which). No model set: no text, every page a photo.
// ponytail: pages one after another and the first failure stops the rest (Ollama down, or too slow); a whole scan can
// still take MAX_PAGES × the page limit on a slow CPU. Offer it only where it's quick if measurements say so.
pub async fn read_pages(db: &crate::db::Db, pages: Vec<Vec<u8>>) -> (String, Vec<Vec<u8>>) {
    let Some((model, url)) = configured(db) else { return (String::new(), pages) };
    let Ok(http) = reqwest::Client::builder().connect_timeout(Duration::from_secs(5)).timeout(Duration::from_secs(120)).build() else { return (String::new(), pages) };
    let (mut text, mut photos, mut failed) = (String::new(), vec![], false);
    for (i, p) in pages.into_iter().enumerate() {
        let read = if failed { None } else { page(&http, &url, &model, &p).await.inspect_err(|e| eprintln!("local OCR, page {}: {e:#}", i + 1)).ok() };
        failed |= read.is_none();
        match read.filter(|t| !t.trim().is_empty()) {
            Some(t) => text.push_str(&format!("--- page {} ---\n{}\n\n", i + 1, t.trim())),
            None => {
                photos.push(p);
                text.push_str(&format!("--- page {}: not read, it's photo {} with this message ---\n\n", i + 1, photos.len()));
            }
        }
    }
    if photos.len() == text.matches("--- page").count() {
        return (String::new(), photos); // nothing read: the scan goes as photos, as without a model
    }
    (text, photos)
}

/// One page (a JPEG) through Ollama's chat API.
async fn page(http: &reqwest::Client, url: &str, model: &str, jpeg: &[u8]) -> Result<String> {
    use base64::Engine;
    let body = serde_json::json!({
        "model": model,
        "stream": false,
        "messages": [{"role": "user", "content": PROMPT, "images": [base64::engine::general_purpose::STANDARD.encode(jpeg)]}],
    });
    let v: serde_json::Value = http.post(format!("{url}/api/chat")).json(&body).send().await.context("couldn't reach Ollama")?.json().await?;
    if let Some(e) = v["error"].as_str() {
        bail!("{e}");
    }
    Ok(v["message"]["content"].as_str().unwrap_or_default().to_string())
}

#[cfg(test)]
mod tests {
    use crate::db::Db;

    /// A stand-in Ollama: pages "p1" and "p3" read, "p2" comes back empty (a photo, but not a failure: "p3" is still asked).
    #[tokio::test]
    async fn reads_what_it_can_and_keeps_the_rest_as_photos() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let mut asked = vec![];
                let mut buf = [0u8; 4096];
                // base64 of p1, p2, p3
                while !["cDE=", "cDI=", "cDM="].iter().any(|p| String::from_utf8_lossy(&asked).contains(p)) {
                    let n = tokio::io::AsyncReadExt::read(&mut sock, &mut buf).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    asked.extend(&buf[..n]);
                }
                let asked = String::from_utf8_lossy(&asked);
                let content = if asked.contains("cDE=") { "| Item | Total |\\n|---|---|\\n| Rent | 9 100 |" } else if asked.contains("cDM=") { "Signed, Sam" } else { "" };
                let body = format!(r#"{{"message":{{"role":"assistant","content":"{content}"}}}}"#);
                let resp = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                let _ = tokio::io::AsyncWriteExt::write_all(&mut sock, resp.as_bytes()).await;
            }
        });
        let p = std::env::temp_dir().join(format!("mimir-ocr-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let pages = vec![b"p1".to_vec(), b"p2".to_vec(), b"p3".to_vec()];
        assert_eq!(super::read_pages(&db, pages.clone()).await, (String::new(), pages.clone()), "off until a model is set");
        db.set_setting("ocr_model", "dots-ocr").unwrap();
        db.set_setting("ocr_url", &format!("http://{addr}/")).unwrap();
        let (text, photos) = super::read_pages(&db, pages.clone()).await;
        assert_eq!(photos, vec![b"p2".to_vec()]);
        assert_eq!(text, "--- page 1 ---\n| Item | Total |\n|---|---|\n| Rent | 9 100 |\n\n--- page 2: not read, it's photo 1 with this message ---\n\n--- page 3 ---\nSigned, Sam\n\n");
        // Ollama not running: every page a photo, as before.
        db.set_setting("ocr_url", "http://127.0.0.1:9").unwrap();
        assert_eq!(super::read_pages(&db, pages.clone()).await, (String::new(), pages));
        let _ = std::fs::remove_file(&p);
    }

    /// Seconds per page with a real model: MIMIR_TEST_OCR_MODEL=… MIMIR_TEST_PDF=scan.pdf cargo test -p mimir-core ocr_real -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn ocr_real() {
        let p = std::env::temp_dir().join(format!("mimir-ocr-real-{}.db", std::process::id()));
        let db = Db::open(&p).unwrap();
        db.set_setting("ocr_model", &std::env::var("MIMIR_TEST_OCR_MODEL").unwrap()).unwrap();
        let pages = crate::pdf_page_images(&std::fs::read(std::env::var("MIMIR_TEST_PDF").unwrap()).unwrap());
        let (n, t) = (pages.len(), std::time::Instant::now());
        let (text, photos) = super::read_pages(&db, pages).await;
        println!("{text}\n{n} pages, {} not read, {:.1} s per page", photos.len(), t.elapsed().as_secs_f64() / n.max(1) as f64);
        let _ = std::fs::remove_file(&p);
    }
}
