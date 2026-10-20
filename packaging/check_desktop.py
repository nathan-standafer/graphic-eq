#!/usr/bin/env python3
"""Opt-in desktop smoke test: synthetic audio, temporary settings/session, no audio routing."""
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / 'gpu/target/release/geq-gpu'


def main():
    with tempfile.TemporaryDirectory(prefix='sfeq-desktop-') as tmp:
        env = dict(os.environ, GEQ_SYNTH='silence', GEQ_SETTINGS='default',
                   SPECTRAFORGE_RUNTIME_DIR=tmp + '/run', XDG_CONFIG_HOME=tmp + '/config')
        processes = []
        with open('/tmp/spectraforge-desktop-test.log', 'w') as log:
            def start():
                process = subprocess.Popen([str(BINARY), '--background'], env=env, stdout=log, stderr=log)
                processes.append(process)
                return process

            def command(arg):
                return subprocess.check_output([str(BINARY), arg], env=env, text=True, stderr=log, timeout=6).strip()

            def wait_for(predicate, description):
                deadline = time.monotonic() + 30
                last = None
                while time.monotonic() < deadline:
                    try:
                        last = json.loads(command('--status'))
                        if predicate(last):
                            print(description + ': OK', flush=True)
                            return last
                    except (subprocess.SubprocessError, json.JSONDecodeError):
                        pass
                    time.sleep(.2)
                raise AssertionError(f'{description}: timed out; last state={last}. See /tmp/spectraforge-desktop-test.log')

            try:
                p = start()
                wait_for(lambda s: s.get('hidden') and s.get('tray'), 'Quiet login startup with tray')
                command('--disable')
                wait_for(lambda s: not s.get('eq_on', True), 'Bypass while hidden')
                command('--enable')
                command('--preset=2')
                wait_for(lambda s: s.get('eq_on') and s.get('preset') == 'Bass Boost', 'Preset and EQ enable while hidden')
                # Default launch must activate the existing process and immediately return.
                subprocess.run([str(BINARY)], env=env, check=True, timeout=6)
                wait_for(lambda s: not s.get('hidden', True), 'Repeated launcher opens existing instance')
                windows = subprocess.check_output(['wmctrl', '-lp'], text=True)
                window = next(line.split()[0] for line in windows.splitlines() if int(line.split()[2]) == p.pid)
                subprocess.run(['wmctrl', '-ic', window], check=True)
                wait_for(lambda s: s.get('hidden'), 'Window close keeps EQ and tray running')
                command('--show')
                wait_for(lambda s: not s.get('hidden', True), 'Tray-style reopen')
                command('--hide')
                wait_for(lambda s: s.get('hidden'), 'Hide command')
                # A failed tray must not strand an invisible app.
                children = subprocess.check_output(['ps', '-o', 'pid=,comm=', '--ppid', str(p.pid)], text=True)
                tray_pid = next(int(line.split()[0]) for line in children.splitlines() if 'python3' in line)
                os.kill(tray_pid, signal.SIGTERM)
                wait_for(lambda s: not s.get('hidden', True) and not s.get('tray', True), 'Tray failure restores accessible window')
                command('--quit')
                assert p.wait(timeout=10) == 0
                assert not (Path(tmp) / 'run/control.sock').exists()
                print('Explicit quit cleans up session: OK', flush=True)
                # A stale socket after a crash must not block the next launch.
                p = start()
                wait_for(lambda s: s.get('hidden') and s.get('tray'), 'Restart')
                p.kill()
                p.wait(timeout=5)
                p = start()
                wait_for(lambda s: s.get('hidden') and s.get('tray'), 'Recovery after a crashed instance')
                command('--quit')
                assert p.wait(timeout=10) == 0
                print('All desktop smoke checks passed.', flush=True)
            finally:
                for p in processes:
                    if p.poll() is None:
                        p.terminate()
                        try:
                            p.wait(timeout=8)
                        except subprocess.TimeoutExpired:
                            p.kill()
                            p.wait()


if __name__ == '__main__':
    main()
