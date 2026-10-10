# Mimir, the film

A 127-second motion film of what Mimir is and does, made in [Remotion](https://remotion.dev) and scored to the picture.

- **One clock.** `src/timeline.json` holds the tempo (100 BPM), each scene's bars and its cues in beats. The scenes
  animate on those beats and `score/score.py` plays the music and sound effects on the same ones, so every cut and
  click lands on the score.
- **The look is the app's.** Peat-dark water and paper from heymimir.app, Junicode for display, colour only for bots and
  state. The bot faces, icons and pigments are imported from `../ui/src` (face.css is moved by the frame, not by CSS).
- **The score is original.** It's synthesised (felt piano, pads, a soft kit, plucks, the effects), so there's nothing to
  license.

## Make it

```sh
npm install
python3 -m venv /tmp/score && /tmp/score/bin/pip install numpy scipy
/tmp/score/bin/python score/score.py            # → public/score.wav (-16 LUFS)
npx remotion render Mimir out/mimir-film.mp4 --crf=16 --audio-bitrate=256k   # ~2 min on an M-series Mac
```

`npm run dev` opens the Studio. `node stills.mjs /tmp/frames 900,2000 0.5` renders single frames quickly (one bundle,
many frames) for checking the picture. Frames are 36 per beat at 60 fps: scene bar × 144 + beat × 36.

Change a cue's beat in `timeline.json`, then run the score again: picture and sound move together.
