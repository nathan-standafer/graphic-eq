#!/bin/bash
# Record what is playing (the default output's monitor: read-only, no routing changes) to labels/audio/NAME.wav as
# 48 kHz stereo float, the same signal the app analyzes. Start the song first, then this; it stops after SECONDS
# (default 120) or on Ctrl-C. Begin a few seconds before the stretch you want to label: the detector needs about 3 s
# of lead-in. The audio is not committed (labels/audio is ignored); the labels are.
#   tools/capture-song.sh NAME [SECONDS]
set -e
cd "$(dirname "$0")/.."
name=${1:?usage: tools/capture-song.sh NAME [SECONDS]}
secs=${2:-120}
mkdir -p labels/audio
out=labels/audio/$name.wav
echo "Recording up to $secs s of the default output to $out (Ctrl-C stops early)"
timeout -s INT "$secs" parec --device=@DEFAULT_MONITOR@ --file-format=wav --format=float32le --rate=48000 --channels=2 "$out" || true
ls -la "$out" | awk '{print $5 " bytes written"}'
