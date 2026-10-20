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
# the fountain throws sparks where the volume rises: 13.6 s is a kick and a snare landing together
shot 4 0 4-ice-geyser 13.6;    shot 4 1 4-ember-fountain 13.6; shot 4 2 4-neon-spray 13.6; shot 4 3 4-spectrum-jets 13.6
# the orb needs a moment for its spikes to answer the groove
shot 7 0 7-obsidian-ice 13.6;  shot 7 1 7-ember-core 13.6;     shot 7 2 7-neon-magnet 13.6; shot 7 3 7-prism-oil 13.6
# the aurora needs a few seconds for its curtains to answer the groove
shot 8 0 8-boreal-green 13.6;  shot 8 1 8-polar-ice 13.6;      shot 8 2 8-neon-veil 13.6;  shot 8 3 8-prism-curtains 13.6
# the fireworks: at 19.9 s a willow, a peony and a crackle are up together, with sky around them
shot 9 0 9-gold-willow 19.9;   shot 9 1 9-ice-chrysanthemum 19.9; shot 9 2 9-neon-peony 19.9; shot 9 3 9-prism-finale 19.9
# the lightning: at 12.6 s two bolts are up with a thunder ring and a splash of sparks
shot 10 0 10-ice-storm 12.6;   shot 10 1 10-ember-strike 12.6;     shot 10 2 10-neon-arc 12.6;   shot 10 3 10-prism-bolt 12.6
# the pond: at 20.4 s several rings from the last seconds of hits cross, with towers lit and spray in the air
shot 11 0 11-moonlit-pond 20.4; shot 11 1 11-lantern-pond 20.4;    shot 11 2 11-neon-canal 20.4; shot 11 3 11-prism-rain 20.4
cargo build --release
