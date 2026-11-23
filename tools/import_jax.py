"""One conversation of the Bantam-Voice jax set -> one labelled BAC file.

    python import_jax.py FOLDER OUT.bac [--bac PATH] [-q 4 | --lossless]

FOLDER holds `transcript.json` and one FLAC per turn. The output has the whole conversation on its own time line,
the user on channel 1 and the assistant on channel 2 (the channel that is not speaking is digital silence, which
costs nothing). From the transcript it takes: the speakers with what is known of their voices, every turn's text on
the tier `text`, every word with its times on the tier `word`, and the conversation's id, language, set and system
prompt as attributes. Python standard library and a built `bac`.
"""

import argparse
import array
import json
import os
import subprocess
import sys
import tempfile
import wave

HERE = os.path.dirname(os.path.abspath(__file__))


def run(*cmd):
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"{' '.join(cmd)}\n{r.stderr}")
    return r.stderr


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("folder")
    ap.add_argument("out")
    ap.add_argument("--bac", default=os.path.join(HERE, "..", "target", "release", "bac.exe" if os.name == "nt" else "bac"))
    ap.add_argument("-q", default="4")
    ap.add_argument("--lossless", action="store_true")
    a = ap.parse_args()
    t = json.load(open(os.path.join(a.folder, "transcript.json"), encoding="utf-8"))
    rate = t["sample_rate"]
    roles = ["user", "assistant"]
    line = [array.array("h", bytes(2 * round(t["duration"] * rate))) for _ in roles]
    labels = {
        "attrs": {k: str(t[k]) for k in ("id", "language", "set", "strand", "system") if k in t},
        "speakers": [],
        "events": [],
    }
    names = {}
    for i, role in enumerate(roles):
        v = t["voices"][role]
        names[role] = v["voice"]
        attrs = {"role": role, "channel": str(i + 1)}
        attrs.update({k: str(v[k]) for k in ("gender", "age", "accent") if k in v})
        labels["speakers"].append({"id": i + 1, "name": v["voice"], "attrs": attrs})
    with tempfile.TemporaryDirectory() as tmp:
        for turn in t["turns"]:
            # the turn's samples (its first channel: the set stores one voice on two channels)
            wav = os.path.join(tmp, "turn.wav")
            run(a.bac, "decode", os.path.join(a.folder, turn["file"]), "-o", wav)
            with wave.open(wav) as w:
                assert w.getframerate() == rate and w.getsampwidth() == 2
                x = array.array("h", w.readframes(w.getnframes()))[:: w.getnchannels()]
            at = round(turn["start"] * rate)
            track = line[roles.index(turn["role"])]
            if at + len(x) > len(track):
                track.extend([0] * (at + len(x) - len(track)))
            track[at : at + len(x)] = x
            who = names[turn["role"]]
            labels["events"].append({"tier": "text", "start": turn["start"], "end": turn["start"] + len(x) / rate, "speaker": who, "text": turn["text"]})
            for w in turn.get("words", []):
                labels["events"].append({"tier": "word", "start": turn["start"] + w["start"], "end": turn["start"] + w["end"], "speaker": who, "text": w["w"]})
        n = max(len(c) for c in line)
        both = array.array("h", bytes(4 * n))
        for i, c in enumerate(line):
            both[i : 2 * len(c) : 2] = c
        wav = os.path.join(tmp, "all.wav")
        with wave.open(wav, "wb") as w:
            w.setnchannels(2)
            w.setsampwidth(2)
            w.setframerate(rate)
            w.writeframes(both.tobytes())
        with open(os.path.join(tmp, "labels.json"), "w", encoding="utf-8") as f:
            json.dump(labels, f, ensure_ascii=False)
        quality = ["--lossless"] if a.lossless else ["-q", a.q]
        print(run(a.bac, "encode", wav, "-o", a.out, "--import", os.path.join(tmp, "labels.json"), *quality).strip())


if __name__ == "__main__":
    main()
