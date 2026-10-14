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
# the fountain throws sparks where the volume rises: 13.6 s is a kick and a snare landing together
shot 4 0 4-ice-geyser 13.6;    shot 4 1 4-ember-fountain 13.6; shot 4 2 4-neon-spray 13.6; shot 4 3 4-spectrum-jets 13.6
# the smoke needs twenty seconds of the groove to fill the screen
shot 5 0 5-ice-vent 22;        shot 5 1 5-ember-smoke 22;      shot 5 2 5-neon-haze 22;     shot 5 3 5-prism-plumes 22
# the sand needs a few seconds of the groove to settle into its figures
shot 6 0 6-moonlit-sand 14;    shot 6 1 6-amber-dust 14;       shot 6 2 6-neon-grit 14;     shot 6 3 6-prism-sand 14
# the orb needs a moment for its spikes to answer the groove
shot 7 0 7-obsidian-ice 13.6;  shot 7 1 7-ember-core 13.6;     shot 7 2 7-neon-magnet 13.6; shot 7 3 7-prism-oil 13.6
# the aurora needs a few seconds for its curtains to answer the groove
shot 8 0 8-boreal-green 13.6;  shot 8 1 8-polar-ice 13.6;      shot 8 2 8-neon-veil 13.6;  shot 8 3 8-prism-curtains 13.6
# the fireworks: at 19.9 s a willow, a peony and a crackle are up together, with sky around them
shot 9 0 9-gold-willow 19.9;   shot 9 1 9-ice-chrysanthemum 19.9; shot 9 2 9-neon-peony 19.9; shot 9 3 9-prism-finale 19.9
# the lightning: at 12.6 s two bolts are up with a thunder ring and a splash of sparks
shot 10 0 10-ice-storm 12.6;   shot 10 1 10-ember-strike 12.6;     shot 10 2 10-neon-arc 12.6;   shot 10 3 10-prism-bolt 12.6
cargo build --release
