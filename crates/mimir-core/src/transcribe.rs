//! Speech to text for meetings and calls: whisper.cpp on this computer (private and free; the model downloads once),
//! built into the desktop app (feature local-whisper) or an installed whisper-cli; else the OpenAI-compatible key. Audio arrives as f32 samples; this turns them into 16 kHz mono WAV chunks.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// whisper.cpp's model, by name (setting `whisper_model`): base is small and quick; small/medium are better.
pub const DEFAULT_MODEL: &str = "base";
const MODELS: [&str; 6] = ["tiny", "base", "small", "medium", "large-v3-turbo", "large-v3"];

fn which(bin: &str) -> Option<PathBuf> {
    // Apps started from Finder get a bare PATH; Homebrew's folders are where whisper.cpp usually is.
    let extra = ["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from);
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default().into_iter().chain(extra).map(|d| d.join(bin)).find(|p| p.is_file())
}

/// whisper.cpp's command line, if installed (`brew install whisper-cpp`, or from github.com/ggml-org/whisper.cpp).
pub fn whisper_bin() -> Option<PathBuf> {
    ["whisper-cli", "whisper-cpp", "whisper-cli.exe"].into_iter().find_map(which)
}

pub fn model_name(db: &crate::db::Db) -> String {
    db.setting("whisper_model").ok().flatten().filter(|m| MODELS.contains(&m.as_str())).unwrap_or_else(|| DEFAULT_MODEL.into())
}

pub fn model_path(home: &Path, name: &str) -> PathBuf {
    home.join("whisper").join(format!("ggml-{name}.bin"))
}

/// Which engine transcribes here: "local" (whisper.cpp), "api" (the OpenAI-compatible key), or None.
pub fn engine() -> Option<&'static str> {
    if cfg!(feature = "local-whisper") || whisper_bin().is_some() {
        Some("local")
    } else if crate::host::api_key("openai").is_some() {
        Some("api")
    } else {
        None
    }
}

/// Download whisper.cpp's model once (from its Hugging Face repo), into Mimir's home. The small voice-activity
/// model comes along, for whisper.cpp versions that can use it.
pub async fn ensure_model(home: &Path, name: &str) -> Result<PathBuf> {
    let path = model_path(home, name);
    // A fixed revision of the repository, and each file's SHA-256 (Hugging Face lists them), so a model that changed
    // or was swapped upstream is refused rather than loaded.
    const REV: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";
    let sha = match name {
        "tiny" => "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
        "base" => "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        "small" => "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        "medium" => "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
        "large-v3-turbo" => "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
        "large-v3" => "64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2",
        other => bail!("unknown speech model {other}"),
    };
    let url = format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/{REV}/ggml-{name}.bin");
    download(&url, &path, 10_000_000, Some(sha)).await.context("couldn't download the speech model")?;
    if vad_supported()
        && let Err(e) = download(VAD_URL, &vad_path(home), 500_000, Some(VAD_SHA256)).await
    {
        eprintln!("voice activity model: {e:#}"); // transcribing works without it, just less clean on silence
    }
    Ok(path)
}

/// Silero's voice-activity model (~1 MB): whisper.cpp then transcribes only the speech, so it doesn't invent
/// words ("[music]", "Thanks for watching") over silence, and long quiet stretches cost nothing.
const VAD_URL: &str = "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v5.1.2.bin";
const VAD_SHA256: &str = "29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf";

fn vad_path(home: &Path) -> PathBuf {
    home.join("whisper").join("ggml-silero-v5.1.2.bin")
}

/// The built-in whisper.cpp has it; an installed whisper-cli needs 1.7.6+ (older ones would fail on --vad).
fn vad_supported() -> bool {
    if cfg!(feature = "local-whisper") {
        return true;
    }
    static SUPPORTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        whisper_bin().and_then(|b| std::process::Command::new(b).arg("--help").output().ok()).is_some_and(|o| String::from_utf8_lossy(&o.stdout).contains("--vad") || String::from_utf8_lossy(&o.stderr).contains("--vad"))
    })
}

/// Who's who among the other people on a call: pyannote's segmentation (6 MB) and 3D-Speaker's CAM++ voice
/// embeddings (28 MB), run by sherpa-onnx on this computer once the call ends.
const SEGMENTATION_URL: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-pyannote-segmentation-3-0/resolve/main/model.onnx";
const VOICES_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx";
const SEGMENTATION_SHA256: &str = "220ad67ca923bef2fa91f2390c786097bf305bceb5e261d4af67b38e938e1079";
const VOICES_SHA256: &str = "aa3cfc16963a10586a9393f5035d6d6b57e98d358b347f80c2a30bf4f00ceba2";

pub fn diarize_available() -> bool {
    cfg!(any(feature = "diarize", feature = "diarize-dl"))
}

fn diarize_paths(home: &Path) -> (PathBuf, PathBuf) {
    let dir = home.join("whisper");
    (dir.join("pyannote-segmentation-3.0.onnx"), dir.join("campplus-sv-zh-en-16k.onnx"))
}

pub async fn ensure_diarize_models(home: &Path) -> Result<(PathBuf, PathBuf)> {
    let (seg, voices) = diarize_paths(home);
    download(SEGMENTATION_URL, &seg, 1_000_000, Some(SEGMENTATION_SHA256)).await.context("couldn't download the speaker model")?;
    download(VOICES_URL, &voices, 10_000_000, Some(VOICES_SHA256)).await.context("couldn't download the voice model")?;
    #[cfg(feature = "diarize-dl")]
    sherpa_dl::ensure_libs(&home.join("whisper").join("sherpa")).await?;
    Ok((seg, voices))
}

/// Speaker turns in 16 kHz audio: (start s, end s, speaker), speakers counted automatically.
#[cfg(feature = "diarize")]
pub fn speaker_turns(models: &(PathBuf, PathBuf), audio: Vec<f32>) -> Result<Vec<(f64, f64, i32)>> {
    let config = sherpa_rs::diarize::DiarizeConfig { num_clusters: Some(-1), threshold: Some(0.5), ..Default::default() };
    let mut d = sherpa_rs::diarize::Diarize::new(&models.0, &models.1, config).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(d.compute(audio, None).map_err(|e| anyhow::anyhow!("{e}"))?.into_iter().map(|s| (s.start as f64, s.end as f64, s.speaker)).collect())
}

#[cfg(feature = "diarize-dl")]
pub fn speaker_turns(models: &(PathBuf, PathBuf), audio: Vec<f32>) -> Result<Vec<(f64, f64, i32)>> {
    let dir = models.0.parent().context("no model folder")?.join("sherpa");
    sherpa_dl::speaker_turns(&dir, models, &audio)
}

/// sherpa-onnx's speaker diarization through its C API, from shared libraries downloaded on first use (Linux: there's
/// no static build of sherpa-onnx to link). The structs mirror sherpa-onnx/c-api/c-api.h of v1.12.9.
#[cfg(feature = "diarize-dl")]
mod sherpa_dl {
    use anyhow::{Context, Result, bail};
    use std::ffi::{CString, c_char, c_void};
    use std::path::Path;

    const VERSION: &str = "v1.12.9";
    const LIBS: [&str; 2] = ["libonnxruntime.so", "libsherpa-onnx-c-api.so"];

    /// The release archive for this CPU and its SHA-256: it's code that gets loaded, so it must be exactly the one
    /// this build was made against.
    fn archive() -> Result<(String, &'static str)> {
        let (arch, sha) = match std::env::consts::ARCH {
            "x86_64" => ("linux-x64-shared-no-tts", "a632847c16c417bf782dec22e4fbc714e3b274d1c51cb0354c8cab411a3373ad"),
            "aarch64" => ("linux-aarch64-shared-cpu", "ebb5686bbdc3e35dc9d84ead46dd52fe5e9174c0cd51e98ec649b82165a1fa88"),
            a => bail!("telling speakers apart isn't available on {a}"),
        };
        Ok((format!("https://github.com/k2-fsa/sherpa-onnx/releases/download/{VERSION}/sherpa-onnx-{VERSION}-{arch}.tar.bz2"), sha))
    }

    /// The two libraries in `dir`, downloaded (~21 MB) and unpacked once.
    pub async fn ensure_libs(dir: &Path) -> Result<()> {
        if LIBS.iter().all(|l| dir.join(l).is_file()) {
            return Ok(());
        }
        let tarball = dir.with_extension("tar.bz2");
        let (url, sha) = archive()?;
        super::download(&url, &tarball, 5_000_000, Some(sha)).await.context("couldn't download the speaker library")?;
        let (to, from) = (dir.to_path_buf(), tarball.clone());
        let unpacked = tokio::task::spawn_blocking(move || -> Result<()> {
            let dir = to;
            std::fs::create_dir_all(&dir)?;
            let mut tar = tar::Archive::new(bzip2::read::BzDecoder::new(std::fs::File::open(&from)?));
            for entry in tar.entries()? {
                let mut entry = entry?;
                let name = entry.path()?.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if LIBS.contains(&name.as_str()) {
                    entry.unpack(dir.join(&name))?;
                }
            }
            Ok(())
        })
        .await?;
        let _ = std::fs::remove_file(&tarball);
        unpacked?;
        if !LIBS.iter().all(|l| dir.join(l).is_file()) {
            bail!("the speaker library download didn't have what's needed");
        }
        Ok(())
    }

    #[repr(C)]
    struct Pyannote {
        model: *const c_char,
    }
    #[repr(C)]
    struct Segmentation {
        pyannote: Pyannote,
        num_threads: i32,
        debug: i32,
        provider: *const c_char,
    }
    #[repr(C)]
    struct Embedding {
        model: *const c_char,
        num_threads: i32,
        debug: i32,
        provider: *const c_char,
    }
    #[repr(C)]
    struct Clustering {
        num_clusters: i32,
        threshold: f32,
    }
    #[repr(C)]
    struct Config {
        segmentation: Segmentation,
        embedding: Embedding,
        clustering: Clustering,
        min_duration_on: f32,
        min_duration_off: f32,
    }
    #[repr(C)]
    struct Segment {
        start: f32,
        end: f32,
        speaker: i32,
    }

    /// The loaded C API (kept for the life of the process).
    struct Api {
        create: unsafe extern "C" fn(*const Config) -> *const c_void,
        destroy: unsafe extern "C" fn(*const c_void),
        process: unsafe extern "C" fn(*const c_void, *const f32, i32) -> *const c_void,
        count: unsafe extern "C" fn(*const c_void) -> i32,
        sorted: unsafe extern "C" fn(*const c_void) -> *const Segment,
        free_segments: unsafe extern "C" fn(*const Segment),
        free_result: unsafe extern "C" fn(*const c_void),
    }

    fn api(dir: &Path) -> Result<&'static Api> {
        static API: std::sync::OnceLock<Api> = std::sync::OnceLock::new();
        if let Some(a) = API.get() {
            return Ok(a);
        }
        use libloading::os::unix::{Library, RTLD_GLOBAL, RTLD_NOW};
        // SAFETY: loading sherpa-onnx's own release libraries; onnxruntime first, globally, so the C API finds it.
        let (ort, lib) = unsafe {
            let ort = Library::open(Some(dir.join(LIBS[0])), RTLD_NOW | RTLD_GLOBAL).context("couldn't load onnxruntime")?;
            (ort, Library::open(Some(dir.join(LIBS[1])), RTLD_NOW).context("couldn't load sherpa-onnx")?)
        };
        // SAFETY: the symbols have the C signatures declared in c-api.h (v1.12.9, the version downloaded).
        let a = unsafe {
            Api {
                create: *lib.get(b"SherpaOnnxCreateOfflineSpeakerDiarization\0")?,
                destroy: *lib.get(b"SherpaOnnxDestroyOfflineSpeakerDiarization\0")?,
                process: *lib.get(b"SherpaOnnxOfflineSpeakerDiarizationProcess\0")?,
                count: *lib.get(b"SherpaOnnxOfflineSpeakerDiarizationResultGetNumSegments\0")?,
                sorted: *lib.get(b"SherpaOnnxOfflineSpeakerDiarizationResultSortByStartTime\0")?,
                free_segments: *lib.get(b"SherpaOnnxOfflineSpeakerDiarizationDestroySegment\0")?,
                free_result: *lib.get(b"SherpaOnnxOfflineSpeakerDiarizationDestroyResult\0")?,
            }
        };
        // The libraries stay loaded: the function pointers above point into them.
        std::mem::forget((ort, lib));
        Ok(API.get_or_init(|| a))
    }

    pub fn speaker_turns(dir: &Path, models: &(std::path::PathBuf, std::path::PathBuf), audio: &[f32]) -> Result<Vec<(f64, f64, i32)>> {
        let api = api(dir)?;
        let cstr = |p: &Path| CString::new(p.to_string_lossy().as_bytes()).context("model path");
        let (seg, voices, cpu) = (cstr(&models.0)?, cstr(&models.1)?, CString::new("cpu")?);
        let config = Config {
            segmentation: Segmentation { pyannote: Pyannote { model: seg.as_ptr() }, num_threads: 1, debug: 0, provider: cpu.as_ptr() },
            embedding: Embedding { model: voices.as_ptr(), num_threads: 2, debug: 0, provider: cpu.as_ptr() },
            // -1: work out how many speakers there are; 0.5: the distance at which two voices count as different.
            clustering: Clustering { num_clusters: -1, threshold: 0.5 },
            min_duration_on: 0.0,
            min_duration_off: 0.0,
        };
        // SAFETY: the config and its strings outlive these calls; every object created here is freed here.
        unsafe {
            let sd = (api.create)(&config);
            if sd.is_null() {
                bail!("couldn't start telling speakers apart (the models may be damaged)");
            }
            let result = (api.process)(sd, audio.as_ptr(), audio.len() as i32);
            let mut out = vec![];
            if !result.is_null() {
                let n = (api.count)(result).max(0) as usize;
                let segs = (api.sorted)(result);
                if !segs.is_null() {
                    out = std::slice::from_raw_parts(segs, n).iter().map(|s| (s.start as f64, s.end as f64, s.speaker)).collect();
                    (api.free_segments)(segs);
                }
                (api.free_result)(result);
            }
            (api.destroy)(sd);
            Ok(out)
        }
    }
}

#[cfg(not(any(feature = "diarize", feature = "diarize-dl")))]
pub fn speaker_turns(_: &(PathBuf, PathBuf), _: Vec<f32>) -> Result<Vec<(f64, f64, i32)>> {
    bail!("this build can't tell speakers apart")
}

/// Each line's speaker from the turns (by when it starts, seconds into the call), numbered 1, 2… in order of first
/// appearance. None when there's only one voice: then "Others" says it all.
pub fn assign_speakers(starts: &[f64], turns: &[(f64, f64, i32)]) -> Option<Vec<usize>> {
    let mut order: Vec<i32> = vec![];
    let picked: Vec<Option<i32>> = starts
        .iter()
        .map(|&t| {
            // The turn it falls in, else the nearest one (lines are stamped where speech starts, give or take).
            turns.iter().find(|(s, e, _)| t >= s - 0.3 && t <= *e).or_else(|| turns.iter().min_by(|a, b| (a.0 - t).abs().total_cmp(&(b.0 - t).abs()))).map(|x| x.2)
        })
        .collect();
    for sp in picked.iter().flatten() {
        if !order.contains(sp) {
            order.push(*sp);
        }
    }
    (order.len() > 1).then(|| picked.iter().map(|p| p.and_then(|sp| order.iter().position(|o| *o == sp)).map_or(0, |i| i + 1)).collect())
}

/// A file downloaded once: into a .part file, then renamed, so a cut-off download is never used. With `sha256`, a file
/// that isn't exactly the expected one is thrown away.
async fn download(url: &str, path: &Path, min_bytes: usize, sha256: Option<&str>) -> Result<()> {
    // One download at a time: a meeting fetches the model as recording starts, and the first chunk waits for that.
    static DOWNLOAD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    if path.is_file() {
        return Ok(());
    }
    let _one = DOWNLOAD.lock().await;
    if path.is_file() {
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().context("no model folder")?)?;
    // Streamed to the .part file (a large model is gigabytes); it gives up only when nothing arrives for a minute.
    let mut resp = reqwest::Client::builder().connect_timeout(Duration::from_secs(30)).build()?.get(url).send().await?.error_for_status()?;
    let tmp = path.with_extension("part");
    let mut file = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
    let mut got = 0usize;
    let mut hash = <sha2::Sha256 as sha2::Digest>::new();
    while let Some(chunk) = tokio::time::timeout(Duration::from_secs(60), resp.chunk()).await.context("the download stalled")?? {
        std::io::Write::write_all(&mut file, &chunk)?;
        sha2::Digest::update(&mut hash, &chunk);
        got += chunk.len();
    }
    std::io::Write::flush(&mut file)?;
    drop(file);
    if got < min_bytes {
        let _ = std::fs::remove_file(&tmp);
        bail!("the download looks incomplete");
    }
    if let Some(want) = sha256 {
        let have: String = sha2::Digest::finalize(hash).iter().map(|b| format!("{b:02x}")).collect();
        if have != want {
            let _ = std::fs::remove_file(&tmp);
            bail!("the download from {url} isn't the expected file (checksum mismatch), so it wasn't used");
        }
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Transcribe one WAV file (16 kHz mono). Empty text for silence.
pub async fn transcribe_wav(home: &Path, model: &str, wav: &Path) -> Result<String> {
    Ok(transcribe_segments(home, model, wav).await?.into_iter().map(|s| s.1).collect::<Vec<_>>().join(" "))
}

/// Transcribe one WAV file into (seconds from its start, sentence). whisper.cpp times each sentence.
// ponytail: the OpenAI-compatible path gives one piece at 0 (its verbose_json has segment times, if that ever matters)
pub async fn transcribe_segments(home: &Path, model: &str, wav: &Path) -> Result<Vec<(f64, String)>> {
    #[cfg(feature = "local-whisper")]
    {
        let m = ensure_model(home, model).await?;
        let vad = Some(vad_path(home)).filter(|p| p.is_file());
        let wav = wav.to_path_buf();
        return tokio::task::spawn_blocking(move || builtin::transcribe(&m, vad.as_deref(), &wav)).await?;
    }
    #[allow(unreachable_code)]
    if let Some(bin) = whisper_bin() {
        let m = ensure_model(home, model).await?;
        let vad = vad_path(home);
        let out = tokio::process::Command::new(bin)
            .arg("-m")
            .arg(&m)
            .arg("-f")
            .arg(wav)
            .args(["-np", "-l", "auto"])
            // Only the speech (timestamps stay on the original audio's clock).
            .args(if vad_supported() && vad.is_file() { vec!["--vad".into(), "-vm".into(), vad.into_os_string()] } else { vec![] })
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .context("couldn't run whisper.cpp")?;
        if !out.status.success() {
            bail!("whisper.cpp failed: {}", crate::tools::truncate(&String::from_utf8_lossy(&out.stderr), 300));
        }
        return Ok(segments(&String::from_utf8_lossy(&out.stdout)));
    }
    let bytes = std::fs::read(wav)?;
    match crate::provider::transcribe(bytes, "chunk.wav").await {
        Some(r) => r.map(|t| Some((0.0, clean(&t))).filter(|s| !s.1.is_empty()).into_iter().collect()),
        None => bail!("nothing to transcribe with: install whisper.cpp (github.com/ggml-org/whisper.cpp; on a Mac: brew install whisper-cpp) or add an OpenAI-compatible key in Settings"),
    }
}

/// Transcribe an audio or video file of any format (ffmpeg turns it into 16 kHz mono WAV first).
pub async fn transcribe_file(home: &Path, model: &str, file: &Path) -> Result<String> {
    let ffmpeg = which("ffmpeg").context("transcribing a file needs ffmpeg (brew install ffmpeg)")?;
    let wav = std::env::temp_dir().join(format!("mimir-{}.wav", crate::random_hex(6)?));
    let out = tokio::process::Command::new(ffmpeg)
        .args(["-y", "-loglevel", "error", "-i"])
        .arg(file)
        .args(["-ar", "16000", "-ac", "1", "-f", "wav"])
        .arg(&wav)
        .kill_on_drop(true)
        .output()
        .await?;
    if !out.status.success() {
        bail!("couldn't read that file: {}", crate::tools::truncate(&String::from_utf8_lossy(&out.stderr), 300));
    }
    let text = transcribe_wav(home, model, &wav).await;
    let _ = std::fs::remove_file(&wav);
    text
}

/// Free the built-in speech models. Call before the process exits: with them still loaded, ggml's Metal backend
/// aborts during exit (a crash report on every quit).
pub fn unload() {
    #[cfg(feature = "local-whisper")]
    builtin::unload();
}

/// whisper.cpp inside the app: the model stays loaded between chunks (Apple GPU on macOS).
#[cfg(feature = "local-whisper")]
mod builtin {
    use anyhow::{Context, Result, bail};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperVadContext, WhisperVadContextParams, WhisperVadParams};

    static LOADED: Mutex<Option<(PathBuf, WhisperContext)>> = Mutex::new(None);
    static VAD: Mutex<Option<(PathBuf, WhisperVadContext)>> = Mutex::new(None);

    pub fn transcribe(model: &Path, vad: Option<&Path>, wav: &Path) -> Result<Vec<(f64, String)>> {
        let audio = read_wav(&std::fs::read(wav)?)?;
        // Where people speak (seconds). whisper.cpp's own VAD only runs in whisper_full, not in the per-state call
        // whisper-rs makes, so it's done here: only speech is transcribed, each stretch at its real time.
        let t = std::time::Instant::now();
        let stretches = match vad {
            Some(v) => speech(v, &audio)?,
            None => vec![(0.0, audio.len() as f64 / 16_000.0)],
        };
        if std::env::var("MIMIR_WHISPER_DEBUG").is_ok() {
            eprintln!("vad {:?}: {stretches:?}", t.elapsed());
        }
        let mut loaded = LOADED.lock().unwrap_or_else(|e| e.into_inner());
        if loaded.as_ref().is_none_or(|(p, _)| p != model) {
            quiet();
            let ctx = WhisperContext::new_with_params(model.to_str().context("model path")?, WhisperContextParameters::default()).context("couldn't load the speech model")?;
            *loaded = Some((model.to_path_buf(), ctx));
        }
        let (_, ctx) = loaded.as_ref().context("no model")?;
        let mut state = ctx.create_state()?;
        let mut out = vec![];
        for (start, end) in stretches {
            let piece = &audio[(start * 16_000.0) as usize..((end * 16_000.0) as usize).min(audio.len())];
            if piece.len() < 4_800 {
                continue; // under 0.3 s: a click, not words
            }
            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_language(Some("auto"));
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_special(false);
            params.set_print_timestamps(false);
            params.set_no_context(true);
            state.full(params, piece)?;
            out.extend(state.as_iter().filter_map(|seg| {
                let text = super::clean(&seg.to_str_lossy().ok()?);
                (!text.is_empty()).then(|| (start + seg.start_timestamp() as f64 / 100.0, text))
            }));
        }
        Ok(out)
    }

    /// Speech stretches in seconds, padded a little; ones less than a second apart are one stretch (fewer, longer
    /// pieces read better and whisper does better with context).
    fn speech(model: &Path, audio: &[f32]) -> Result<Vec<(f64, f64)>> {
        let mut vad = VAD.lock().unwrap_or_else(|e| e.into_inner());
        if vad.as_ref().is_none_or(|(p, _)| p != model) {
            quiet();
            // One thread: the model is tiny and runs once per 32 ms of audio, so spreading each step over threads costs
            // far more than it saves (4 threads took ~80 s for 24 s of audio on Linux; 1 takes well under a second).
            let mut params = WhisperVadContextParams::default();
            params.set_n_threads(1);
            *vad = Some((model.to_path_buf(), WhisperVadContext::new(model.to_str().context("vad path")?, params)?));
        }
        let (_, ctx) = vad.as_mut().context("no vad")?;
        let len = audio.len() as f64 / 16_000.0;
        let mut merged: Vec<(f64, f64)> = vec![];
        for seg in ctx.segments_from_samples(WhisperVadParams::default(), audio)? {
            let (s, e) = (((seg.start as f64 / 100.0) - 0.2).max(0.0), ((seg.end as f64 / 100.0) + 0.2).min(len));
            match merged.last_mut() {
                Some(last) if s - last.1 < 1.0 => last.1 = e,
                _ => merged.push((s, e)),
            }
        }
        Ok(merged)
    }

    pub fn unload() {
        *LOADED.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *VAD.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// whisper.cpp's loading chatter off stderr (MIMIR_WHISPER_DEBUG keeps it).
    fn quiet() {
        if std::env::var("MIMIR_WHISPER_DEBUG").is_err() {
            whisper_rs::install_logging_hooks();
        }
    }

    /// 16-bit PCM mono WAV (ours, or ffmpeg's) → f32 samples.
    fn read_wav(b: &[u8]) -> Result<Vec<f32>> {
        if b.len() < 12 || &b[..4] != b"RIFF" || &b[8..12] != b"WAVE" {
            bail!("not a WAV file");
        }
        let mut at = 12;
        while at + 8 <= b.len() {
            let size = u32::from_le_bytes(b[at + 4..at + 8].try_into()?) as usize;
            let body = &b[at + 8..(at + 8 + size).min(b.len())];
            if &b[at..at + 4] == b"data" {
                return Ok(body.as_chunks::<2>().0.iter().map(|s| i16::from_le_bytes(*s) as f32 / 32768.0).collect());
            }
            at += 8 + size + size % 2;
        }
        bail!("no audio in the WAV file")
    }

    /// MIMIR_TEST_MODEL=… MIMIR_TEST_WAV=… cargo test -p mimir-core --features local-whisper builtin_real -- --ignored --nocapture
    #[test]
    #[ignore]
    fn builtin_real() {
        let (m, w) = (std::env::var("MIMIR_TEST_MODEL").unwrap(), std::env::var("MIMIR_TEST_WAV").unwrap());
        let vad = std::env::var("MIMIR_TEST_VAD").ok();
        for round in 0..2 {
            let t = std::time::Instant::now();
            let out = transcribe(Path::new(&m), vad.as_deref().map(Path::new), Path::new(&w)).unwrap();
            println!("round {round}: {:?} {out:?}", t.elapsed());
        }
        unload();
    }

    #[test]
    fn reads_our_wav() {
        let w = super::wav_bytes(&[0.0, 0.5, -0.5]);
        let s = read_wav(&w).unwrap();
        assert_eq!(s.len(), 3);
        assert!((s[1] - 0.5).abs() < 0.001 && (s[2] + 0.5).abs() < 0.001);
    }
}

/// whisper.cpp's "[00:00:01.240 --> 00:00:04.000]   text" lines → (1.24, "text"); noise markers dropped.
fn segments(out: &str) -> Vec<(f64, String)> {
    out.lines()
        .filter_map(|l| {
            let (stamp, text) = l.trim().strip_prefix('[')?.split_once(']')?;
            let start = stamp.split("-->").next()?.trim();
            let secs = start.split(':').try_fold(0.0, |acc, p| p.parse::<f64>().ok().map(|v| acc * 60.0 + v))?;
            Some((secs, clean(text))).filter(|s| !s.1.is_empty())
        })
        .collect()
}

/// whisper's output as one tidy paragraph; its stock "[BLANK_AUDIO]"/"(music)" markers dropped.
fn clean(text: &str) -> String {
    let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = t.trim();
    let noise = t.starts_with('[') && t.ends_with(']') || t.starts_with('(') && t.ends_with(')');
    if noise { String::new() } else { t.to_string() }
}

/// Mono samples at `from` Hz → 16 kHz (linear interpolation: plenty for speech).
pub fn resample_16k(samples: &[f32], from: u32) -> Vec<f32> {
    if from == 16_000 || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = from as f64 / 16_000.0;
    let n = (samples.len() as f64 / ratio) as usize;
    (0..n)
        .map(|i| {
            let x = i as f64 * ratio;
            let (j, f) = (x as usize, (x.fract()) as f32);
            let a = samples[j.min(samples.len() - 1)];
            let b = samples[(j + 1).min(samples.len() - 1)];
            a + (b - a) * f
        })
        .collect()
}

/// Is there anything worth transcribing? (whisper invents words on silence.)
/// Any half second loud enough to be speech: a short reply in a long, quiet chunk still counts.
pub fn has_speech(samples: &[f32]) -> bool {
    samples.as_chunks::<8_000>().0.iter().any(|w| (w.iter().map(|s| s * s).sum::<f32>() / 8_000.0).sqrt() > 0.004)
}

/// Where speech starts at or after sample `from` (16 kHz): the first 0.1 s window in the next 2 s that isn't quiet,
/// else `from`. whisper puts a sentence after silence at the silence's start, so its times get moved up to the speech;
/// no further than 2 s, so a quiet sentence (an echo, a soft voice) isn't moved onto louder speech that comes later.
pub fn speech_from(samples: &[f32], from: usize) -> usize {
    let from = from.min(samples.len());
    let end = (from + 32_000).min(samples.len());
    samples[from..end].chunks(1_600).position(|w| (w.iter().map(|s| s * s).sum::<f32>() / w.len() as f32).sqrt() > 0.01).map_or(from, |p| from + p * 1_600)
}

/// 16 kHz mono samples as a 16-bit PCM WAV file.
pub fn wav_bytes(samples: &[f32]) -> Vec<u8> {
    let data = samples.len() * 2;
    let mut w = Vec::with_capacity(44 + data);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&((36 + data) as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&16_000u32.to_le_bytes());
    w.extend_from_slice(&32_000u32.to_le_bytes()); // byte rate
    w.extend_from_slice(&2u16.to_le_bytes()); // block align
    w.extend_from_slice(&16u16.to_le_bytes()); // bits
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data as u32).to_le_bytes());
    for s in samples {
        w.extend_from_slice(&((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes());
    }
    w
}

#[cfg(test)]
mod tests {
    /// MIMIR_TEST_SEG=… MIMIR_TEST_VOICES=… MIMIR_TEST_WAV=… cargo test -p mimir-core --features diarize diarize_real -- --ignored --nocapture
    #[cfg(feature = "diarize")]
    #[test]
    #[ignore]
    fn diarize_real() {
        let v = |k: &str| std::env::var(k).unwrap();
        let b = std::fs::read(v("MIMIR_TEST_WAV")).unwrap();
        let audio: Vec<f32> = b[44..].as_chunks::<2>().0.iter().map(|s| i16::from_le_bytes(*s) as f32 / 32768.0).collect();
        let t = std::time::Instant::now();
        let turns = super::speaker_turns(&(v("MIMIR_TEST_SEG").into(), v("MIMIR_TEST_VOICES").into()), audio).unwrap();
        println!("{:?} turns {turns:?}", t.elapsed());
        let starts: Vec<f64> = turns.iter().map(|t| t.0 + 0.2).collect();
        println!("labels {:?}", super::assign_speakers(&starts, &turns));
    }

    #[test]
    fn speakers_numbered_by_first_turn() {
        let turns = [(0.0, 3.5, 7), (3.6, 6.7, 2), (6.8, 9.0, 7)];
        assert_eq!(super::assign_speakers(&[0.2, 4.0, 7.0, 3.55], &turns), Some(vec![1, 2, 1, 2]));
        assert_eq!(super::assign_speakers(&[0.2, 7.0], &[(0.0, 9.0, 3)]), None, "one voice: leave it as Others");
    }

    #[test]
    fn audio_helpers() {
        let tone: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.05).sin() * 0.3).collect();
        let r = super::resample_16k(&tone, 48_000);
        assert_eq!(r.len(), 16_000, "one second either way");
        assert!(super::has_speech(&r) && !super::has_speech(&vec![0.0; 16_000]) && !super::has_speech(&r[..100]));
        // One second of quiet speech in a 30 s chunk still counts.
        let mut long = vec![0.0; 16_000 * 30];
        long[16_000 * 10..16_000 * 11].iter_mut().enumerate().for_each(|(i, s)| *s = (i as f32 * 0.05).sin() * 0.03);
        assert!(super::has_speech(&long));
        let w = super::wav_bytes(&r);
        assert_eq!((&w[..4], &w[8..12], w.len()), (&b"RIFF"[..], &b"WAVE"[..], 44 + 32_000));
        let mut late = vec![0.0; 24_000];
        late.extend(&r);
        assert_eq!((super::speech_from(&late, 0), super::speech_from(&late, 30_000), super::speech_from(&late, 99_999)), (24_000, 30_000, late.len()));
        let mut far = vec![0.0; 80_000];
        far.extend(&r);
        assert_eq!(super::speech_from(&far, 0), 0, "speech 5 s later is someone else's; stay put");
        assert_eq!(super::clean("  [BLANK_AUDIO] "), "");
        let out = "[00:00:00.000 --> 00:00:03.100]   Morning, everyone.\n[00:00:03.100 --> 00:00:05.000]   [BLANK_AUDIO]\n[00:01:02.500 --> 00:01:04.000]  Sam, the deck?\n";
        assert_eq!(super::segments(out), vec![(0.0, "Morning, everyone.".to_string()), (62.5, "Sam, the deck?".to_string())]);
        assert_eq!(super::clean(" Hello\n  there. "), "Hello there.");
    }
}
