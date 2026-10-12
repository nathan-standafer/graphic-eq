#!/bin/bash
# Regenerate the look thumbnails in assets/thumbs from deterministic captures of the synthetic
# demo signal (no audio engine, no routing changes, settings untouched), then rebuild.
#   tools/make-thumbs.sh
set -e
cd "$(dirname "$0")/.."
cargo build --release
bin=target/release/geq-gpu
shot() { # scene preset name seconds
  GEQ_SYNTH=1 GEQ_SETTINGS=default GEQ_SCENE=$1 GEQ_PRESET=$2 GEQ_BARE=1 GEQ_SIZE=1600x450 \
    GEQ_THUMB_W=336 GEQ_SHOT=assets/thumbs/$3.png,$4 timeout 90 $bin >/dev/null
}
shot 0 0 0-studio 12;        shot 0 1 0-amber-rack 12;    shot 0 2 0-ice-deck 12
shot 1 0 1-neon-terrain 12;  shot 1 1 1-classic-mesh 12;  shot 1 2 1-ice-sheet 12
shot 2 0 2-neon-conduit 12;  shot 2 1 2-ice-core 12;      shot 2 2 2-ember-flue 12;  shot 2 3 2-prism 12
shot 3 0 3-p31-green 12;     shot 3 1 3-amber-scope 12;   shot 3 2 3-ice-trace 12;   shot 3 3 3-neon-lissajous 12
shot 4 0 4-ice-geyser 12;    shot 4 1 4-ember-fountain 12; shot 4 2 4-neon-spray 12; shot 4 3 4-spectrum-jets 12
cargo build --release
