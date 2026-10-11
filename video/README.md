# Mimir, the films

Two motion films made in [Remotion](https://remotion.dev), each scored to the picture:

- **Mimir** (`src/Film.tsx`, 127 s): what Mimir is and does.
- **Guide** (`src/Guide.tsx`, 154 s): how to use it. Six setup steps as they look in the app (download, pick your AI,
  name a bot, put it on your phone, connect email and calendar, add skills), then things to try every day: asking,
  schedules and calendar-timed routines, quiet checks, tap buttons, approvals and receipts, quick ask, corrections,
  meeting notes, pausing, Needs you and your own server.

How they're made:

- **One clock.** `src/timeline.json` (the film) and `src/guide.json` (the guide) hold the tempo (100 BPM), each scene's
  bars and its cues in beats. The scenes animate on those beats and the score plays the music and sound effects on the
  same ones, so every cut and click lands on the score. Both share `src/lib.tsx`, so scene ids must differ.
- **The look is the app's.** Peat-dark water and paper from heymimir.app, Junicode for display, colour only for bots and
  state. The bot faces, icons and pigments are imported from `../ui/src` (face.css is moved by the frame, not by CSS).
  The guide's app windows (`src/guide/kit.tsx`) are drawn after the real screens in `ui/src/App.tsx`.
- **The scores are original.** `score/synth.py` synthesises the instruments (felt piano, pads, a soft kit, plucks) and
  the effects; `score/score.py` and `score/guide.py` are the two arrangements. Nothing to license.

## Make it

```sh
npm install
python3 -m venv /tmp/score && /tmp/score/bin/pip install numpy scipy
/tmp/score/bin/python score/score.py            # → public/score.wav (-16 LUFS)
/tmp/score/bin/python score/guide.py            # → public/guide.wav
npx remotion render Mimir out/mimir-film.mp4 --crf=16 --audio-bitrate=256k    # ~2 min on an M-series Mac
npx remotion render Guide out/mimir-guide.mp4 --crf=16 --audio-bitrate=256k   # ~5 min
```

`npm run dev` opens the Studio. `node stills.mjs /tmp/frames 900,2000 0.5 Guide` renders single frames quickly (one
bundle, many frames) for checking the picture. Frames are 36 per beat at 60 fps: scene bar × 144 + beat × 36.

Change a cue's beat in a timeline, then run its score again: picture and sound move together.
