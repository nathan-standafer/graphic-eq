#!/usr/bin/env python3
"""Turns the MDB-Drums annotations into this repo's `*.labels.json` (see labels/README.md), to score the drum detector
on real drums in real mixes without labelling anything by hand.

    git clone --depth 1 https://github.com/CarlSouthall/MDBDrums.git labels/audio/mdb-drums
    tools/import-mdb.py                          # full mixes -> labels/audio/mdb-drums/labels-full_mix/
    tools/import-mdb.py --audio drum_only        # the drums alone -> labels-drum_only/
    GEQ_REAL_DIR="labels/audio/mdb-drums/MDB Drums/audio/full_mix" GEQ_LABEL_DIR=labels/audio/mdb-drums/labels-full_mix \\
        cargo test --release eval_real -- --ignored --nocapture

The labels go next to the dataset, not into `labels/`: MDB-Drums is CC BY-NC-SA 4.0, so its times are not committed here.

Subclass codes (the dataset's `annotations/subclass`): the hits the detector is asked for keep their kind; playing techniques it
has no business telling apart (ghost notes, flams and drags, brush strokes, snares off, a pedalled hat) and the tambourine are
written as ghost notes, which are neither asked for nor counted as false hits when the detector reports one.
"""
import argparse
import json
import os
import wave

KIND = {
    "KD": "kick",
    "SD": "snare", "SST": "snare",
    "CHH": "hat", "OHH": "hat",
    "RDC": "cymbal", "RDB": "cymbal", "CRC": "cymbal", "CHC": "cymbal", "SPC": "cymbal",
    "LFT": "tom", "MHT": "tom", "HFT": "tom", "HIT": "tom",
}
GHOST = {
    "SDG": "snare", "SDB": "snare", "SDF": "snare", "SDD": "snare", "SDNS": "snare",
    "PHH": "hat",
    "TMB": "cymbal",
}
# "OT" (other) is dropped; so is anything unknown, with a warning.

# The detector settles in about 4 s (drum_eval.rs `FROM`); the excerpt starts there.
LEAD_IN = 4.0
TEST_EVERY = 3  # every third track (alphabetically, from the third) is the test split


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", default=os.path.join(os.path.dirname(__file__), "..", "labels", "audio", "mdb-drums", "MDB Drums"))
    ap.add_argument("--audio", choices=["full_mix", "drum_only"], default="full_mix")
    ap.add_argument("--out", help="default: labels-<audio> next to the dataset")
    a = ap.parse_args()
    root = os.path.normpath(a.root)
    out = a.out or os.path.join(os.path.dirname(root), "labels-" + a.audio)
    os.makedirs(out, exist_ok=True)
    wavs = sorted(f for f in os.listdir(os.path.join(root, "audio", a.audio)) if f.endswith(".wav"))
    unknown = {}
    totals = {}
    for i, name in enumerate(wavs):
        stem = name.rsplit("_", 1)[0]  # MusicDelta_Rock_MIX.wav / MusicDelta_Rock_Drum.wav
        with wave.open(os.path.join(root, "audio", a.audio, name)) as w:
            dur = w.getnframes() / w.getframerate()
        labels = []
        for line in open(os.path.join(root, "annotations", "subclass", stem + "_subclass.txt")):
            f = line.split()
            if len(f) < 2:
                continue
            t, code = float(f[0]), f[1]
            if code in KIND:
                labels.append({"t": t, "kind": KIND[code], "ghost": False, "source": "mdb:" + code})
            elif code in GHOST:
                labels.append({"t": t, "kind": GHOST[code], "ghost": True, "source": "mdb:" + code})
            elif code != "OT":
                unknown[code] = unknown.get(code, 0) + 1
        labels.sort(key=lambda l: l["t"])
        split = "test" if i % TEST_EVERY == TEST_EVERY - 1 else "dev"
        doc = {
            "version": 1,
            "audio": {"name": name, "duration": round(dur, 3)},
            "split": split,
            "excerpt": [LEAD_IN, round(dur, 3)],
            "tapLatencyMs": 0,
            "notes": "MDB-Drums (Southall et al., ISMIR 2017), CC BY-NC-SA 4.0; imported by tools/import-mdb.py",
            "labels": labels,
            "noDrums": [],
        }
        with open(os.path.join(out, stem + ".labels.json"), "w") as fh:
            json.dump(doc, fh, indent=1)
        asked = [l for l in labels if not l["ghost"] and l["t"] >= LEAD_IN]
        totals[split] = totals.get(split, 0) + len(asked)
        print(f"{stem:26} {split:4} {dur:6.1f} s  {len(asked):4} hits asked for, {sum(l['ghost'] for l in labels):3} ghost/other")
    print(f"-> {out}: {len(wavs)} tracks; hits asked for per split {totals}")
    if unknown:
        print("unknown codes skipped:", unknown)


if __name__ == "__main__":
    main()
