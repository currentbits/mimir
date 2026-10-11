"""The film's score and sound effects, synthesised to the picture from src/timeline.json.

Felt piano, pads, a soft half-time kit and Karplus-Strong plucks in D major at the film's tempo;
the sound effects land on the scenes' cues. The instruments and the master are in synth.py.

    python score/score.py            # writes public/score.wav at -16 LUFS
"""
import synth
synth.load("timeline")
from synth import *  # noqa: E402,F403  (after load: the film's clock and buses)

# ---------------- the music, bar by bar ----------------

# The sections follow the picture: drums come in with the memory scene, it fills out with approvals,
# builds from "your AI" and lands on the outro.
S = {k: v["at"] for k, v in T["scenes"].items()}
A, B1, B2, BUILD, CODA = 4, S["memory"], S["approve"], S["ai"], S["outro"]
NIGHT, MORNING = S["server"], S["computer"]  # the laptop sleeps: drums out until the bots' computer
def cyc(pattern, n): return [pattern[i % len(pattern)] for i in range(n)]
PROG = (["D", "Bm", "G", "Asus"]                                                    # the myth, the mark
        + cyc(["D", "Bm", "G", "A"], B1 - A)                                       # always on: the pulse starts
        + cyc(["Bm", "G", "D", "A"], B2 - B1)                                      # memory onwards: drums
        + cyc(["G", "A", "F#m", "Bm", "G", "A", "D", "D/F#", "G", "A", "Bm"], NIGHT - B2)  # fuller
        + cyc(["Em7", "G", "Em7", "Asus"], MORNING - NIGHT)                       # night, on your server
        + cyc(["G", "A", "F#m", "Bm", "G", "A", "D", "D/F#"], BUILD - MORNING)     # morning: its own computer
        + cyc(["G", "A", "Bm", "G", "Em7", "Asus"], CODA - BUILD)                  # the build
        + cyc(["D", "D", "G/D", "D"], T["bars"] - CODA))                           # the mark again
assert len(PROG) == T["bars"]
GAP = at(CODA - 1, 3.5)  # half a beat of nothing before the last chord

MELODY = {B2 + k: v for k, v in {  # bar: [(beat, note, beats)]
    0: [(0, 78, 2), (2.5, 76, 1.5)], 1: [(0, 76, 2), (2.5, 73, 1.5)], 2: [(0, 73, 2), (2.5, 76, 1.5)], 3: [(0, 74, 3.5)],
    4: [(0, 78, 2), (2.5, 79, 1.5)], 5: [(0, 81, 2), (2.5, 76, 1.5)], 6: [(0, 78, 3.5)], 7: [(0, 81, 1.5), (1.5, 78, 1), (2.5, 76, 1.5)],
    8: [(0, 74, 2), (2, 78, 2)], 9: [(0, 76, 3.5)], 10: [(0, 74, 2), (2, 73, 2)]}.items()}
MELODY |= {MORNING + k: MELODY[B2 + k] for k in range(BUILD - MORNING) if B2 + k in MELODY}
MELODY |= {BUILD + k: v for k, v in {0: [(0, 79, 4)], 1: [(0, 81, 4)], 2: [(0, 78, 2), (2, 81, 2)], 3: [(0, 83, 4)]}.items()}
INTRO = {0: [(1, 78), (2.5, 76)], 1: [(0, 74), (1.5, 73), (3, 71)], 2: [(1, 74), (2, 78), (3, 76)], 3: [(1, 76)]}
CODA_TUNE = {CODA + 1: [(0, 78), (1.5, 76)], CODA + 2: [(0, 74), (1.5, 71)], CODA + 3: [(0, 69), (1, 74)]}
PATTERN = (0, 2, 3, 2, 1, 2, 3, 2)

def section(bar):
    if NIGHT <= bar < MORNING: return "night"
    return "intro" if bar < A else "A" if bar < B1 else "B1" if bar < B2 else "B2" if bar < BUILD else "build" if bar < CODA else "coda"

for bar, name in enumerate(PROG):
    b, voicing, keys = CH[name]
    sec = section(bar)
    t0 = at(bar)
    def roll(notes, vel, beat=0.0, length=3.4, spread=0.035):
        for i, m in enumerate(notes):
            key(m, at(bar, beat) + i * spread, vel, length)

    if sec == "intro":
        roll((b + 12, *voicing[1:3]), 0.42, length=2.6)
        for beat, m in INTRO.get(bar, []):
            key(m, at(bar, beat), 0.5, 2.8, gain=1.1, verb=0.5)
        pad(voicing, t0, BAR, gain=0.35 + 0.15 * bar)
    elif sec == "coda":
        if bar == CODA:
            roll((b, b + 12, *voicing[1:], *keys), 0.8, length=7.0, spread=0.03)
            pad((*voicing, 69, 76), t0, 4 * BAR - 1.6, gain=1.2)
            bass(b, t0, 2 * BAR, 0.9)
        for beat, m in CODA_TUNE.get(bar, []):
            key(m, at(bar, beat), 0.45, 4.0, gain=1.0, verb=0.6)
    else:
        sixteenths = sec == "build" and bar >= CODA - 2
        steps = 16 if sixteenths else 8
        for s in range(steps):
            tt = at(bar, s * 4 / steps)
            if tt >= GAP: break
            m = keys[PATTERN[s % 8]]
            vel = (0.62 if s % (steps // 2) == 0 else 0.48 if PATTERN[s % 8] == 3 else 0.42) + 0.04 * rng.standard_normal()
            pedal = at(bar + 1) - tt + 0.05
            key(m, tt, np.clip(vel, 0.3, 0.75), min(2.6, pedal), gain=0.7 if sixteenths else 0.85)
        key(b + 12, t0, 0.5, 2.6, gain=0.8)
        pad(voicing, t0, BAR, gain={"A": 0.55, "night": 0.75, "B1": 0.6, "B2": 0.7, "build": 0.8}[sec])
        if sec in ("A", "night"):
            bass(b, t0, BAR - 0.05, 0.75)
        else:
            bass(b, t0, 2.5 * BEAT, 0.85)
            if at(bar, 2.5) < GAP: bass(b, at(bar, 2.5), 1.4 * BEAT, 0.7)
        for beat, m, ln in (MELODY.get(bar, []) if sec != "night" else []):
            key(m, at(bar, beat), 0.58, ln * BEAT + 0.8, gain=0.9, verb=0.45)

    # the kit
    if sec in ("B1", "B2", "build"):
        four = sec == "build" and bar >= CODA - 4
        for beat in ((0, 1, 2, 3) if four else (0, 2.5)):
            if at(bar, beat) < GAP: kick(at(bar, beat), 0.55 if not four else 0.5)
        if not (sec == "build" and bar == CODA - 1): snap(at(bar, 2), 0.5 if sec == "B1" else 0.6)
        for s in range(16):
            tt = at(bar, s / 4)
            if tt < GAP: shaker(tt, (0.55, 0.22, 0.4, 0.22)[s % 4] * (0.8 if sec == "B1" else 1.0))
        if sec != "B1":
            for s in range(4):
                if at(bar, s + 0.5) < GAP: shaker(at(bar, s + 0.5), 0.5, pan=0.3)
        if sec == "B2" or (sec == "build" and bar < CODA - 2):
            for s in range(16):
                m = keys[(0, 1, 2, 3, 2, 1)[s % 6]] + 12
                pluck(m, at(bar, s / 4), 0.5 if s % 4 == 0 else 0.32, pan=0.45 if s % 2 else -0.45)
    if sec == "build" and bar == CODA - 1:
        for s in range(14):  # the roll into the gap, quickening
            snap(at(bar, s * 3.5 / 14), 0.15 + 0.4 * s / 14)

# the riser through the last two bars of the build
d = GAP - at(CODA - 2)
put(fx_swell({"dur": d / BEAT})[0], at(CODA - 2), 0.16, verb=0.4, to="kit")

cues()

# The pads open up through the film and close again at the end.
PAD = [(0, 700), (at(A), 1100), (at(B1), 1400), (at(B2), 1900), (at(NIGHT), 1900), (at(NIGHT, 3), 950), (at(MORNING) - 0.6, 1300), (at(MORNING), 1900), (at(BUILD), 2000), (GAP, 5200), (GAP + 0.01, 2600), (at(T["bars"]), 900)]
# The arc: quiet myth, the pulse, the drums, fuller, the build; the hit stands on its own.
ARC = list(zip([0, at(A - 0.5), at(A), at(B1), at(B2), at(NIGHT), at(NIGHT, 2), at(MORNING) - 0.01, at(MORNING), at(BUILD), GAP, at(CODA)],
               [0.5, 0.55, 0.72, 0.84, 0.94, 0.94, 0.8, 0.84, 0.96, 0.97, 1.12, 1.0]))
master("score", PAD, ARC, (GAP, at(CODA)))
