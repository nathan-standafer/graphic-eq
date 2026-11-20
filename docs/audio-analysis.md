# Audio analysis and drum detection

[Back to the README](../README.md) · [Visualization guide](visualizations.md)

Sparks, Fireworks, Lightning Storm and Ripple Pond all use the drum detector. Fireworks also traces
the spectrum and uses sharp rises in individual frequencies for smaller bursts between drums. `gpu/src/drums.rs` runs in the analyzer thread on the same 1024-point spectrum as the onset envelopes, one
frame every 10 ms: sixteen frequency bands described by their level against a slow average over the last four frames,
how many of their bins rose at once and a median of the rise across neighbouring frequencies (a drum lifts a whole
stretch of the spectrum, a note only its harmonics), and a loudness follower that makes it work at any volume. Five
small neural networks (one hidden layer of 48 units each, weights in `gpu/src/drum_model.bin`) are averaged into three
probabilities: **low** (kick, tom), **snare** (snare, clap, rim) and **high** (hat, cymbal); a picker reports each
hit once, about 20 ms after the drum, with a place on the frequency line, a strength and a confidence. The scenes see
hits whose confidence is at least 0.9 at Sensitivity 1 (about 0.81 for Fireworks during a sharp spectral rise, and 0.76 for Ripple Pond); the Display menu's **sensitivity** lowers or raises that (down
to 0.7) and the fountain still uses it as its rise threshold. The networks are trained on audio rendered from the system's
General MIDI soundfont with known drum times (randomized kits, tempos, instruments, mixes, noise, distortion, speech and
volumes; `gpu/src/corpus.rs`, `gpu/src/drum_train.rs`). On held-out songs it finds about three quarters of the kicks, snares and hats
and 97% of cymbals with 96% precision, against 31% recall for the rise detector it replaced; it still misses most toms,
click-less sub-bass kicks and fast double-bass, and fires a few times a minute on music without drums. It has not been
measured on real recorded drums (see [historical measurements](../gpu/VISUALIZATION_PLAN.md), Milestone 11, for the numbers and the gaps). Run these commands from `gpu/`:

    cargo test --release dump_corpus -- --ignored --nocapture       # writes the labelled test songs as WAVs
    cargo test --release train_drums -- --ignored --nocapture       # retrains (about 12 min); GEQ_WRITE_MODEL=1 installs
    cargo test --release eval_builtin -- --ignored --nocapture      # scores the built-in model against the old detector
    GEQ_REAL_RAW=clip.raw cargo test --release real_music -- --ignored --nocapture   # a capture: hits, rhythm, onset image
    # clip.raw: parec --device=@DEFAULT_MONITOR@ --format=float32le --rate=48000 --channels=2 > clip.raw

**Labelling real songs.** To measure the drum detector on real drums, `gpu/tools/labeler.html` (a single page, open it in a
browser) shows a recording as an *onset image* (a drum is a vertical bar), plays it at reduced speed and lets you tap each
drum type along with it; taps are corrected for your latency and snapped to the sharpest onset of that drum's frequency band, so
the labels carry the sound's timing, not your finger's. It can also show what the detector heard, for review. Capture with
`gpu/tools/capture-song.sh`, then `cargo test --release propose_labels` / `eval_real` (`-- --ignored --nocapture`); labels live
in `gpu/labels/` (the audio is not committed). See `gpu/labels/README.md`.

