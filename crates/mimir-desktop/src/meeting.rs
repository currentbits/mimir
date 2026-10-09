//! Meeting and call transcription on this computer. Your microphone is "you"; the computer's own sound is "others"
//! (the people on Teams, Zoom, Meet, WhatsApp or a phone call through the computer), so the transcript knows who spoke
//! without a speaker model. Every ~30 s the new audio is transcribed (whisper.cpp here, or your OpenAI-compatible key).
//! Computer sound: a Core Audio process tap on macOS 14.4+ (ScreenCaptureKit before), WASAPI loopback on Windows, the PulseAudio/PipeWire monitor on Linux.

use mimir_core::transcribe;
use serde::Serialize;
use mimir_core::db::Db;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const CHUNK_SECS: u64 = 30;

#[cfg(target_os = "macos")]
const PERMISSION: &str = "Allow Mimir the microphone (and System Audio Recording for the call's sound) in System Settings → Privacy & Security.";
#[cfg(target_os = "macos")]
const MIC_SILENT: &str = "Allow Mimir under System Settings → Privacy & Security → Microphone (or pick another microphone below), then start again.";
#[cfg(not(target_os = "macos"))]
const MIC_SILENT: &str = "Check it isn't muted, or pick another microphone below.";

#[cfg(target_os = "macos")]
const SOUND_PERMISSION: &str = "Allow Mimir under System Settings → Privacy & Security → Screen & System Audio Recording (System Audio Recording Only on macOS 14.4+), then start again.";
#[cfg(not(target_os = "macos"))]
const SOUND_PERMISSION: &str = "";

#[cfg(windows)]
const PERMISSION: &str = "Allow desktop apps the microphone in Settings → Privacy & security → Microphone.";
#[cfg(all(unix, not(target_os = "macos")))]
const PERMISSION: &str = "Check that a microphone is connected and not muted (pavucontrol shows the inputs).";

#[derive(Clone, Serialize)]
pub struct Line {
    pub ts: i64,
    /// "you", "others", or once speakers are told apart "others:1", "others:2"…
    pub who: String,
    pub text: String,
}

#[derive(Default, Clone, Serialize)]
pub struct Status {
    pub recording: bool,
    pub started_at: i64,
    pub lines: Vec<Line>,
    /// "microphone", "computer sound": what's being captured.
    pub sources: Vec<&'static str>,
    /// Chunks waiting for transcription.
    pub pending: usize,
    pub error: Option<String>,
    /// Recording, but not everything: why the computer's sound (the others) isn't being captured.
    pub warning: Option<String>,
    /// Getting ready (the speech model downloading, the first time).
    pub preparing: Option<String>,
    /// The microphone has sent nothing but exact zeros so far (a blocked mic), and what to do about it.
    pub mic_warning: Option<String>,
    /// The saved meeting these lines go to (in this computer's Mimir).
    pub meeting_id: Option<i64>,
}

/// Samples from one source, at its own rate (resampled to 16 kHz when transcribed).
#[derive(Default)]
struct Buf {
    rate: u32,
    samples: Vec<f32>,
    /// Any sound at all since recording started.
    heard: bool,
    /// When this source's first sample was recorded, and samples kept since: to fill gaps with silence.
    t0: Option<std::time::Instant>,
    total: u64,
}

#[derive(Default)]
pub struct Meeting {
    status: Arc<Mutex<Status>>,
    mic: Arc<Mutex<Buf>>,
    sys: Arc<Mutex<Buf>>,
    stop: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    chunker: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    /// Telling the last meeting's speakers apart, after it ended.
    diarizing: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    /// One start or stop at a time: a start during a stop (tray and app) would otherwise wipe the buffers, and the
    /// stop would then delete the new meeting.
    op: tokio::sync::Mutex<()>,
}

fn now() -> i64 {
    mimir_core::db::now()
}

/// Now, to the millisecond: chunk times are worked out from it, so lines don't drift by up to a second.
fn now_f() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

impl Meeting {
    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    /// Start recording and transcribing.
    pub fn start(&self, db: Db, model: String) -> Result<Status, String> {
        // Kept for the Meetings view too: a start from the tray has no one else to tell.
        self.try_start(db, model).inspect_err(|e| self.status.lock().unwrap().error = Some(e.clone()))
    }

    fn try_start(&self, db: Db, model: String) -> Result<Status, String> {
        let Ok(_op) = self.op.try_lock() else {
            return Err("still finishing the last meeting; try again in a moment".into());
        };
        if self.status.lock().unwrap().recording {
            return Ok(self.status());
        }
        if transcribe::engine().is_none() {
            return Err("to transcribe, install whisper.cpp (github.com/ggml-org/whisper.cpp; on a Mac: brew install whisper-cpp) or add an OpenAI-compatible key in Settings".into());
        }
        *self.mic.lock().unwrap() = Buf::default();
        *self.sys.lock().unwrap() = Buf::default();
        let started_at = now();
        // Named after the app the call is in ("Teams call"), when the system says which one has the microphone.
        let title = mic_in_use().filter(|a| !a.is_empty()).map(|a| if a.starts_with("Phone") { "Phone call".to_string() } else { format!("{a} call") }).unwrap_or_default();
        let meeting_id = db.meeting_save(&title, started_at, started_at, &[]).map_err(|e| e.to_string())?;
        *self.status.lock().unwrap() = Status { recording: true, started_at, meeting_id: Some(meeting_id), ..Default::default() };

        // The audio streams aren't Send (CoreAudio), so they live on their own thread until stop.
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(Vec<&'static str>, Option<String>), String>>();
        let (mic, sys) = (self.mic.clone(), self.sys.clone());
        let mic_name = db.setting("meeting_mic").ok().flatten();
        std::thread::spawn(move || {
            let mut sources = vec![];
            let mic_stream = open_mic(mic, mic_name.as_deref());
            if mic_stream.is_ok() {
                sources.push("microphone");
            }
            let sys_stream = open_system_sound(sys);
            if sys_stream.is_ok() {
                sources.push("computer sound");
            }
            let _ = ready_tx.send(if sources.is_empty() {
                Err(mic_stream.as_ref().err().cloned().unwrap_or_else(|| "no audio source".into()))
            } else {
                Ok((sources, sys_stream.as_ref().err().map(|e| {
                    let why = if e.contains("TCC") { "not allowed yet".to_string() } else { e.clone() };
                    format!("The computer's sound (the others on the call) isn't being recorded: {why}. {SOUND_PERMISSION}")
                })))
            });
            let _ = rx.recv();
            drop(sys_stream);
            drop(mic_stream);
        });
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok((sources, warning))) => {
                let mut st = self.status.lock().unwrap();
                (st.sources, st.warning) = (sources, warning);
            }
            Ok(Err(e)) => {
                self.status.lock().unwrap().recording = false;
                let _ = db.meeting_delete(meeting_id);
                return Err(format!("couldn't record: {e}. {PERMISSION}"));
            }
            Err(_) => {
                self.status.lock().unwrap().recording = false;
                let _ = db.meeting_delete(meeting_id);
                return Err("the audio devices didn't respond".into());
            }
        }
        *self.stop.lock().unwrap() = Some(tx);

        // Every chunk: whatever was said since the last one goes to the transcriber.
        let (status, mic, sys) = (self.status.clone(), self.mic.clone(), self.sys.clone());
        // whisper.cpp's model, the first time: fetched now, so it's there by the first chunk (~30 s in).
        if transcribe::engine() == Some("local") && !transcribe::model_path(&db.dir(), &model).is_file() {
            let (status, home, model) = (self.status.clone(), db.dir(), model.clone());
            status.lock().unwrap().preparing = Some(format!("Downloading the speech model ({model}, once)…"));
            tauri::async_runtime::spawn(async move {
                let done = transcribe::ensure_model(&home, &model).await;
                let mut st = status.lock().unwrap();
                st.preparing = None;
                if let Err(e) = done {
                    st.error = Some(format!("{e:#}"));
                }
            });
        }
        // The calendar's meeting on now, if one is linked: its name (better than "Teams call") and who's invited.
        {
            let db = db.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(ev) = mimir_core::calendar::now().await {
                    let _ = db.meeting_update(meeting_id, Some(&ev.title), None);
                    let _ = db.meeting_attendees(meeting_id, &ev.attendees);
                }
            });
        }
        // Audio a crashed meeting left behind is no use now; the speaker models come down in the background.
        if self.diarizing.lock().unwrap().as_ref().is_none_or(|t| t.inner().is_finished()) {
            let _ = std::fs::remove_dir_all(db.dir().join("meeting-audio"));
        }
        if transcribe::diarize_available() {
            let home = db.dir();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = transcribe::ensure_diarize_models(&home).await {
                    eprintln!("speaker models: {e:#}");
                }
            });
        }
        let task = tauri::async_runtime::spawn(async move {
            let home = db.dir();
            // Wakes each second so stop never waits a whole chunk; stop does the last flush itself. A chunk ends at a
            // pause (both sides quiet), so lines show up seconds after they're said; at the latest after CHUNK_SECS.
            // The shortest chunk is twice what the last one took to transcribe, so a slow computer never falls behind.
            let (mut waited, mut shortest) = (0u64, 6u64);
            while status.lock().unwrap().recording {
                tokio::time::sleep(Duration::from_secs(1)).await;
                waited += 1;
                // A real microphone always has some noise; only zeros means a blocked one (macOS hands apps silence,
                // not an error, until they're allowed). Noise-suppressing mics go to zero between words, so this
                // only counts until the first sound, and clears then.
                let silent = {
                    let st = status.lock().unwrap();
                    st.sources.contains(&"microphone") && now() - st.started_at >= 5
                } && !mic.lock().unwrap().heard;
                status.lock().unwrap().mic_warning = silent.then(|| format!("Your microphone is sending pure silence, so you won't be transcribed. {MIC_SILENT}"));
                let pause = waited >= shortest && quiet_tail(&mic) && quiet_tail(&sys);
                if (pause || waited >= CHUNK_SECS) && status.lock().unwrap().recording {
                    waited = 0;
                    let took = std::time::Instant::now();
                    if std::env::var("MIMIR_WHISPER_DEBUG").is_ok() {
                        eprintln!("chunk at {}s ({})", now() - status.lock().unwrap().started_at, if pause { "pause" } else { "longest" });
                    }
                    flush(&db, &home, &model, &status, &mic, &sys).await;
                    if std::env::var("MIMIR_WHISPER_DEBUG").is_ok() {
                        eprintln!("chunk took {:?}", took.elapsed());
                    }
                    shortest = (took.elapsed().as_secs() * 2).clamp(6, CHUNK_SECS);
                }
            }
        });
        *self.chunker.lock().unwrap() = Some(task);
        Ok(self.status())
    }

    /// Wait until the speakers of the last meeting are told apart (the notes bot reads the labels).
    pub async fn speakers_done(&self) {
        let task = self.diarizing.lock().unwrap().take();
        if let Some(t) = task {
            let _ = t.await;
        }
    }

    /// Stop recording; transcribes what's left. Returns the full transcript (already saved; an empty one is dropped),
    /// or None if it wasn't recording (a second Stop).
    pub async fn stop(&self, db: &Db, model: String) -> Option<Status> {
        let _op = self.op.lock().await;
        if !self.status.lock().unwrap().recording {
            return None;
        }
        if let Some(tx) = self.stop.lock().unwrap().take() {
            let _ = tx.send(());
        }
        self.status.lock().unwrap().recording = false;
        // Let a chunk that's mid-transcription finish (aborting it would lose those lines).
        let task = self.chunker.lock().unwrap().take();
        if let Some(t) = task {
            let _ = t.await;
        }
        flush(db, &db.dir(), &model, &self.status, &self.mic, &self.sys).await;
        let id = {
            let mut st = self.status.lock().unwrap();
            if let Some(id) = st.meeting_id {
                // Nothing said, and no notes typed either: nothing worth keeping.
                let noted = db.meetings().ok().and_then(|v| v.into_iter().find(|m| m["id"] == id)).is_some_and(|m| !m["notes"].as_str().unwrap_or("").trim().is_empty());
                if st.lines.is_empty() && !noted {
                    let _ = db.meeting_delete(id);
                    let _ = std::fs::remove_file(tape_path(&db.dir(), id));
                    st.meeting_id = None;
                } else {
                    let _ = db.meeting_update(id, None, Some(now()));
                }
            }
            st.meeting_id
        };
        // Telling speakers apart takes a while for a long call (~5 min an hour), so it runs after Stop returns.
        // The audio is read now and its file deleted at once: nothing (a new meeting's clean-up) can pull it away, and
        // the others' voices don't sit on disk. Over 3 hours is left out (it would need gigabytes of memory).
        if let Some(id) = id
            && transcribe::diarize_available()
        {
            let tape = tape_path(&db.dir(), id);
            let audio = std::fs::metadata(&tape).ok().filter(|m| m.len() <= 2 * 16_000 * 3 * 3600).and_then(|_| std::fs::read(&tape).ok());
            let _ = std::fs::remove_file(&tape);
            if let Some(audio) = audio {
                let (db, status) = (db.clone(), self.status.clone());
                *self.diarizing.lock().unwrap() = Some(tauri::async_runtime::spawn(speakers(db, status, id, audio)));
            }
        }
        Some(self.status())
    }
}

/// Transcribe both sources' new audio and add the lines (yours first when they overlap in a chunk).
async fn flush(db: &Db, home: &std::path::Path, model: &str, status: &Arc<Mutex<Status>>, mic: &Arc<Mutex<Buf>>, sys: &Arc<Mutex<Buf>>) {
    let started_at = status.lock().unwrap().started_at;
    // The others first: on speakers, the microphone hears them too, and "you" lines that only repeat them are dropped.
    for (who, buf) in [("others", sys), ("you", mic)] {
        // When this chunk's first sample was recorded (the source's start, plus what earlier chunks took).
        let (rate, samples, first) = {
            let mut b = buf.lock().unwrap();
            let first = b.t0.map(|t0| t0 + Duration::from_secs_f64((b.total - b.samples.len() as u64) as f64 / b.rate.max(1) as f64));
            (b.rate, std::mem::take(&mut b.samples), first)
        };
        let audio = transcribe::resample_16k(&samples, rate.max(1));
        if !transcribe::has_speech(&audio) {
            continue;
        }
        // Each sentence is stamped from the chunk's start, so lines interleave in the order people spoke.
        // On the wall clock (not "now minus the length": a source can stop sending, e.g. the computer's sound
        // after the last thing played, so a buffer needn't end now).
        let chunk_start = first.map_or(now_f() - audio.len() as f64 / 16_000.0, |f| now_f() - f.elapsed().as_secs_f64());
        // The others' audio is kept (on the call's clock) until the call ends, to tell the speakers apart then.
        let meeting_id = status.lock().unwrap().meeting_id;
        if who == "others"
            && transcribe::diarize_available()
            && let Some(id) = meeting_id
            && let Err(e) = tape_append(&tape_path(home, id), chunk_start - started_at as f64, &audio)
        {
            eprintln!("meeting audio: {e}");
        }
        status.lock().unwrap().pending += 1;
        // In the data folder (owner-only), not the shared temp folder: it's someone's voice.
        let wav = home.join("meeting-audio").join(format!("chunk-{who}-{}.wav", mimir_core::random_hex(6).unwrap_or_default()));
        let _ = std::fs::create_dir_all(wav.parent().unwrap_or(home));
        let text = match std::fs::write(&wav, transcribe::wav_bytes(&audio)) {
            Ok(()) => transcribe::transcribe_segments(home, model, &wav).await,
            Err(e) => Err(e.into()),
        };
        let _ = std::fs::remove_file(&wav);
        // A failed chunk (a network blip, the model still downloading) goes back for the next try, up to 5 minutes'
        // worth, instead of being lost.
        if text.is_err() {
            let mut b = buf.lock().unwrap();
            if b.samples.len() + samples.len() < b.rate as usize * 300 {
                let newer = std::mem::replace(&mut b.samples, samples);
                b.samples.extend(newer);
            }
        }
        let mut st = status.lock().unwrap();
        st.pending -= 1;
        match text {
            Ok(segments) => {
                for (offset, t) in segments {
                    let start = transcribe::speech_from(&audio, (offset * 16_000.0) as usize) as f64 / 16_000.0;
                    let ts = ((chunk_start + start).round() as i64).max(started_at);
                    if who == "you" {
                        // What the others said while this line was spoken: from 3 s before it to 3 s after it ends
                        // (speech runs ~2.5 words a second; an echo can run several of their lines together).
                        let until = ts + (t.split_whitespace().count() as f64 / 2.5) as i64 + 3;
                        let nearby: Vec<&str> = st.lines.iter().filter(|l| l.who != "you" && l.ts >= ts - 3 && l.ts <= until).map(|l| l.text.as_str()).collect();
                        if is_echo(&t, &nearby) {
                            continue;
                        }
                    }
                    if let Some(id) = st.meeting_id
                        && let Err(e) = db.meeting_line(id, ts, who, &t)
                    {
                        st.error = Some(format!("couldn't save the transcript: {e:#}"));
                    }
                    st.lines.push(Line { ts, who: who.to_string(), text: t });
                }
                st.lines.sort_by_key(|l| l.ts); // in the order people spoke
            }
            Err(e) => st.error = Some(format!("{e:#}")),
        }
    }
}

/// Telling the other speakers apart once the call has ended: diarize the kept audio, relabel the others' lines
/// "others:1", "others:2"… (left as "Others" if there's only one voice), then delete the audio.
// ponytail: sherpa-rs runs the models on one thread (~8% of the call's length); its sys API takes a thread count
async fn speakers(db: Db, status: Arc<Mutex<Status>>, id: i64, bytes: Vec<u8>) {
    let (home, started_at) = (db.dir(), status.lock().unwrap().started_at);
    let result: anyhow::Result<()> = async {
        let others = db.meeting_others(id)?;
        if others.len() < 2 {
            return Ok(());
        }
        status.lock().unwrap().preparing = Some("Telling the speakers apart…".into());
        let models = transcribe::ensure_diarize_models(&home).await?;
        let audio: Vec<f32> = bytes.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b) as f32 / 32768.0).collect();
        let turns = tokio::task::spawn_blocking(move || transcribe::speaker_turns(&models, audio)).await??;
        let starts: Vec<f64> = others.iter().map(|(_, ts)| (ts - started_at) as f64).collect();
        if let Some(speakers) = transcribe::assign_speakers(&starts, &turns) {
            let labels: Vec<(i64, String)> = others.iter().zip(speakers).filter(|(_, n)| *n > 0).map(|((line, _), n)| (*line, format!("others:{n}"))).collect();
            db.meeting_relabel(&labels)?;
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        eprintln!("telling speakers apart: {e:#}"); // the transcript stays, just with "Others"
    }
    let mut st = status.lock().unwrap();
    st.preparing = None;
    // The finished meeting's lines with their new labels, for whoever shows the status next.
    if st.meeting_id == Some(id) && !st.recording {
        let lines = db.meeting_lines(id).unwrap_or_default();
        for (l, (_, who, _)) in st.lines.iter_mut().zip(lines.iter()) {
            l.who = who.clone();
        }
    }
}

/// The last 0.7 s of a source is quiet (or there's nothing): a pause to end a chunk at.
fn quiet_tail(buf: &Arc<Mutex<Buf>>) -> bool {
    let b = buf.lock().unwrap();
    let n = (b.rate as usize * 7 / 10).max(1);
    b.samples.len() < n || (b.samples[b.samples.len() - n..].iter().map(|s| s * s).sum::<f32>() / n as f32).sqrt() < 0.01
}

/// A line from the microphone that's the others' words coming out of the speakers: 80% of its word pairs, in order,
/// are in what the others said just then. Pairs, not single words, so a reply that reuses the question's words
/// ("Do you think we should ship it?" / "Yes, I think we should ship it") isn't mistaken for an echo; and 3+ words,
/// so "Yes" or "OK" never is.
fn is_echo(mine: &str, theirs: &[&str]) -> bool {
    let words = |s: &str| s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect::<Vec<_>>();
    let pairs = |w: &[String]| w.windows(2).map(|p| (p[0].clone(), p[1].clone())).collect::<Vec<_>>();
    let mine = words(mine);
    let theirs: std::collections::HashSet<(String, String)> = theirs.iter().flat_map(|t| pairs(&words(t))).collect();
    let mine = pairs(&mine);
    mine.len() >= 2 && mine.iter().filter(|p| theirs.contains(*p)).count() * 10 >= mine.len() * 8
}

/// Where a meeting's other-side audio waits until the call ends (deleted after telling speakers apart).
fn tape_path(home: &std::path::Path, id: i64) -> std::path::PathBuf {
    home.join("meeting-audio").join(format!("{id}.pcm"))
}

/// Append 16 kHz audio at `at` seconds into the call (16-bit PCM): silence fills a gap before it, and an overlap
/// with what's already there is skipped, so the file stays on the call's clock.
fn tape_append(path: &std::path::Path, at: f64, audio: &[f32]) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(path.parent().unwrap_or(path))?;
    let have = std::fs::metadata(path).map_or(0, |m| m.len() as usize / 2);
    let start = (at.max(0.0) * 16_000.0) as usize;
    let mut out = Vec::with_capacity(audio.len() * 2);
    out.extend(std::iter::repeat_n(0u8, start.saturating_sub(have) * 2));
    for s in &audio[have.saturating_sub(start).min(audio.len())..] {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes());
    }
    std::fs::OpenOptions::new().create(true).append(true).open(path)?.write_all(&out)
}

/// Mono f32 from any channel count, kept in step with the clock: the computer's sound sends nothing while nothing
/// plays (PulseAudio suspends an idle output, WASAPI loopback is silent), so a gap is filled with silence. Lines are
/// timed back from "now" by the buffer's length, which is only right when the buffer covers all the time that passed.
fn push_mono(buf: &Arc<Mutex<Buf>>, rate: u32, data: &[f32], channels: usize) {
    let mut b = buf.lock().unwrap();
    b.rate = rate;
    let ch = channels.max(1);
    let frames = (data.len() / ch) as u64;
    let now = std::time::Instant::now();
    match b.t0 {
        None => b.t0 = now.checked_sub(Duration::from_secs_f64(frames as f64 / rate.max(1) as f64)).or(Some(now)),
        Some(t0) => {
            let due = (now.duration_since(t0).as_secs_f64() * rate as f64) as u64;
            let have = b.total + frames;
            if due > have + rate as u64 / 2 {
                // A long gap (the computer slept) isn't kept as hours of silence: up to one chunk of it, and the
                // clock moves on for the rest.
                let gap = due - have;
                let kept = gap.min(rate as u64 * CHUNK_SECS);
                b.samples.extend(std::iter::repeat_n(0.0, kept as usize));
                b.total += kept;
                if gap > kept {
                    b.t0 = Some(t0 + Duration::from_secs_f64((gap - kept) as f64 / rate.max(1) as f64));
                }
            }
        }
    }
    b.total += frames;
    b.heard |= data.iter().any(|s| *s != 0.0);
    b.samples.extend(data.chunks(ch).map(|f| f.iter().sum::<f32>() / ch as f32));
}

/// The microphones, by name (to pick a headset that isn't the system's default).
pub fn microphones() -> Vec<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    cpal::default_host().input_devices().map(|d| d.filter_map(|d| d.description().ok().map(|n| n.to_string())).collect()).unwrap_or_default()
}

/// The chosen microphone (setting meeting_mic), else the system default (also when the chosen one is unplugged).
fn open_mic(buf: Arc<Mutex<Buf>>, name: Option<&str>) -> Result<cpal::Stream, String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let chosen = name.and_then(|n| host.input_devices().ok()?.find(|d| d.description().is_ok_and(|x| x.to_string() == n)));
    let device = chosen.or_else(|| host.default_input_device()).ok_or("no microphone")?;
    let config = device.default_input_config().map_err(|e| e.to_string())?;
    open_input(&device, config, buf)
}

/// Record from a device (an input, or on Windows an output: WASAPI then records what it plays).
fn open_input(device: &cpal::Device, config: cpal::SupportedStreamConfig, buf: Arc<Mutex<Buf>>) -> Result<cpal::Stream, String> {
    use cpal::traits::{DeviceTrait, StreamTrait};
    let (rate, ch) = (config.sample_rate(), config.channels() as usize);
    let err = |e| eprintln!("audio: {e}");
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(config.into(), move |d: &[f32], _: &_| push_mono(&buf, rate, d, ch), err, None),
        cpal::SampleFormat::I16 => device.build_input_stream(
            config.into(),
            move |d: &[i16], _: &_| push_mono(&buf, rate, &d.iter().map(|s| *s as f32 / i16::MAX as f32).collect::<Vec<_>>(), ch),
            err,
            None,
        ),
        f => return Err(format!("unsupported audio format {f:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

/// The computer's sound (everyone else on the call), Mimir's own sounds excluded.
/// Keeps the computer-sound capture running; dropping it stops it.
type Capture = Box<dyn std::any::Any>;

#[cfg(target_os = "macos")]
struct Sck(screencapturekit::prelude::SCStream);
#[cfg(target_os = "macos")]
impl Drop for Sck {
    fn drop(&mut self) {
        let _ = self.0.stop_capture();
    }
}

/// macOS 14.4+: a Core Audio process tap, which needs only the audio-recording permission; older Macs (or a tap that
/// fails) use ScreenCaptureKit, which needs Screen & System Audio Recording.
#[cfg(target_os = "macos")]
fn open_system_sound(buf: Arc<Mutex<Buf>>) -> Result<Capture, String> {
    if macos_at_least(14, 4) {
        match open_tap(buf.clone()) {
            Ok(c) => return Ok(c),
            Err(e) => eprintln!("audio tap: {e}; trying ScreenCaptureKit"),
        }
    }
    open_screen_capture(buf)
}

#[cfg(target_os = "macos")]
fn macos_at_least(major: u32, minor: u32) -> bool {
    let mut v = [0u8; 32];
    let mut len = v.len();
    // SAFETY: sysctl writes at most `len` bytes into `v` and sets `len` to what it wrote.
    if unsafe { libc::sysctlbyname(c"kern.osproductversion".as_ptr(), v.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) } != 0 {
        return false;
    }
    let v = String::from_utf8_lossy(&v[..len]);
    let mut parts = v.trim_end_matches('\0').split('.').map(|n| n.parse::<u32>().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0)) >= (major, minor)
}

/// A process tap of everything playing except Mimir, read through a private aggregate device (as in
/// github.com/insidegui/AudioCap). Each part is undone, in reverse, when dropped.
#[cfg(target_os = "macos")]
struct Tap {
    tap: u32,
    device: u32,
    proc_id: objc2_core_audio::AudioDeviceIOProcID,
    buf: *mut (Arc<Mutex<Buf>>, u32),
}

#[cfg(target_os = "macos")]
impl Drop for Tap {
    fn drop(&mut self) {
        use objc2_core_audio::*;
        // SAFETY: the ids came from Core Audio and are destroyed once; the IOProc is stopped before its data is freed.
        unsafe {
            if self.proc_id.is_some() {
                AudioDeviceStop(self.device, self.proc_id);
                AudioDeviceDestroyIOProcID(self.device, self.proc_id);
            }
            if self.device != 0 {
                AudioHardwareDestroyAggregateDevice(self.device);
            }
            if self.tap != 0 {
                AudioHardwareDestroyProcessTap(self.tap);
            }
            drop(Box::from_raw(self.buf));
        }
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C-unwind" fn tap_audio(
    _: u32,
    _: std::ptr::NonNull<objc2_core_audio_types::AudioTimeStamp>,
    input: std::ptr::NonNull<objc2_core_audio_types::AudioBufferList>,
    _: std::ptr::NonNull<objc2_core_audio_types::AudioTimeStamp>,
    _: std::ptr::NonNull<objc2_core_audio_types::AudioBufferList>,
    _: std::ptr::NonNull<objc2_core_audio_types::AudioTimeStamp>,
    data: *mut std::ffi::c_void,
) -> i32 {
    // SAFETY: `data` is the Tap's (buffer, rate), alive until the IOProc is stopped; Core Audio hands IOProcs f32
    // buffers, `mNumberBuffers` of them.
    unsafe {
        let (buf, rate) = &*(data as *const (Arc<Mutex<Buf>>, u32));
        let list = input.as_ref();
        let bufs = std::slice::from_raw_parts(list.mBuffers.as_ptr(), list.mNumberBuffers as usize);
        let chans: Vec<(&[f32], usize)> = bufs
            .iter()
            .filter(|b| !b.mData.is_null())
            .map(|b| (std::slice::from_raw_parts(b.mData as *const f32, b.mDataByteSize as usize / 4), b.mNumberChannels as usize))
            .collect();
        match chans.as_slice() {
            [] => {}
            [(d, ch)] => push_mono(buf, *rate, d, *ch),
            // Non-interleaved: one buffer per channel; average them.
            many => {
                let n = many.iter().map(|c| c.0.len()).min().unwrap_or(0);
                let mono: Vec<f32> = (0..n).map(|i| many.iter().map(|c| c.0[i]).sum::<f32>() / many.len() as f32).collect();
                push_mono(buf, *rate, &mono, 1);
            }
        }
    }
    0
}

#[cfg(target_os = "macos")]
fn open_tap(buf: Arc<Mutex<Buf>>) -> Result<Capture, String> {
    use objc2::{AnyThread, rc::Retained, runtime::AnyObject};
    use objc2_core_audio::*;
    use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString, NSUUID};
    use std::ptr::NonNull;
    // A tap the user turned down records only silence, so ask first (Apple's TCC framework, as AudioCap does):
    // 0 allowed, 1 denied, 2 not asked yet (the system asks when recording starts).
    // SAFETY: dlopen/dlsym with NUL-terminated names; TCCAccessPreflight(CFStringRef service, CFDictionaryRef options).
    unsafe {
        let tcc = libc::dlopen(c"/System/Library/PrivateFrameworks/TCC.framework/Versions/A/TCC".as_ptr(), libc::RTLD_NOW);
        let f = if tcc.is_null() { std::ptr::null_mut() } else { libc::dlsym(tcc, c"TCCAccessPreflight".as_ptr()) };
        if !f.is_null() {
            let preflight: unsafe extern "C" fn(*const AnyObject, *const AnyObject) -> i32 = std::mem::transmute(f);
            if preflight(Retained::as_ptr(&NSString::from_str("kTCCServiceAudioCapture")).cast(), std::ptr::null()) == 1 {
                return Err("audio recording not allowed (TCC)".into());
            }
        }
    }

    let address = |selector| AudioObjectPropertyAddress { mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain };
    // A property of `object` into `out`, optionally asked about `qualifier`.
    fn get<T>(object: u32, address: AudioObjectPropertyAddress, qualifier: Option<&i32>, out: &mut T) -> Result<(), String> {
        let mut size = std::mem::size_of::<T>() as u32;
        let (qsize, q) = qualifier.map_or((0, std::ptr::null()), |q| (4, q as *const i32 as *const std::ffi::c_void));
        // SAFETY: `out` is a writable T of `size` bytes and `q` is `qsize` bytes; Core Audio writes at most `size`.
        let status = unsafe { AudioObjectGetPropertyData(object, NonNull::from(&address), qsize, q, NonNull::from(&mut size), NonNull::from(out).cast()) };
        if status == 0 { Ok(()) } else { Err(format!("Core Audio property {:#x}: {status}", address.mSelector)) }
    }
    let system = kAudioObjectSystemObject as u32;

    // Everything but Mimir (its process object exists once it has opened the microphone; otherwise nothing to leave out).
    let mut me = 0u32;
    let _ = get(system, address(kAudioHardwarePropertyTranslatePIDToProcessObject), Some(&(std::process::id() as i32)), &mut me);
    let exclude: Vec<Retained<NSNumber>> = (me != 0).then(|| NSNumber::new_u32(me)).into_iter().collect();
    // SAFETY: a CATapDescription made with its documented initialiser; the class exists on 14.2+, checked by the caller.
    let desc = unsafe { CATapDescription::initMonoGlobalTapButExcludeProcesses(CATapDescription::alloc(), &NSArray::from_retained_slice(&exclude)) };
    unsafe {
        desc.setName(&NSString::from_str("Mimir meeting"));
        desc.setPrivate(true);
    }
    let mut capture = Tap { tap: 0, device: 0, proc_id: None, buf: Box::into_raw(Box::new((buf, 0u32))) };
    // SAFETY: `desc` is a live CATapDescription and `capture.tap` a writable id.
    let status = unsafe { AudioHardwareCreateProcessTap(Some(&desc), &mut capture.tap) };
    if status != 0 {
        return Err(format!("couldn't make the audio tap: {status}"));
    }

    // The aggregate device runs on the speakers' clock, so it needs them as a sub-device.
    let mut output = 0u32;
    get(system, address(kAudioHardwarePropertyDefaultOutputDevice), None, &mut output)?;
    let mut output_uid: *const NSString = std::ptr::null();
    get(output, address(kAudioDevicePropertyDeviceUID), None, &mut output_uid)?;
    // SAFETY: the property hands over a +1 CFString (toll-free bridged to NSString).
    let output_uid = unsafe { Retained::from_raw(output_uid as *mut NSString) }.ok_or("no speakers")?;
    let key = |k: &std::ffi::CStr| NSString::from_str(k.to_str().unwrap_or_default());
    let dict = |pairs: &[(&std::ffi::CStr, &AnyObject)]| {
        let keys: Vec<Retained<NSString>> = pairs.iter().map(|(k, _)| key(k)).collect();
        NSDictionary::<NSString, AnyObject>::from_slices(&keys.iter().map(|k| &**k).collect::<Vec<_>>(), &pairs.iter().map(|(_, v)| *v).collect::<Vec<_>>())
    };
    let (yes, no) = (NSNumber::new_bool(true), NSNumber::new_bool(false));
    // SAFETY: reading the tap's UUID back from its own description.
    let tap_uid = unsafe { desc.UUID() }.UUIDString();
    let sub_device = dict(&[(kAudioSubDeviceUIDKey, &output_uid)]);
    let sub_tap = dict(&[(kAudioSubTapUIDKey, &tap_uid), (kAudioSubTapDriftCompensationKey, &yes)]);
    let description = dict(&[
        (kAudioAggregateDeviceNameKey, &NSString::from_str("Mimir meeting")),
        (kAudioAggregateDeviceUIDKey, &NSUUID::new().UUIDString()),
        (kAudioAggregateDeviceMainSubDeviceKey, &output_uid),
        (kAudioAggregateDeviceIsPrivateKey, &yes),
        (kAudioAggregateDeviceIsStackedKey, &no),
        (kAudioAggregateDeviceTapAutoStartKey, &yes),
        (kAudioAggregateDeviceSubDeviceListKey, &NSArray::from_slice(&[&*sub_device])),
        (kAudioAggregateDeviceTapListKey, &NSArray::from_slice(&[&*sub_tap])),
    ]);
    // SAFETY: NSDictionary is toll-free bridged to CFDictionary; `capture.device` is a writable id.
    let status = unsafe {
        AudioHardwareCreateAggregateDevice(&*(Retained::as_ptr(&description) as *const objc2_core_foundation::CFDictionary), NonNull::from(&mut capture.device))
    };
    if status != 0 {
        return Err(format!("couldn't make the tap's audio device: {status}"));
    }
    let mut rate = 0f64;
    get(capture.device, address(kAudioDevicePropertyNominalSampleRate), None, &mut rate)?;
    // SAFETY: the IOProc isn't running yet, so nothing else reads the rate.
    unsafe { (*capture.buf).1 = rate as u32 };
    // SAFETY: `capture.buf` outlives the IOProc (Tap's drop stops it first); `proc_id` is a writable id.
    let status = unsafe { AudioDeviceCreateIOProcID(capture.device, Some(tap_audio), capture.buf.cast(), NonNull::from(&mut capture.proc_id)) };
    if status != 0 {
        return Err(format!("couldn't read the audio tap: {status}"));
    }
    // SAFETY: a device and IOProc made above.
    let status = unsafe { AudioDeviceStart(capture.device, capture.proc_id) };
    if status != 0 {
        return Err(format!("couldn't start the audio tap: {status}"));
    }
    eprintln!("computer sound: process tap at {rate} Hz");
    Ok(Box::new(capture))
}

#[cfg(target_os = "macos")]
fn open_screen_capture(buf: Arc<Mutex<Buf>>) -> Result<Capture, String> {
    use screencapturekit::prelude::*;
    let content = SCShareableContent::get().map_err(|e| format!("{e:?}"))?;
    let display = content.displays().into_iter().next().ok_or("no display")?;
    let filter = SCContentFilter::create().with_display(&display).with_excluding_windows(&[]).build().map_err(|e| format!("{e:?}"))?;
    let config = SCStreamConfiguration::new()
        .with_width(2)
        .with_height(2)
        .with_captures_audio(true)
        .with_excludes_current_process_audio(true)
        .with_sample_rate(48_000)
        .with_channel_count(1);
    let mut stream = SCStream::new(&filter, &config).map_err(|e| format!("{e:?}"))?;
    stream.add_output_handler(
        move |sample: CMSampleBuffer, _: SCStreamOutputType| {
            if let Ok(list) = sample.audio_buffer_list() {
                // Non-interleaved f32: one buffer per channel; average them.
                let chans: Vec<&[f32]> = list.iter().map(|b| bytemuck_f32(b.data())).collect();
                if let Some(first) = chans.first() {
                    let mono: Vec<f32> = (0..first.len()).map(|i| chans.iter().map(|c| c.get(i).copied().unwrap_or(0.0)).sum::<f32>() / chans.len() as f32).collect();
                    push_mono(&buf, 48_000, &mono, 1);
                }
            }
        },
        SCStreamOutputType::Audio,
    )
    .map_err(|e| format!("{e:?}"))?;
    stream.start_capture().map_err(|e| format!("{e:?}"))?;
    Ok(Box::new(Sck(stream)))
}

/// Windows: WASAPI loopback, i.e. recording the default speakers/headset.
#[cfg(windows)]
fn open_system_sound(buf: Arc<Mutex<Buf>>) -> Result<Capture, String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let device = cpal::default_host().default_output_device().ok_or("no speakers")?;
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    Ok(Box::new(open_input(&device, config, buf)?))
}

/// Linux: the default output's monitor source, via parec (PulseAudio, and PipeWire's pipewire-pulse).
#[cfg(all(unix, not(target_os = "macos")))]
fn open_system_sound(buf: Arc<Mutex<Buf>>) -> Result<Capture, String> {
    use std::io::Read;
    struct Kill(std::process::Child);
    impl Drop for Kill {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = std::process::Command::new("parec")
        .args(["-d", "@DEFAULT_MONITOR@", "--format=float32le", "--rate=48000", "--channels=1", "--raw"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("computer sound needs parec (pulseaudio-utils): {e}"))?;
    let mut out = child.stdout.take().ok_or("no parec output")?;
    std::thread::spawn(move || {
        let mut chunk = [0u8; 9600];
        // 50 ms at a time; read_exact keeps the 4-byte samples aligned.
        while out.read_exact(&mut chunk).is_ok() {
            let samples: Vec<f32> = chunk.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
            push_mono(&buf, 48_000, &samples, 1);
        }
    });
    Ok(Box::new(Kill(child)))
}

#[cfg(target_os = "macos")]
fn bytemuck_f32(bytes: &[u8]) -> &[f32] {
    // SAFETY: ScreenCaptureKit delivers 32-bit float PCM; the slice is re-read as f32s, trimmed to whole samples.
    let n = bytes.len() / 4;
    if n == 0 || !(bytes.as_ptr() as usize).is_multiple_of(std::mem::align_of::<f32>()) {
        return &[];
    }
    unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const f32, n) }
}

/// A friendlier name for the app holding the microphone ("ms-teams" → "Teams").
fn app_label(raw: &str) -> String {
    let l = raw.to_lowercase();
    let known = [
        // Meetings and calls (Teams and Zoom include their phone systems).
        ("teams", "Teams"), ("zoom", "Zoom"), ("slack", "Slack"), ("webex", "Webex"), ("goto", "GoTo"), ("skype", "Skype"),
        ("discord", "Discord"), ("whatsapp", "WhatsApp"), ("signal", "Signal"), ("telegram", "Telegram"), ("viber", "Viber"),
        // Business phone systems and softphones.
        ("ringcentral", "RingCentral"), ("dialpad", "Dialpad"), ("aircall", "Aircall"), ("8x8", "8x8"), ("vonage", "Vonage"),
        ("zoiper", "Zoiper"), ("bria", "Bria"), ("3cx", "3CX"), ("nextiva", "Nextiva"), ("ooma", "Ooma"),
        // Your mobile's calls, taken on the computer: iPhone on a Mac (FaceTime's audio daemon), Phone Link on Windows.
        ("facetime", "FaceTime"), ("avconferenced", "Phone (iPhone)"), ("callservicesd", "Phone (iPhone)"), ("yourphone", "Phone Link"), ("phoneexperiencehost", "Phone Link"),
    ];
    let browsers = ["chrome", "msedge", "firefox", "safari", "brave", "opera", "vivaldi"];
    if let Some((_, name)) = known.iter().find(|(k, _)| l.contains(k)) {
        name.to_string()
    } else if browsers.iter().any(|b| l.contains(b)) || l == "arc" {
        "Your browser".into()
    } else {
        raw.to_string()
    }
}

/// Is another app using the microphone (most likely a call)? Some(app name, or "" when the system doesn't say).
/// Asked only while Mimir isn't recording, so its own use never counts.
#[cfg(target_os = "macos")]
pub fn mic_in_use() -> Option<String> {
    use objc2_core_audio::*;
    use std::ptr::NonNull;
    let address = |selector| AudioObjectPropertyAddress { mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain };
    // A property that's an array of u32s (object ids, a flag, a pid).
    let read = |object: u32, selector: u32, out: &mut [u32]| -> Option<usize> {
        let mut size = std::mem::size_of_val(out) as u32;
        // SAFETY: `out` is a writable buffer of `size` bytes; CoreAudio writes at most that and reports how much.
        let status = unsafe { AudioObjectGetPropertyData(object, NonNull::from(&address(selector)), 0, std::ptr::null(), NonNull::from(&mut size), NonNull::new(out.as_mut_ptr())?.cast()) };
        (status == 0).then_some(size as usize / 4)
    };
    let one = |object, selector| {
        let mut v = [0u32];
        read(object, selector, &mut v).filter(|n| *n == 1).map(|_| v[0])
    };
    // Every process using CoreAudio (macOS 14+); the ones recording now, other than Mimir.
    let mut processes = [0u32; 512];
    let n = read(kAudioObjectSystemObject as u32, kAudioHardwarePropertyProcessObjectList, &mut processes)?;
    let me = std::process::id();
    processes[..n].iter().filter(|p| one(**p, kAudioProcessPropertyIsRunningInput) == Some(1)).find_map(|p| {
        let pid = one(*p, kAudioProcessPropertyPID)?;
        (pid != me).then(|| {
            let mut name = [0u8; 64];
            // SAFETY: proc_name writes at most the buffer's size, NUL-terminated.
            let len = unsafe { libc::proc_name(pid as i32, name.as_mut_ptr().cast(), name.len() as u32) };
            app_label(&String::from_utf8_lossy(&name[..len.max(0) as usize]))
        })
    })
}

/// Windows keeps a microphone log for its privacy settings: an app whose last use hasn't stopped is using it now.
#[cfg(windows)]
pub fn mic_in_use() -> Option<String> {
    let base = windows_registry::CURRENT_USER.open(r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone").ok()?;
    let active = |k: windows_registry::Key| k.get_u64("LastUsedTimeStop").is_ok_and(|t| t == 0) && k.get_u64("LastUsedTimeStart").is_ok_and(|t| t > 0);
    // Desktop apps, named by their path with # for \ (Mimir itself skipped).
    let me = std::env::current_exe().map(|p| p.to_string_lossy().replace('\\', "#").to_lowercase()).unwrap_or_default();
    if let Ok(desktop) = base.open("NonPackaged") {
        for name in desktop.keys().ok()?.filter(|n| n.to_lowercase() != me) {
            if desktop.open(&name).is_ok_and(active) {
                return Some(app_label(name.rsplit('#').next().unwrap_or(&name).trim_end_matches(".exe")));
            }
        }
    }
    // Store apps (the new Teams is one): MSTeams_8wekyb3d8bbwe.
    base.keys().ok()?.filter(|n| n != "NonPackaged").find(|n| base.open(n).is_ok_and(active)).map(|n| app_label(n.split('_').next().unwrap_or(&n)))
}

/// Linux: PulseAudio/PipeWire's recording streams, named by app.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn mic_in_use() -> Option<String> {
    let out = std::process::Command::new("pactl").env("LC_ALL", "C").args(["list", "source-outputs"]).output().ok()?;
    String::from_utf8_lossy(&out.stdout).split("Source Output #").skip(1).find_map(|block| {
        // Only apps' streams: sound modules (echo cancelling, remapping) record too but have no application.
        let app = block.lines().find_map(|l| l.trim().strip_prefix("application.name = "))?.trim_matches('"');
        // Level meters (pavucontrol, the sound settings) aren't calls.
        let meter = block.contains("Peak detect") || app.contains("Volume Control") || app.contains("Settings");
        (!meter).then(|| app_label(app))
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_names() {
        assert_eq!(super::app_label("ms-teams"), "Teams");
        assert_eq!(super::app_label("MSTeams"), "Teams");
        assert_eq!(super::app_label("zoom.us"), "Zoom");
        assert_eq!(super::app_label("Google Chrome"), "Your browser");
        assert_eq!(super::app_label("Audacity"), "Audacity");
        assert_eq!(super::app_label("avconferenced"), "Phone (iPhone)");
        assert_eq!(super::app_label("Microsoft.YourPhone"), "Phone Link");
        assert_eq!(super::app_label("RingCentral"), "RingCentral");
    }

    /// The whole after-the-call path on real speech: MIMIR_TEST_MODELS=<dir with the two .onnx files>
    /// MIMIR_TEST_WAV=<16 kHz mono, several voices> cargo test -p mimir-desktop speakers_real -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn speakers_real() {
        let home = std::env::temp_dir().join(format!("mimir-spk-{}", std::process::id()));
        std::fs::create_dir_all(home.join("whisper")).unwrap();
        for f in ["pyannote-segmentation-3.0.onnx", "campplus-sv-zh-en-16k.onnx"] {
            std::fs::copy(std::path::Path::new(&std::env::var("MIMIR_TEST_MODELS").unwrap()).join(f), home.join("whisper").join(f)).unwrap();
        }
        let db = mimir_core::db::Db::open(&home.join("mimir.db")).unwrap();
        // A call at t=1000 whose other side said something at these moments (the recording's turns).
        let id = db.meeting_save("Call", 1_000, 1_060, &[]).unwrap();
        for (t, text) in [(1, "a"), (8, "b"), (14, "c"), (23, "d"), (28, "e"), (49, "f"), (53, "g")] {
            db.meeting_line(id, 1_000 + t, "others", text).unwrap();
        }
        let b = std::fs::read(std::env::var("MIMIR_TEST_WAV").unwrap()).unwrap();
        let audio: Vec<f32> = b[44..].as_chunks::<2>().0.iter().map(|s| i16::from_le_bytes(*s) as f32 / 32768.0).collect();
        super::tape_append(&super::tape_path(&home, id), 0.0, &audio).unwrap();
        let tape = std::fs::read(super::tape_path(&home, id)).unwrap();
        let status = std::sync::Arc::new(std::sync::Mutex::new(super::Status { started_at: 1_000, meeting_id: Some(id), ..Default::default() }));
        let t = std::time::Instant::now();
        super::speakers(db.clone(), status, id, tape).await;
        let who: Vec<String> = db.meeting_lines(id).unwrap().into_iter().map(|l| l.1).collect();
        println!("{:?} {who:?}", t.elapsed());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn echoes_are_recognised() {
        let theirs = ["Morning everyone. The launch has moved to the 14th."];
        assert!(super::is_echo("the launch has moved to the fourteenth", &theirs), "most words match");
        assert!(!super::is_echo("Got it, I'll update the landing page.", &theirs));
        assert!(!super::is_echo("Yes.", &["yes"]), "too short to call it an echo");
        assert!(!super::is_echo("Yes, I think we should ship it.", &["Do you think we should ship it on Monday?"]), "a reply, not an echo");
        assert!(super::is_echo("Morning everyone. The launch has moved to the 14th. Sam, please send the deck", &["Morning everyone. The launch has moved to the 14th.", "Sam, please send the deck to the team by Friday."]), "two of their lines heard as one");
    }

    #[test]
    fn tape_stays_on_the_call_clock() {
        let p = std::env::temp_dir().join(format!("mimir-tape-{}.pcm", std::process::id()));
        let _ = std::fs::remove_file(&p);
        super::tape_append(&p, 1.0, &[0.5; 16_000]).unwrap(); // 1 s of silence first, then 1 s
        super::tape_append(&p, 1.5, &[0.5; 16_000]).unwrap(); // overlaps by 0.5 s: only the new half is added
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 2 * 40_000);
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn gaps_become_silence() {
        let buf = std::sync::Arc::new(std::sync::Mutex::new(super::Buf::default()));
        super::push_mono(&buf, 1_000, &[0.5; 100], 1);
        std::thread::sleep(std::time::Duration::from_millis(900));
        super::push_mono(&buf, 1_000, &[0.5; 100], 1);
        let b = buf.lock().unwrap();
        // 100 + ~800 ms of silence + 100: the buffer spans the time that passed.
        assert!((950..=1_150).contains(&b.samples.len()), "{}", b.samples.len());
        assert_eq!(b.samples[150], 0.0);
    }

    /// `cargo test -p mimir-desktop mic_probe -- --ignored --nocapture` while something uses the mic.
    #[test]
    #[ignore]
    fn mic_probe() {
        println!("mic in use: {:?}", super::mic_in_use());
        println!("microphones: {:?}", super::microphones());
    }
}
