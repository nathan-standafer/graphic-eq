# Installation and desktop integration

[Back to the README](../README.md)

From this checkout, run (as your normal user, without sudo):

    cargo fetch --manifest-path gpu/Cargo.toml --locked
    python3 packaging/install.py

The installer builds the Rust app and installs its executable, application-menu launcher,
icon and login autostart entry under `~/.local` and `~/.config/autostart`. Re-running it upgrades
the installed app. Close any older development copy before launching the installed app.
Rust and cached Cargo dependencies are needed to build; for a previously built release, use
`python3 packaging/install.py --skip-build`.

Runtime requirements: PipeWire, `pipewire-pulse`, `pactl`/`parec` (pulseaudio-utils), `pw-cli`,
`pw-dump`, a Vulkan-capable GPU, `/usr/bin/python3`, PyGObject, GTK 3 and XApp introspection
(`python3-gi`, `gir1.2-gtk-3.0`, `gir1.2-xapp-1.0`). The tray integration targets Cinnamon on
Linux Mint. These desktop dependencies are checked before installation; Cargo is not needed
by the installed app. The icon and tray bridge are embedded in the executable.

- Open **SpectraForgeEQ** from the application menu; its launcher can be pinned to the panel.
- Login startup opens it quietly in the system tray, restoring your saved gains, preset and EQ state.
  This starts at desktop login, not before login.
- Click the tray icon to open the window. Its right-click menu provides EQ enable/bypass,
  presets and Quit, with live status. Launching it again opens the existing instance.
- Closing the window hides it while the EQ continues. Visualization rendering and real-audio
  analysis pause while hidden. **Quit** stops the engine and restores normal playback routing.
- At login, the app waits up to 30 seconds for the desktop's tray to become available.
  If no tray is available after that, the app shows its window and closing it quits normally.
  If the tray helper exits or an established tray disappears, the window opens immediately.
- If PipeWire or an output device is not ready at login, the app retries every five seconds.

Existing settings stay at `~/.config/graphic-eq/gpu-settings.json` (or under `XDG_CONFIG_HOME`),
so upgrading from Graphic EQ preserves them. The EQ uses a virtual output named **SpectraForgeEQ**;
its internal PipeWire node name remains `geq_sink` for compatibility. Playback streams are routed
through the EQ, while the default output and volume keys stay on your real output device.

To disable login startup, use Linux Mint's **Startup Applications**, or reinstall with:

    python3 packaging/install.py --skip-build --no-autostart

To remove the installed app and autostart entry, keeping your settings:

    ~/.local/bin/spectraforgeeq --quit
    python3 packaging/install.py --uninstall

Command-line controls are available even with the window hidden:

    ~/.local/bin/spectraforgeeq --show
    ~/.local/bin/spectraforgeeq --hide
    ~/.local/bin/spectraforgeeq --enable          # --disable / --toggle also supported
    ~/.local/bin/spectraforgeeq --preset=2       # Bass Boost; see tray for all 10 presets
    ~/.local/bin/spectraforgeeq --status         # JSON: readiness, bypass, preset, tray, visibility

`--prefix` and `--config-home` let the installer stage files in another directory for packaging.
The default layout is a per-user installation; no root service is installed.

