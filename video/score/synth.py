"""The instruments, sound effects and master shared by the scores (score.py: the film, guide.py: the guide).

A piece calls load() with its timeline, then `from synth import *` (so T, N, BEAT, bus… are its own), writes its
music into the buses with key(), pad(), bass(), kick()…, places the picture's sound effects with cues(), and
master() mixes, levels and writes public/<name>.wav. Felt piano, pads, a soft kit, Karplus-Strong plucks and the
effects are all synthesised, so there is nothing to license.
"""
import json
import subprocess
from functools import lru_cache
from pathlib import Path

import numpy as np
from scipy.io import wavfile
from scipy.ndimage import maximum_filter1d, minimum_filter1d
from scipy.signal import butter, fftconvolve, lfilter, sosfilt, sosfilt_zi

ROOT = Path(__file__).resolve().parent.parent
SR = 48000
T = BEAT = BAR = N = rng = bus = None
kicks = []

def load(timeline):
    """Set the clock and empty buses from src/<timeline>.json. Call before `from synth import *`."""
    global T, BEAT, BAR, N, rng, bus
    T = json.loads((ROOT / f"src/{timeline}.json").read_text())
    BEAT = 60 / T["bpm"]
    BAR = 4 * BEAT
    N = int(T["bars"] * BAR * SR)
    rng = np.random.default_rng(7)
    bus = {k: np.zeros((2, N)) for k in ("keys", "kit", "fx", "pluck", "duck", "pad", "verb", "room")}
    kicks.clear()
    piano.cache_clear()

def mtof(m): return 440.0 * 2 ** ((m - 69) / 12)
def ts(n): return np.arange(n) / SR
def at(bar, beat=0.0): return (bar * 4 + beat) * BEAT

# ---------------- buses ----------------

def put(sig, t0, gain=1.0, pan=0.0, verb=0.0, room=0.0, to="fx"):
    """Mix a mono or stereo signal in at t0 seconds, panned (-1..1), with sends to the reverbs."""
    sig = np.atleast_2d(sig)
    if sig.shape[0] == 1:
        a = (pan + 1) * np.pi / 4
        sig = np.vstack([sig[0] * np.cos(a), sig[0] * np.sin(a)]) * np.sqrt(2)
    i = int(round(t0 * SR))
    j = min(N, i + sig.shape[1])
    if j <= i: return
    s = sig[:, : j - i] * gain
    bus[to][:, i:j] += s
    if verb: bus["verb"][:, i:j] += s * verb
    if room: bus["room"][:, i:j] += s * room

def lp(x, fc, order=2): return sosfilt(butter(order, min(fc, SR / 2 - 100), "low", fs=SR, output="sos"), x)
def hp(x, fc, order=2): return sosfilt(butter(order, fc, "high", fs=SR, output="sos"), x)
def bp(x, lo, hi, order=2): return sosfilt(butter(order, [lo, min(hi, SR / 2 - 100)], "band", fs=SR, output="sos"), x)

def sweep(x, fc_of_t, kind="low", block=256):
    """A filter whose cutoff follows fc_of_t(seconds) (scalar for low/high, (lo, hi) for band)."""
    out = np.zeros_like(x)
    zi = None
    for s in range(0, len(x), block):
        fc = fc_of_t((s + block / 2) / SR)
        sos = butter(2, fc, {"low": "low", "high": "high", "band": "band"}[kind], fs=SR, output="sos")
        if zi is None: zi = sosfilt_zi(sos) * 0
        out[s : s + block], zi = sosfilt(sos, x[s : s + block], zi=zi)
    return out

def noise(n): return rng.standard_normal(n)

# ---------------- instruments ----------------

@lru_cache(maxsize=None)
def piano(m, vel, length):
    """A felt piano note: slightly inharmonic, two detuned strings, a soft hammer, damped highs."""
    f0 = mtof(m)
    n = int(length * SR)
    t = ts(n)
    out = np.zeros(n)
    B = 0.00018 * (f0 / 262) ** 0.8
    for k in range(1, 18):
        fk = k * f0 * np.sqrt(1 + B * k * k)
        if fk > 9000: break
        amp = k ** -1.3 * np.exp(-(k - 1) * (0.36 - 0.22 * vel))
        tau1 = 0.35 / k ** 0.4
        tau2 = (2.2 + 2.5 * (262 / f0) ** 0.5) / k ** 0.65
        env = 0.5 * np.exp(-t / tau1) + 0.5 * np.exp(-t / tau2)
        ph = rng.uniform(0, 2 * np.pi)
        out += amp * env * 0.5 * (np.sin(2 * np.pi * fk * 1.0004 * t + ph) + np.sin(2 * np.pi * fk * 0.9996 * t + ph * 1.3))
    out *= np.minimum(1, t / 0.004)
    thock = lp(noise(int(0.03 * SR)), 900) * np.exp(-ts(int(0.03 * SR)) / 0.008) * 0.25 * vel
    out[: len(thock)] += thock
    tick = hp(noise(int(0.012 * SR)), 3000) * np.exp(-ts(int(0.012 * SR)) / 0.0025) * 0.05 * vel  # the key's mechanism
    out[: len(tick)] += tick
    out = lp(out, 2600 + 4200 * vel)
    out *= np.minimum(1, (length - t) / 0.12).clip(0, 1)  # damper
    return out * vel

def key(m, t0, vel=0.6, length=3.0, gain=1.0, verb=0.35):
    q = round(vel * 20) / 20
    put(piano(m, q, round(length, 1)), t0, gain, pan=(m - 64) / 40, verb=verb, to="keys")

def saw(f, n, det):
    t = ts(n)
    return sum(2 * ((f * (1 + d) * t + rng.random()) % 1) - 1 for d in det) / len(det)

def pad(notes, t0, dur, gain=1.0):
    n = int((dur + 1.6) * SR)
    t = ts(n)
    env = np.minimum(1, t / 0.9) * np.clip((dur + 1.6 - t) / 1.6, 0, 1)
    L = sum(saw(mtof(m), n, (-0.004, 0.0021, 0.0068)) for m in notes)
    R = sum(saw(mtof(m), n, (-0.0062, -0.0013, 0.0043)) for m in notes)
    put(np.vstack([L, R]) * env / len(notes), t0, gain, verb=0.6, to="pad")

def bass(m, t0, dur, gain=1.0):
    n = int((dur + 0.15) * SR)
    t = ts(n)
    f = mtof(m)
    x = np.sin(2 * np.pi * f * t) + 0.35 * np.sin(4 * np.pi * f * t) + 0.2 * np.sin(np.pi * f * t)
    x = np.tanh(1.6 * x) * np.minimum(1, t / 0.02) * np.clip((dur + 0.15 - t) / 0.15, 0, 1)
    put(x, t0, gain, to="duck")

def kick(t0, gain=1.0):
    n = int(0.6 * SR)
    t = ts(n)
    f = 46 + 95 * np.exp(-t / 0.03)
    x = np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t / 0.3)
    x[: int(0.004 * SR)] += lp(noise(int(0.004 * SR)), 3000) * 0.3
    put(np.tanh(1.3 * x), t0, gain, room=0.15, to="kit")
    kicks.append(t0)

def snap(t0, gain=1.0):
    n = int(0.3 * SR)
    t = ts(n)
    x = bp(noise(n), 1400, 5200) * (np.exp(-t / 0.05) + 0.6 * np.exp(-np.maximum(0, t - 0.011) / 0.045) * (t > 0.011))
    put(x, t0, gain, pan=-0.1, room=0.5, verb=0.15, to="kit")

def shaker(t0, gain=1.0, pan=-0.25):
    n = int(0.08 * SR)
    t = ts(n)
    x = hp(noise(n), 6500) * np.minimum(1, t / 0.006) * np.exp(-t / 0.022)
    put(x, t0, gain, pan=pan, room=0.2, to="kit")

def pluck(m, t0, gain=1.0, pan=0.0):
    f = mtof(m)
    n = int(1.2 * SR)
    P = int(SR / f)
    exc = np.zeros(n)
    exc[:P] = lp(noise(P), 4000)
    a = np.zeros(P + 2); a[0] = 1; a[P] = -0.497; a[P + 1] = -0.497
    x = lfilter([1], a, exc)
    put(lp(x, 5000) * 0.6, t0, gain, pan=pan, verb=0.55, to="pluck")

# ---------------- sound effects ----------------

def fx_drop(o):
    p, n = o.get("pitch", 1.0), int(0.5 * SR)
    t = ts(n)
    f = 650 * p * (1 + 1.4 * (1 - np.exp(-t / 0.025)))
    x = np.sin(2 * np.pi * np.cumsum(f) / SR) * np.minimum(1, t / 0.002) * np.exp(-t / 0.07)
    x += 0.5 * np.sin(2 * np.pi * 170 * p * t) * np.exp(-t / 0.11) * np.minimum(1, t / 0.004)
    return x, dict(gain=0.32 * o.get("gain", 1), verb=0.9)

def fx_pop(o):
    p, n = o.get("pitch", 1.0), int(0.15 * SR)
    t = ts(n)
    f = 560 * p * (1 - 0.25 * np.minimum(1, t / 0.05))
    x = (np.sin(2 * np.pi * np.cumsum(f) / SR) + 0.2 * np.sin(4 * np.pi * np.cumsum(f) / SR)) * np.minimum(1, t / 0.0015) * np.exp(-t / 0.035)
    return x, dict(gain=0.2 * o.get("gain", 1), verb=0.25)

def fx_tick(o):
    n = int(0.03 * SR)
    t = ts(n)
    x = bp(noise(n), 2500, 6000) * np.exp(-t / 0.003) * 0.6 + np.sin(2 * np.pi * 2300 * t) * np.exp(-t / 0.008) * 0.4
    return x, dict(gain=0.09 * o.get("gain", 1), room=0.3)

def click1():
    n = int(0.03 * SR)
    t = ts(n)
    return hp(noise(n), 1800) * np.exp(-t / 0.0025) * 0.7 + np.sin(2 * np.pi * 1350 * t) * np.exp(-t / 0.006) * 0.5

def fx_click(o):
    x = np.zeros(int(0.15 * SR))
    c = click1()
    x[: len(c)] += c
    x[int(0.075 * SR) : int(0.075 * SR) + len(c)] += c * 0.6
    return x, dict(gain=0.2 * o.get("gain", 1), room=0.3)

def fx_type(o):
    d = o.get("dur", 1) * BEAT
    x = np.zeros(int((d + 0.1) * SR))
    s = 0.0
    while s < d:
        c = click1() * (0.5 + 0.5 * rng.random())
        c = bp(c, 1200 + 1500 * rng.random(), 6000)
        i = int(s * SR)
        x[i : i + len(c)] += c[: len(x) - i]
        s += 0.045 + 0.05 * rng.random()
    return x, dict(gain=0.075 * o.get("gain", 1), room=0.25)

def whoosh(dur, lo=300, hi=2600):
    n = int(dur * SR)
    t = ts(n)
    u = t / dur
    x = sweep(noise(n), lambda s: (lo + (hi - lo) * np.sin(np.pi * min(1, s / dur)) ** 2, lo * 1.8 + (hi - lo) * 1.6 * np.sin(np.pi * min(1, s / dur)) ** 2), "band")
    env = np.sin(np.pi * u ** 0.8) ** 2
    pan = -0.6 + 1.2 * u
    a = (pan + 1) * np.pi / 4
    return np.vstack([x * env * np.cos(a), x * env * np.sin(a)]) * np.sqrt(2)

def fx_whoosh(o): return whoosh(0.75), dict(gain=0.16 * o.get("gain", 1), verb=0.25)
def fx_send(o):
    w = np.concatenate([whoosh(0.32, 800, 3200)[0] * 0.6, np.zeros(int(0.1 * SR))])
    p, _ = fx_pop({"pitch": 1.25})
    w[int(0.18 * SR) : int(0.18 * SR) + len(p)] += p * 0.8
    return w, dict(gain=0.17 * o.get("gain", 1), verb=0.2)

def fx_scratch(o):
    n = int(0.42 * SR)
    t = ts(n)
    jitter = 0.5 + 0.5 * np.abs(np.sin(2 * np.pi * 23 * t + 3 * np.sin(2 * np.pi * 7 * t)))
    x = bp(noise(n), 2200, 7000) * jitter * np.minimum(1, t / 0.02) * np.clip((0.42 - t) / 0.08, 0, 1)
    return x, dict(gain=0.05 * o.get("gain", 1), room=0.3)

def bell(f, n):
    t = ts(n)
    idx = 2.2 * np.exp(-t / 0.35)
    return np.sin(2 * np.pi * f * t + idx * np.sin(2 * np.pi * f * 3.5 * t)) * np.exp(-t / 0.9) * np.minimum(1, t / 0.002)

def fx_chime(o):
    n = int(2.2 * SR)
    x = bell(mtof(83), n) * 0.7
    d = int(0.11 * SR)
    x[d:] += bell(mtof(88), n - d)
    return x, dict(gain=0.08 * o.get("gain", 1), verb=0.6, pan=0.35)

def fx_boop(o):
    f, n = mtof(o.get("note", 76)), int(0.6 * SR)
    t = ts(n)
    fs = f * (1 + 0.04 * np.exp(-t / 0.01))
    ph = 2 * np.pi * np.cumsum(fs) / SR
    x = (np.sin(ph) * np.exp(-t / 0.22) + 0.3 * np.sin(4 * ph) * np.exp(-t / 0.04)) * np.minimum(1, t / 0.002)
    return x, dict(gain=0.17 * o.get("gain", 1), verb=0.4, pan=(o.get("note", 76) - 78) / 14)

def fx_thump(o):
    n = int(0.6 * SR)
    t = ts(n)
    f = 52 + 60 * np.exp(-t / 0.04)
    x = np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t / 0.22) + lp(noise(n), 400) * np.exp(-t / 0.03) * 0.4
    return np.tanh(1.2 * x), dict(gain=0.3 * o.get("gain", 1), verb=0.3)

def fx_swell(o):
    d = o.get("dur", 4) * BEAT
    n = int(d * SR)
    t = ts(n)
    u = t / d
    x = sweep(noise(n), lambda s: 400 + 7000 * min(1, s / d) ** 2.2, "low") * u ** 3
    tone = sum(np.sin(2 * np.pi * mtof(m) * t + rng.random() * 6) for m in (57, 64, 69, 76)) / 4 * u ** 2.5
    x = x * 0.5 + tone * 0.6
    x *= np.clip((d - t) / 0.03, 0, 1)
    return x, dict(gain=0.13 * o.get("gain", 1), verb=0.5)

def fx_boom(o):
    n = int(5 * SR)
    t = ts(n)
    f = 38 + 50 * np.exp(-t / 0.25)
    x = np.tanh(1.5 * np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t / 1.6))
    x += lp(noise(n), 1600) * np.exp(-t / 0.35) * 0.35
    x += hp(noise(n), 5000) * np.exp(-t / 1.2) * 0.06
    return x, dict(gain=0.42 * o.get("gain", 1), verb=0.45)

FX = {k[3:]: v for k, v in globals().items() if k.startswith("fx_")}

# ---------------- chords (D major) ----------------

# name: (bass, pad voicing, the piano's four ostinato notes)
CH = {
    "D": (38, (50, 57, 64, 66), (62, 66, 69, 76)),
    "Bm": (35, (47, 54, 62, 66), (59, 62, 66, 73)),
    "G": (43, (43, 50, 59, 66), (62, 66, 67, 71)),
    "A": (45, (45, 52, 61, 64), (61, 64, 69, 71)),
    "Asus": (45, (45, 52, 62, 64), (62, 64, 69, 71)),
    "F#m": (42, (42, 49, 57, 61), (61, 64, 66, 69)),
    "Em7": (40, (40, 47, 55, 62), (62, 64, 67, 71)),
    "D/F#": (42, (42, 50, 57, 64), (62, 66, 69, 74)),
    "G/D": (38, (38, 50, 55, 59, 66), (62, 67, 71, 74)),
}


def cues():
    """The picture's sound effects, on its cues: [beat, sound, options] in the timeline."""
    for s in T["scenes"].values():
        for cue in s["cues"].values():
            if len(cue) < 2: continue
            beat, kind, opts = cue[0], cue[1], (cue[2] if len(cue) > 2 else {})
            sig, kw = FX[kind](opts)
            put(sig, at(s["at"], beat), **kw)

# ---------------- mix ----------------

def ir(seconds, rt60, predelay, bright_rt):
    n = int(seconds * SR)
    t = ts(n)
    out = []
    for ch in range(2):
        nz = noise(n)
        dark = lp(nz, 2600) * np.exp(-6.9 * t / rt60)
        bright = nz * np.exp(-6.9 * t / bright_rt) * 0.35
        x = (dark + bright) * np.minimum(1, t / 0.01)
        x = np.concatenate([np.zeros(int(predelay * SR)), x])
        out.append(x / np.sqrt(np.sum(x ** 2)))
    return out

def convolve(stereo, irs):
    return np.vstack([fftconvolve(stereo[c], irs[c])[:N] for c in range(2)])

def master(name, pad_curve, arc, gap):
    """Mix the buses and write public/<name>.wav at -16 LUFS.

    pad_curve: [(seconds, cutoff Hz)], the pads opening and closing. arc: [(seconds, gain)], the piece's dynamics.
    gap: (start, end) seconds of silence before the last hit.
    """
    xs, ys = zip(*pad_curve)
    padded = np.vstack([hp(sweep(bus["pad"][c], lambda s: float(np.interp(s, xs, ys))), 110) for c in range(2)])

    # Sidechain: the pads and bass breathe around each kick.
    duck = np.ones(N)
    tt = ts(N)
    for k in kicks:
        i = int(k * SR)
        j = min(N, i + int(0.4 * SR))
        duck[i:j] = np.minimum(duck[i:j], 1 - 0.38 * np.exp(-(tt[i:j] - k) / 0.13))
    bed = (bus["duck"] * 0.5 + padded * 1.4) * duck  # balance measured with level() below

    hall = convolve(bus["verb"] + padded * 0.25, ir(3.6, 3.1, 0.03, 0.9))
    room = convolve(bus["room"], ir(0.8, 0.55, 0.008, 0.3))
    music = (bus["keys"] + bus["kit"] + bus["pluck"] * 2 + bed + hall * 0.4 + room * 0.5) * np.interp(tt, *zip(*arc))
    mix = music + bus["fx"] * 2.2

    # Silence for the gap, then the hit; a short fade at the very end.
    g0, g1 = int(gap[0] * SR), int(gap[1] * SR)
    fade = np.ones(N)
    fade[g0:g1] = np.linspace(1, 0, g1 - g0) ** 6
    fade[-int(2.0 * SR):] *= np.linspace(1, 0, int(2.0 * SR)) ** 2
    mix *= fade
    mix = np.vstack([hp(mix[c], 28) + 0.2 * hp(mix[c], 6000) for c in range(2)])  # a little air on top
    mix *= 0.9 / np.max(np.abs(mix))

    def level(v, a, b):
        v = v[:, int(a * SR) : int(b * SR)]
        h = np.vstack([hp(v[c], 150) for c in range(2)])
        return 20 * np.log10(np.sqrt(np.mean(v ** 2)) + 1e-12), 20 * np.log10(np.sqrt(np.mean(h ** 2)) + 1e-12)
    for k, v in [("keys", bus["keys"]), ("pad", padded * 1.4 * duck), ("bass", bus["duck"] * 0.5 * duck), ("kit", bus["kit"]), ("pluck", bus["pluck"] * 2), ("fx", bus["fx"] * 2.2), ("hall", hall * 0.4)]:
        print(f"{k:5s}", "  ".join(f"{a}-{b}s rms {r:6.1f} hp {h:6.1f}" for a, b in ((10, 28), (50, 70)) for r, h in [level(v, a, b)]))
    # Loudness for the web: measure (ffmpeg's EBU R128), apply plain gain to -16 LUFS, and catch the few peaks
    # above -2.4 dBFS with a 4 ms lookahead limiter, so the arc stays as written.
    raw, out = ROOT / f"score/{name}.raw.wav", ROOT / f"public/{name}.wav"
    wavfile.write(raw, SR, (mix.T * 32767).astype(np.int16))
    m = subprocess.run(["ffmpeg", "-hide_banner", "-i", raw, "-af", "loudnorm=print_format=json", "-f", "null", "-"], capture_output=True, text=True).stderr
    lufs = float(json.loads(m[m.rindex("{") : m.rindex("}") + 1])["input_i"])
    y = mix * 10 ** ((-16 - lufs) / 20)
    ceiling = 10 ** (-2.4 / 20)  # AAC adds ~0.6 dB of overshoot
    w = int(0.004 * SR)
    g = minimum_filter1d(np.minimum(1, ceiling / np.maximum(maximum_filter1d(np.max(np.abs(y), axis=0), 2 * w + 1), 1e-9)), 2 * w + 1)
    a = np.exp(-1 / (0.012 * SR))
    g = np.minimum(g, lfilter([1 - a], [1, -a], g, zi=[g[0] * a])[0])
    y = np.clip(y * g, -ceiling, ceiling)
    wavfile.write(out, SR, (y.T * 32767).astype(np.int16))
    print("wrote", out, f"{N / SR:.1f}s; mix was {lufs} LUFS, limiter took up to {-20 * np.log10(g.min()):.1f} dB")
