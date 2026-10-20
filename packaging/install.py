#!/usr/bin/env python3
"""Install SpectraForgeEQ for the current user (no sudo), or remove installed files."""
import argparse
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
APP_ID = 'spectraforgeeq'


def desktop_exec(path):
    # Desktop Entry quoting has two layers: string escapes, then Exec argument escapes.
    value = str(path).replace('%', '%%')
    for char in ['\\', '"', '`', '$']:
        value = value.replace(char, '\\' + char)
    return '"' + value.replace('\\', '\\\\') + '"'


def entry(launcher, background=False):
    return '\n'.join([
        '[Desktop Entry]', 'Type=Application', 'Version=1.0',
        'Name=SpectraForgeEQ', 'GenericName=System Equalizer',
        'Comment=11-band system equalizer and audio visualizer',
        f'Exec={desktop_exec(launcher)}' + (' --background' if background else ''),
        'Icon=spectraforgeeq', 'Terminal=false', 'Categories=AudioVideo;Audio;Mixer;',
        'Keywords=equalizer;audio;sound;music;visualizer;', 'StartupWMClass=spectraforgeeq',
        'StartupNotify=false',
        *(['X-GNOME-Autostart-enabled=true', 'X-GNOME-Autostart-Delay=3'] if background else []), '',
    ])


def atomic_write(path, data, mode=0o644):
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as tmp:
        name = Path(tmp.name)
        try:
            tmp.write(data)
            tmp.flush()
            name.chmod(mode)
            name.replace(path)
        finally:
            name.unlink(missing_ok=True)


def refresh(prefix):
    for command in [
        ['update-desktop-database', str(prefix / 'share/applications')],
        ['gtk-update-icon-cache', '-f', '-t', str(prefix / 'share/icons/hicolor')],
    ]:
        if shutil.which(command[0]):
            subprocess.run(command, check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def install(prefix, config, binary, autostart):
    launcher = prefix / 'bin/spectraforgeeq'
    executable = prefix / 'lib/spectraforgeeq/spectraforgeeq'
    files = {
        executable: (binary.read_bytes(), 0o755),
        launcher: (f'#!/bin/sh\nexec {shlex.quote(str(executable))} "$@"\n'.encode(), 0o755),
        prefix / 'share/applications/spectraforgeeq.desktop': (entry(launcher).encode(), 0o644),
        prefix / 'share/icons/hicolor/scalable/apps/spectraforgeeq.svg': ((ROOT / 'gpu/assets/spectraforgeeq.svg').read_bytes(), 0o644),
        prefix / 'share/icons/hicolor/256x256/apps/spectraforgeeq.png': ((ROOT / 'gpu/assets/spectraforgeeq.png').read_bytes(), 0o644),
    }
    if autostart:
        files[config / 'autostart/spectraforgeeq.desktop'] = (entry(launcher, True).encode(), 0o644)
    for path, (data, mode) in files.items():
        atomic_write(path, data, mode)
    if not autostart:
        (config / 'autostart/spectraforgeeq.desktop').unlink(missing_ok=True)
    refresh(prefix)
    print(f'Installed SpectraForgeEQ: {launcher}')
    print('Launch it from the application menu; pin its icon to the panel if desired.')
    print('Login startup: ' + ('enabled (starts hidden in the tray).' if autostart else 'disabled.'))
    print('Existing EQ settings are preserved in ~/.config/graphic-eq/gpu-settings.json.')


def uninstall(prefix, config):
    # Only the exact files this installer owns are removed; settings remain intact.
    for relative in [
        'bin/spectraforgeeq', 'lib/spectraforgeeq/spectraforgeeq',
        'share/applications/spectraforgeeq.desktop',
        'share/icons/hicolor/scalable/apps/spectraforgeeq.svg',
        'share/icons/hicolor/256x256/apps/spectraforgeeq.png',
    ]:
        (prefix / relative).unlink(missing_ok=True)
    (config / 'autostart/spectraforgeeq.desktop').unlink(missing_ok=True)
    refresh(prefix)
    print('Removed SpectraForgeEQ launcher, executable, icons, and login startup. Settings retained.')
    print('If the app is still running, use Quit in its tray menu.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prefix', type=Path, default=Path.home() / '.local')
    parser.add_argument('--config-home', type=Path, default=Path(os.environ.get('XDG_CONFIG_HOME', Path.home() / '.config')))
    parser.add_argument('--no-autostart', action='store_true')
    parser.add_argument('--skip-build', action='store_true', help='install the existing release build')
    parser.add_argument('--uninstall', action='store_true')
    args = parser.parse_args()
    prefix, config = args.prefix.expanduser().resolve(), args.config_home.expanduser().resolve()
    if args.uninstall:
        uninstall(prefix, config)
        return
    missing = [name for name in ['pipewire', 'pactl', 'pw-cli', 'pw-dump', 'parec'] if not shutil.which(name)]
    if missing:
        parser.error('Missing audio commands: ' + ', '.join(missing) + '. Install PipeWire and pulseaudio-utils first.')
    check = subprocess.run(['/usr/bin/python3', '-c', "import gi; gi.require_version('Gtk', '3.0'); gi.require_version('XApp', '1.0'); from gi.repository import Gtk, XApp"], capture_output=True)
    if check.returncode:
        parser.error('Tray support requires python3-gi, gir1.2-gtk-3.0 and gir1.2-xapp-1.0 (provided by Linux Mint).')
    if not args.skip_build:
        subprocess.run(['cargo', 'build', '--release', '--offline', '--manifest-path', str(ROOT / 'gpu/Cargo.toml')], check=True)
    binary = ROOT / 'gpu/target/release/geq-gpu'
    if not binary.is_file():
        parser.error('Release binary missing. Run without --skip-build to build it first.')
    install(prefix, config, binary, not args.no_autostart)


if __name__ == '__main__':
    main()
