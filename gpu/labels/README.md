# Real-drum labels

Hand-checked drum times for real recordings, to measure the drum detector on real drums (the training data is rendered from a
soundfont; see `VISUALIZATION_PLAN.md`, Milestone 11). The **labels** (`*.labels.json`, only times) are committed here; the
**audio** is not (`audio/` is ignored: it is copyrighted music you captured).

## Workflow

1. **Capture** a song while it plays (read-only; the same signal the app analyzes). Start a few seconds before the stretch you
   want to label, since the detector needs about 3 s of lead-in:

       tools/capture-song.sh my-song 120        # labels/audio/my-song.wav, 120 s (Ctrl-C stops early)

2. **Proposals** (optional, for review): what the detector hears at confidence 0.4 or more, written next to each WAV:

       cargo test --release propose_labels -- --ignored --nocapture

3. **Label** with `tools/labeler.html` (open it in a browser: *Open audio…*, then *Proposals…*). The picture shows what rises in
   the sound; a drum is a vertical bar through many rows. Press **Help (?)** for the keys. In short:
   - *Measure* your tap latency once. Play at 0.5×. Do one pass per drum type: pick it (`K` kick, `T` tom, `S` snare, `H` hat,
     `C` cymbal) and tap along (`Space` taps the picked drum, `Shift` marks a soft ghost note). Each tap snaps to the sharpest
     onset of that kind's frequency band near it, so the timing is the sound's, not your finger's.
   - **Your own passes first.** Then press `V` to show the detector's marks (hollow triangles; the bright ones have no mark of
     yours near them), press `E` and click them in their lane to accept, or click your marks to delete.
   - Mark the **excerpt** you labelled (`I` and `O`) and stretches with **no drums** (`N`), so false hits are counted too.
   - Pick the **split**: `dev` (tune on these) or `test` (leave alone until a final report), then *Export* and put
     `my-song.labels.json` in this folder.

4. **Score** the built-in detector on every labelled song, dev and test separately, next to the old detector:

       cargo test --release eval_real -- --ignored --nocapture        # GEQ_CONF=0.8 for another confidence

## What makes a good set

- Split by **song**, never by frame; keep songs of one band or album in the same split.
- About 1,000 labelled hits per drum type gives roughly ±1.5% on recall: at rock tempos 12-16 excerpts of 60-90 s. Toms and
  cymbals need songs picked to have them.
- Variety: eras, tempos, drummers, loud compressed mixes with vocals and distortion; a few stretches without drums.
- Keep the `test` songs untouched while tuning. Real labels can also train the model later, from `dev` songs only.

## File format (`*.labels.json`)

    {"version": 1, "audio": {"name": "my-song.wav", "duration": 123.4}, "split": "dev", "excerpt": [12.0, 72.0],
     "tapLatencyMs": 150, "notes": "",
     "labels": [{"t": 12.345, "kind": "kick|tom|snare|hat|cymbal", "ghost": false, "snapped": true, "source": "tap|edit|proposal"}],
     "noDrums": [[30.0, 34.5]]}

Kick and tom score as the *low* output, snare as *snare*, hat and cymbal as *high*. A ghost note is not asked of the detector
(and a detection of it is not a false hit). `GEQ_REAL_DIR` and `GEQ_LABEL_DIR` move the audio and label folders.
