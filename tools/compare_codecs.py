"""Sizes and quality numbers of BAC next to FLAC, Opus, MP3 and Vorbis on the same clips.

    python compare_codecs.py [--bac PATH] [--limit N] [--rate HZ] [--q 2,3,4,5,6,8] FILE_OR_FOLDER...

Needs ffmpeg (with libopus, libmp3lame, libvorbis) on the PATH and a built `bac`. Every codec codes the same mono
16-bit clips; lossy output is decoded back to the clip's sample rate and measured by `bac compare` against the
source. The numbers compare waveforms (see crates/bac/src/metrics.rs): they rank settings of one codec well, they
are rough between codecs, and they do not replace listening.
"""

import argparse
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))


def run(*cmd):
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"{' '.join(cmd)}\n{r.stderr}")
    return r.stdout + r.stderr


def clips(paths):
    out = []
    for p in paths:
        if os.path.isdir(p):
            for root, dirs, files in os.walk(p):
                dirs.sort()
                out += [os.path.join(root, f) for f in sorted(files) if f.lower().endswith((".flac", ".wav"))]
        else:
            out.append(p)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bac", default=os.path.join(HERE, "..", "target", "release", "bac.exe" if os.name == "nt" else "bac"))
    ap.add_argument("--limit", type=int, default=40)
    ap.add_argument("--q", default="2,3,4,5,6,8")
    ap.add_argument("--others", default="opus:12,opus:16,opus:24,opus:32,mp3:32,mp3:48,vorbis:32")
    ap.add_argument("--rate", type=int, help="resample every clip to this rate first")
    ap.add_argument("paths", nargs="+")
    a = ap.parse_args()
    files = clips(a.paths)
    if a.limit and a.limit < len(files):
        files = [files[i * len(files) // a.limit] for i in range(a.limit)]
    ff = ["ffmpeg", "-v", "error", "-y"]
    # name -> [bytes, weighted SNR, segSNR, LSD, NMR, band distance, lossy?]
    rows = {}
    secs = 0.0

    def add(name, size, m=None, dur=0.0):
        r = rows.setdefault(name, [0, 0.0, 0.0, 0.0, 0.0, 0.0, m is not None])
        r[0] += size
        if m:
            for i, v in enumerate(m):
                r[1 + i] += v * dur

    def measure(ref, test):
        out = run(a.bac, "compare", ref, test)
        v = re.search(r"SNR ([-\d.]+) dB, segmental SNR ([-\d.]+) dB, log-spectral distance ([-\d.]+) dB, noise-to-mask ([-\d.]+) dB, band distance ([-\d.]+) dB", out)
        return [float(x) for x in v.groups()]

    with tempfile.TemporaryDirectory() as tmp:
        t = lambda name: os.path.join(tmp, name)
        for n, f in enumerate(files):
            run(*ff, "-i", f, "-ac", "1", *(["-ar", str(a.rate)] if a.rate else []), "-c:a", "pcm_s16le", t("ref.wav"))
            rate = int(run("ffprobe", "-v", "error", "-show_entries", "stream=sample_rate", "-of", "csv=p=0", t("ref.wav")).strip())
            dur = (os.path.getsize(t("ref.wav")) - 44) / 2 / rate
            secs += dur
            add("wav (plain samples)", os.path.getsize(t("ref.wav")))
            run(*ff, "-i", t("ref.wav"), "-c:a", "flac", "-compression_level", "12", t("x.flac"))
            add("flac -12", os.path.getsize(t("x.flac")))
            run(a.bac, "encode", t("ref.wav"), "--lossless", "-o", t("x.bac"))
            add("bac lossless", os.path.getsize(t("x.bac")))
            for spec in filter(None, a.others.split(",")):
                codec, kb = spec.split(":")
                ext, lib, extra = {"opus": ("opus", "libopus", ["-application", "voip"]), "mp3": ("mp3", "libmp3lame", []), "vorbis": ("ogg", "libvorbis", [])}[codec]
                run(*ff, "-i", t("ref.wav"), "-c:a", lib, "-b:a", kb + "k", *extra, t("x." + ext))
                run(*ff, "-i", t("x." + ext), "-ar", str(rate), "-ac", "1", "-c:a", "pcm_s16le", t("dec.wav"))
                add(f"{codec} {kb}k", os.path.getsize(t("x." + ext)), measure(t("ref.wav"), t("dec.wav")), dur)
            for q in a.q.split(","):
                run(a.bac, "encode", t("ref.wav"), "-q", q, "-o", t("x.bac"))
                add(f"bac -q {q}", os.path.getsize(t("x.bac")), measure(t("ref.wav"), t("x.bac")), dur)
            print(f"\r{n + 1}/{len(files)}", end="", file=sys.stderr)
    print(file=sys.stderr)
    print(f"{len(files)} clips, {secs:.0f} s")
    print(f"{'codec':<20} {'kbit/s':>7} {'MB/hour':>8} {'SNR':>6} {'segSNR':>7} {'LSD':>6} {'NMR':>6} {'band':>6}")
    for name, (size, snr, seg, lsd, nmr, band, lossy) in rows.items():
        kb = size * 8 / secs / 1000
        q = f"{snr / secs:6.1f} {seg / secs:7.1f} {lsd / secs:6.2f} {nmr / secs:6.1f} {band / secs:6.2f}" if lossy else f"{'exact':>6}"
        print(f"{name:<20} {kb:7.1f} {kb * 0.45:8.1f} {q}")


if __name__ == "__main__":
    main()
