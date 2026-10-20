#!/usr/bin/env python3
"""Capture the README gallery from real shaders with deterministic synthetic audio.

Requires a release build and a graphical session. Never changes system audio or
saved settings. Pass a screenshot name to regenerate only that image.
"""
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
BINARY = ROOT / 'gpu/target/release/geq-gpu'
SHOTS = [
    ('hero', 1, 0, 12, False),
    ('columns', 0, 0, 12, True),
    ('waterfall', 1, 0, 12, True),
    ('ferrofluid', 7, 2, 13.6, True),
    ('aurora', 8, 0, 13.6, True),
    ('fountain', 4, 2, 13.6, True),
    ('fireworks', 9, 0, 25.4, True),
    ('lightning', 10, 0, 12.6, True),
    ('ripple-pond', 11, 2, 20.4, True),
]


def main():
    selected = set(sys.argv[1:])
    unknown = selected - {s[0] for s in SHOTS}
    if unknown:
        raise SystemExit('Unknown screenshot: ' + ', '.join(sorted(unknown)))
    output = ROOT / 'docs/images'
    output.mkdir(parents=True, exist_ok=True)
    for name, scene, look, seconds, bare in SHOTS:
        if selected and name not in selected:
            continue
        # Ignore developer capture overrides inherited from the shell.
        env = {k: v for k, v in os.environ.items() if not k.startswith('GEQ_')}
        env.update(GEQ_SYNTH='1', GEQ_SETTINGS='default', GEQ_SCENE=str(scene),
                   GEQ_PRESET=str(look), GEQ_SIZE='1440x720' if bare else '1440x900',
                   GEQ_SHOT=f'{output / (name + ".png")},{seconds}')
        if name == 'fireworks':
            env.update(GEQ_GRADIENT='1.7', GEQ_DENSITY='1.3')
        if bare:
            env['GEQ_BARE'] = '1'
        print('Capturing ' + name, flush=True)
        subprocess.run([str(BINARY)], env=env, check=True, timeout=120)
    print('Gallery ready: ' + str(output), flush=True)


if __name__ == '__main__':
    main()
