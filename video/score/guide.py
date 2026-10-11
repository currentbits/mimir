"""The guide's score and sound effects, synthesised to the picture from src/guide.json.

Same instruments, key and tempo as the film, and the film's opening motif and closing tune, so the two sound like one
family. The arc follows the guide: calm while you set up, the kit from your first bot, fuller once it's every day, a
breath with no drums while the bots are paused, then a build into the last hit.

    python score/guide.py            # writes public/guide.wav at -16 LUFS
"""
import synth
synth.load("guide")
from synth import *  # noqa: E402,F403  (after load: the guide's clock and buses)

S = {k: v["at"] for k, v in T["scenes"].items()}
A, B1, CARD, B2 = S["what"], S["name"], S["daily"], S["ask"]
NIGHT, MORNING, BUILD, CODA = S["hush"], S["inbox"], S["recap"], S["end"]
def cyc(pattern, n): return [pattern[i % len(pattern)] for i in range(n)]
PROG = (["D", "Bm", "G"]                                              # the mark: how to use Mimir
        + cyc(["D", "G", "Bm", "A"], B1 - A)                         # what it is, and the first steps
        + cyc(["G", "D/F#", "Em7", "Asus"], CARD - B1)               # your first bot onwards: the kit comes in
        + ["A"]                                                       # Part two: a lift, no drums
        + cyc(["D", "A", "Bm", "G"], NIGHT - B2)                     # every day: fuller
        + ["Em7", "Asus"]                                             # paused for the night
        + cyc(["G", "A", "F#m", "Bm"], BUILD - MORNING)              # morning: what needs you, your server
        + ["G", "Em7", "Asus"]                                        # the six steps again: the build
        + ["D", "D", "G/D", "D"])                                     # the mark again
assert len(PROG) == T["bars"]
GAP = at(CODA - 1, 3.5)  # half a beat of nothing before the last chord

PHRASE = {  # every day: bar of the phrase → [(beat, note, beats)] over D A Bm G
    0: [(0, 78, 1.5), (1.5, 76, 0.5), (2, 74, 2)], 1: [(0, 76, 1.5), (1.5, 73, 0.5), (2, 69, 2)],
    2: [(0, 71, 1), (1, 74, 1), (2, 78, 2)], 3: [(0, 79, 2.5), (2.5, 78, 0.5), (3, 76, 1)],
    4: [(0, 78, 2), (2, 81, 2)], 5: [(0, 81, 1.5), (1.5, 79, 0.5), (2, 76, 2)],
    6: [(0, 74, 1), (1, 78, 1), (2, 83, 2)], 7: [(0, 81, 3.5)]}
MELODY = {B2 + k: PHRASE[k % 8] for k in range(16)}
MELODY |= {MORNING + k: v for k, v in enumerate([[(0, 74, 2), (2, 79, 2)], [(0, 81, 1.5), (1.5, 79, 0.5), (2, 76, 2)], [(0, 73, 2), (2, 78, 2)], [(0, 74, 3.5)]])}
MELODY |= {BUILD + k: v for k, v in enumerate([[(0, 79, 4)], [(0, 81, 4)], [(0, 83, 3)]])}
INTRO = {0: [(1, 78), (2.5, 76)], 1: [(0, 74), (1.5, 73), (3, 71)], 2: [(1, 74), (2, 78), (3, 76)]}
CODA_TUNE = {CODA + 1: [(0, 78), (1.5, 76)], CODA + 2: [(0, 74), (1.5, 71)], CODA + 3: [(0, 69), (1, 74)]}
PATTERN = (0, 2, 3, 2, 1, 2, 3, 2)

def section(bar):
    if bar < A: return "intro"
    if bar < B1: return "A"
    if bar < CARD: return "B1"
    if bar < B2: return "card"
    if bar < NIGHT: return "B2"
    if bar < MORNING: return "night"
    if bar < BUILD: return "morning"
    return "build" if bar < CODA else "coda"

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
        pad(voicing, t0, BAR, gain={"A": 0.55, "B1": 0.6, "card": 0.7, "B2": 0.7, "night": 0.75, "morning": 0.7, "build": 0.8}[sec])
        if sec in ("A", "card", "night"):
            bass(b, t0, BAR - 0.05, 0.75)
        else:
            bass(b, t0, 2.5 * BEAT, 0.85)
            if at(bar, 2.5) < GAP: bass(b, at(bar, 2.5), 1.4 * BEAT, 0.7)
        for beat, m, ln in MELODY.get(bar, []):
            key(m, at(bar, beat), 0.58, ln * BEAT + 0.8, gain=0.9, verb=0.45)

    # the kit
    if sec in ("B1", "B2", "morning", "build"):
        four = sec == "build" and bar >= CODA - 2
        for beat in ((0, 1, 2, 3) if four else (0, 2.5)):
            if at(bar, beat) < GAP: kick(at(bar, beat), 0.5 if four else 0.55 if sec != "B1" else 0.5)
        if not (sec == "build" and bar == CODA - 1): snap(at(bar, 2), 0.45 if sec == "B1" else 0.6)
        for s in range(16):
            tt = at(bar, s / 4)
            if tt < GAP: shaker(tt, (0.55, 0.22, 0.4, 0.22)[s % 4] * (0.8 if sec == "B1" else 1.0))
        if sec != "B1":
            for s in range(4):
                if at(bar, s + 0.5) < GAP: shaker(at(bar, s + 0.5), 0.5, pan=0.3)
        if sec in ("B2", "morning") or (sec == "build" and bar < CODA - 2):
            for s in range(16):
                m = keys[(0, 1, 2, 3, 2, 1)[s % 6]] + 12
                pluck(m, at(bar, s / 4), 0.5 if s % 4 == 0 else 0.32, pan=0.45 if s % 2 else -0.45)
    if bar == CARD - 1:
        for s in range(4):  # a little fill into Part two
            snap(at(bar, 3 + s / 4), 0.2 + 0.1 * s)
    if sec == "build" and bar == CODA - 1:
        for s in range(14):  # the roll into the gap, quickening
            snap(at(bar, s * 3.5 / 14), 0.15 + 0.4 * s / 14)

# the riser through the last two bars of the build
d = GAP - at(CODA - 2)
put(fx_swell({"dur": d / BEAT})[0], at(CODA - 2), 0.16, verb=0.4, to="kit")

cues()

# The pads open up as you go, close for the night, and open again for the build.
PAD = [(0, 700), (at(A), 1100), (at(B1), 1400), (at(CARD), 1700), (at(B2), 1900), (at(NIGHT), 1900), (at(NIGHT, 3), 950), (at(MORNING) - 0.6, 1300), (at(MORNING), 1900), (at(BUILD), 2000), (GAP, 5200), (GAP + 0.01, 2600), (at(T["bars"]), 900)]
ARC = list(zip([0, at(A - 0.5), at(A), at(B1), at(CARD), at(B2), at(NIGHT), at(NIGHT, 2), at(MORNING) - 0.01, at(MORNING), at(BUILD), GAP, at(CODA)],
               [0.5, 0.55, 0.7, 0.8, 0.84, 0.94, 0.94, 0.8, 0.84, 0.96, 0.97, 1.12, 1.0]))
master("guide", PAD, ARC, (GAP, at(CODA)))
