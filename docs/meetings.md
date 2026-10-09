# Meetings and calls

Mimir transcribes meetings and calls **on your computer**, with no bot joining the call: Teams, Zoom, Google Meet,
Webex, Slack huddles, WhatsApp, FaceTime, and phone calls made from the computer (Teams Phone, Zoom Phone, RingCentral,
Dialpad, Aircall, softphones). Then a bot can write the notes, and you can ask your bots about any meeting later.

Meetings are a **desktop app** feature (they record this computer's microphone and sound).

## Using it

1. **Start.** Sidebar → **Meetings** → **Start transcribing**, or the menu-bar / tray icon → *Transcribe a meeting*.
   Mimir can also offer: a notification when another app has used your microphone for 20 seconds ("Teams is using your
   microphone"), or when a meeting from your calendar starts.
2. **During the call** the transcript fills in a few seconds after each pause: **You** (your microphone) and
   **Others** (the computer's sound). Jot a few words under **Your notes**; the notes will be built around them.
3. **Stop.** Everything is already saved. The meeting opens with a field to name it (it's named after the calendar event
   or the app, e.g. "Weekly standup" or "Teams call"). In the background, the other side is split by voice into
   **Speaker 1, 2…**; click a speaker's label to give them a name.
4. **Notes.** Set **When it ends** to a bot ("Ada writes the notes") and it writes a summary, decisions, action items
   with owners and anything you promised, as soon as the call ends; you get a notification and the notes appear on the
   meeting and in the chat. Or press **Summarise with…** on any saved meeting.
5. **Later.** Ask any bot, from Telegram or the app: "what did we decide in this morning's standup?", "what did Priya
   say about the budget?". Bots use `meeting_search` and `meeting_read`. **Search what was said** in Meetings does the
   same for you.

**Tell people you're transcribing.** Many places require the consent of everyone on the call.

### Settings (in the Meetings card)

- **Your microphone**: pick your headset if it isn't the system default (otherwise the default mic is "You").
- **When it ends**: just save the transcript, or have a bot write the notes.
- **Calendar**: paste your calendar's private iCal link (see [Calendar](#calendar)).
- **Offer to transcribe when a call starts**: the microphone-in-use notification.

### Calls on your phone

A call that stays on your phone can't be heard by the computer. Take it on the computer instead: iPhone calls on a
Mac (Continuity), Phone Link on Windows. Then it's transcribed like any other call. On speakerphone next to the
computer it also works, but everything reads as **You** (both voices come through the microphone).

## Permissions

- **macOS** asks for the **Microphone** and, for the call's sound, for **System Audio Recording** the first time (it's
  listed under Screen & System Audio Recording → System Audio Recording Only). Macs before macOS 14.4 ask for
  **Screen & System Audio Recording** instead.
  If one is missing, the live card says so: macOS gives an app without microphone access pure silence rather than an
  error, so Mimir watches for that and tells you.
- **Windows**: Settings → Privacy & security → Microphone must allow desktop apps.
- **Linux**: the computer's sound comes from PulseAudio / PipeWire through `parec` (package `pulseaudio-utils`).

## How it works

```
 microphone ─────────▶ "You" buffer ─┐
 computer's sound ───▶ "Others" buffer ┼─▶ chunk at a pause ─▶ voice activity ─▶ whisper.cpp ─▶ lines (saved at once)
                                      │       (6–30 s)         (only speech)     (on device)
                                      └─▶ the others' audio, kept until Stop ─▶ who's who (speakers) ─▶ deleted
```

**Two sources, two speakers.** Your microphone is recorded with `cpal`. The computer's sound comes from
a Core Audio process tap on macOS 14.4 and later (every app's sound except Mimir's; ScreenCaptureKit on older Macs
or if the tap fails), WASAPI loopback on Windows, and the
PulseAudio / PipeWire monitor on Linux. Because they're separate streams, the transcript knows what *you* said
without guessing.

**Chunks end at pauses.** Each second Mimir checks both streams; when both have been quiet for 0.7 s, and at least a
few seconds have passed, the new audio becomes a chunk (at the latest after 30 s). The shortest chunk is twice what the
last one took to transcribe, so a slower computer automatically uses longer chunks and never falls behind.

**Only speech is transcribed.** A small voice-activity model (Silero, ~1 MB) finds the stretches where someone speaks;
only those go to whisper.cpp, each stamped at its real time. Silence costs nothing, and whisper doesn't invent words
over quiet stretches ("Thanks for watching").

**whisper.cpp is built into the app.** Nothing to install; on a Mac it runs on the GPU (about 0.15 s per stretch). The
speech model (`base`, ~150 MB; setting `whisper_model`: tiny, base, small, medium, large-v3-turbo, large-v3) downloads
once, when you first press Start. Without the built-in engine (the server or CLI build), an installed `whisper-cli` is
used, or your OpenAI-compatible key.

**On the wall clock.** The computer's sound sends nothing while nothing plays, so each stream is kept in step with
the clock (gaps are filled with silence; a long gap, like the laptop sleeping, is capped). Lines from both sides then
interleave in the order people spoke.

**Echo.** On speakers, your microphone also hears the others. A "You" line whose word pairs mostly match what the
others said in the same moment is dropped as an echo. Headphones still work best.

**Saved as it goes.** Every line is written to the database the moment it's transcribed. Quitting mid-call finishes
the meeting first (up to 20 s); a chunk that fails to transcribe (a network blip, the model still downloading) is kept
and retried.

**Who's who.** While recording, the others' audio is kept in a temporary file on the call's clock. After Stop, it's read
into memory and the file is deleted; in the background, sherpa-onnx runs pyannote's speaker segmentation and 3D-Speaker
voice embeddings (about 35 MB of models, downloaded on first use) and relabels each line Speaker 1, 2… in order of
first appearance. One voice stays **Others**. It takes about 8% of the call's length (around 5 minutes for an hour) and
runs entirely on your computer, on macOS, Windows and Linux (on Linux, sherpa-onnx's ~21 MB library is downloaded on first
use, as there's no build of it to include in the app).

**Detecting a call.** macOS 14+: CoreAudio reports which processes are recording (other than Mimir). Windows: the
microphone privacy log in the registry shows which app is using the mic. Linux: PulseAudio's recording streams, by app
name (sound modules like echo cancellation don't count). After 20 seconds you get one notification, naming the app.

## Calendar

Paste your calendar's **private iCal link** in Meetings → Calendar:

- **Google Calendar**: Settings → your calendar → *Integrate calendar* → **Secret address in iCal format**.
- **Outlook / Microsoft 365**: Settings → Calendar → Shared calendars → *Publish a calendar* → the ICS link.

Anyone with that link can read your calendar, so Mimir keeps it in the OS keychain and checks it before saving ("6
meetings this week"). It's fetched at most every 10 minutes. With it:

- a recording takes the name of the meeting on now, and its invite list (rooms are left out);
- a notification offers to transcribe when a meeting with other people starts;
- bots see who was invited, and can say which **Speaker N** is probably who (and say when it's a guess);
- in a chat with you, bots can read the calendar between two dates ("what's on tomorrow?"), and the Meeting prep
  skill uses it ([Connect your email, calendar and web search](connect.md#calendar)).

Repeating meetings, skipped dates, moved occurrences and time zones are handled. All-day events and cancelled ones are
ignored. Outlook's Windows-style time zone names fall back to your computer's time zone.

## Bots and meetings

- `meeting_search` finds lines that mention a word, name or topic across meetings (or lists the latest ones);
  `meeting_read` returns one meeting: who was invited, the notes a bot wrote, your notes, then the transcript (a very
  long one keeps its start and end; the middle is marked as cut). They're offered only in the app and in direct chats,
  never in a group, channel or server (the reply would go to everyone there).
- A meeting's title comes from the calendar invite (anyone can send you one) or the call app, so it's outside content
  too: the notes bot isn't told it in its request, only through `meeting_read`. Calendar entries that repeat more often
  than hourly count once, and a calendar link that sends more than 10 MB is refused.
- What other people said is **outside content**: after reading a meeting, a bot asks before changing anything (see
  [Tools and safety](tools-and-safety.md#outside-content-taint)). That's also why the automatic notes bot reads the
  meeting with `meeting_read` rather than having the transcript pasted in as if you'd said it.
- Meetings live on the computer that recorded them. When the app is connected to [your server](self-hosting.md),
  meetings stay on this computer; *Summarise with* still sends the text to the server's bot, but server bots can't
  search desktop meetings.

## Where it's stored

The transcript, your notes, speaker names, the invite list and the bot's notes are in `~/.mimir/mimir.db`. The speech
and speaker models are in `~/.mimir/whisper/`. No audio is kept after a meeting ends. See [Privacy](privacy.md).

## Transcribing a recording

`mimir transcribe <file>` turns an audio or video file into text (ffmpeg converts it first), with an installed
`whisper-cli` or your OpenAI-compatible key.

## Troubleshooting

| You see | What to do |
|---|---|
| "Your microphone is sending pure silence" | macOS: allow Mimir under System Settings → Privacy & Security → Microphone, or pick another microphone |
| "The computer's sound … isn't being recorded" | macOS: allow Mimir under Screen & System Audio Recording (System Audio Recording Only on 14.4+), then start again. Linux: install `pulseaudio-utils` |
| Others' words appear under **You** too | You're on speakers and the echo wasn't caught; use headphones |
| Your words are missing | Pick your headset under **Your microphone** if the call app uses it |
| "Downloading the speech model…" for a long time | The first recording downloads ~150 MB; lines appear once it's done |
| Speakers aren't split | Needs two or more voices on the other side, and the models (and on Linux the library) downloaded once |
