//! Per-user single instance, local control socket, and Cinnamon tray bridge.
use eframe::egui;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

pub const NAME: &str = "SpectraForgeEQ";
pub const ICON: &[u8] = include_bytes!("../assets/spectraforgeeq.png");

#[derive(Debug, PartialEq)]
pub enum Action {
    Show, Hide, Quit, Toggle, Enable(bool), Preset(usize), Tray(bool), TrayExited,
}

// Cinnamon's panel can register after the application during login.
const TRAY_STARTUP_GRACE: Duration = Duration::from_secs(30);

pub struct TrayState {
    ready: bool,
    was_ready: bool,
    exited: bool,
    started: Instant,
}

impl TrayState {
    fn new(started: Instant, exited: bool) -> Self {
        Self { ready: false, was_ready: false, exited, started }
    }

    pub fn ready(&self) -> bool { self.ready }

    pub fn update(&mut self, ready: bool) {
        self.ready = ready;
        self.was_ready |= ready;
    }

    pub fn exited(&mut self) {
        self.ready = false;
        self.exited = true;
    }

    pub fn needs_window(&self, now: Instant) -> bool {
        !self.ready && (self.exited || self.was_ready || now.duration_since(self.started) >= TRAY_STARTUP_GRACE)
    }
}

pub fn parse(s: &str) -> Option<Action> {
    Some(match s {
        "show" => Action::Show,
        "hide" => Action::Hide,
        "quit" => Action::Quit,
        "toggle" => Action::Toggle,
        "on" => Action::Enable(true),
        "off" => Action::Enable(false),
        "tray-ready" => Action::Tray(true),
        "tray-unavailable" => Action::Tray(false),
        _ => Action::Preset(s.strip_prefix("preset ")?.parse::<usize>().ok().filter(|i| *i < crate::settings::PRESETS.len())?),
    })
}

fn directory() -> io::Result<PathBuf> {
    let base = std::env::var_os("SPECTRAFORGE_RUNTIME_DIR").map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_RUNTIME_DIR").map(|p| PathBuf::from(p).join("spectraforgeeq")))
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache/spectraforgeeq/run")))
        .ok_or_else(|| io::Error::other("Neither XDG_RUNTIME_DIR nor HOME is set"))?;
    fs::DirBuilder::new().recursive(true).mode(0o700).create(&base)?;
    if fs::symlink_metadata(&base)?.file_type().is_symlink() {
        return Err(io::Error::other("The session directory must not be a symlink"));
    }
    fs::set_permissions(&base, fs::Permissions::from_mode(0o700))?;
    Ok(base)
}

/// The OS releases this lock after a crash; the next owner can remove a stale socket safely.
pub struct Instance {
    _lock: File,
    listener: UnixListener,
    path: PathBuf,
}

impl Instance {
    pub fn acquire() -> io::Result<Option<Self>> {
        let dir = directory()?;
        let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600).open(dir.join("instance.lock"))?;
        match lock.try_lock() {
            Ok(()) => {},
            Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
            Err(std::fs::TryLockError::Error(e)) => return Err(e),
        }
        let path = dir.join("control.sock");
        match fs::remove_file(&path) {
            Ok(()) => {},
            Err(e) if e.kind() == io::ErrorKind::NotFound => {},
            Err(e) => return Err(e),
        }
        let listener = UnixListener::bind(&path)?;
        Ok(Some(Self { _lock: lock, listener, path }))
    }
}

impl Drop for Instance {
    fn drop(&mut self) { let _ = fs::remove_file(&self.path); }
}

pub fn send(command: &str) -> io::Result<String> {
    let mut stream = UnixStream::connect(directory()?.join("control.sock"))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    writeln!(stream, "{command}")?;
    let mut reply = String::new();
    BufReader::new(stream).take(8192).read_line(&mut reply)?;
    Ok(reply.trim().to_owned())
}

pub struct Desktop {
    // Drop the tray before releasing the single-instance lock.
    tray: Option<Child>,
    tray_input: Option<ChildStdin>,
    rx: mpsc::Receiver<Action>,
    status: Arc<Mutex<String>>,
    last_status: String,
    pub tray_state: TrayState,
    _instance: Instance,
}

impl Desktop {
    pub fn new(instance: Instance, ctx: egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(String::from("{\"status\":\"Starting\"}")));
        let listener = instance.listener.try_clone().expect("clone control listener");
        let (events, wake, state) = (tx.clone(), ctx.clone(), status.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
                let mut line = String::new();
                if BufReader::new(&stream).take(1024).read_line(&mut line).is_err() { continue; }
                let line = line.trim();
                let reply = if line == "status" {
                    state.lock().unwrap().clone()
                } else if line == "ping" {
                    "ok".into()
                } else if let Some(action) = parse(line).filter(|a| !matches!(a, Action::Tray(_))) {
                    if events.send(action).is_err() { break; }
                    wake.request_repaint();
                    "ok".into()
                } else { "error: unknown command".into() };
                let _ = writeln!(stream, "{reply}");
            }
        });
        let icon = instance.path.with_file_name("icon.png");
        let _ = fs::write(&icon, ICON);
        let mut tray = Command::new("/usr/bin/python3")
            .args(["-u", "-c", include_str!("../tools/tray.py")]).arg(&icon)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn()
            .map_err(|e| eprintln!("Tray unavailable: {e}")).ok();
        let tray_input = tray.as_mut().and_then(|c| c.stdin.take());
        if let Some(out) = tray.as_mut().and_then(|c| c.stdout.take()) {
            std::thread::spawn(move || {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    if let Some(a) = parse(&line) {
                        if tx.send(a).is_err() { return; }
                        ctx.request_repaint();
                    }
                }
                let _ = tx.send(Action::TrayExited);
                ctx.request_repaint();
            });
        }
        let tray_state = TrayState::new(Instant::now(), tray.is_none());
        Self { tray, tray_input, rx, status, last_status: String::new(), tray_state, _instance: instance }
    }

    pub fn actions(&self) -> Vec<Action> { self.rx.try_iter().collect() }

    pub fn publish(&mut self, cfg: &crate::settings::Settings, ready: bool, status: &str, hidden: bool) {
        let state = serde_json::json!({
            "eq_on": cfg.eq_on, "preset": cfg.preset, "ready": ready, "status": status,
            "hidden": hidden, "tray": self.tray_state.ready(),
            "presets": crate::settings::PRESETS.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        }).to_string();
        if state != self.last_status {
            *self.status.lock().unwrap() = state.clone();
            if let Some(input) = &mut self.tray_input {
                if writeln!(input, "{state}").is_err() { self.tray_input = None; }
            }
            self.last_status = state;
        }
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        self.tray_input = None;
        if let Some(mut child) = self.tray.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_panel_keeps_background_startup_hidden() {
        let start = Instant::now();
        let mut tray = TrayState::new(start, false);
        tray.update(false); // XApp initially reports NO_SUPPORT while Cinnamon starts.
        assert!(!tray.needs_window(start + Duration::from_millis(1500)));
        assert!(!tray.needs_window(start + Duration::from_secs(10)));
        tray.update(true);
        assert!(tray.ready());
        assert!(!tray.needs_window(start + Duration::from_secs(60)));
    }

    #[test]
    fn missing_or_failed_tray_keeps_window_accessible() {
        let start = Instant::now();
        let mut tray = TrayState::new(start, false);
        assert!(tray.needs_window(start + TRAY_STARTUP_GRACE));
        tray.update(true);
        tray.update(false);
        assert!(tray.needs_window(start + Duration::from_secs(2)));

        let mut tray = TrayState::new(start, false);
        tray.exited();
        assert!(tray.needs_window(start));
        assert!(TrayState::new(start, true).needs_window(start));
    }
    #[test]
    fn controls_reject_invalid_commands_and_out_of_range_presets() {
        assert_eq!(parse("on"), Some(Action::Enable(true)));
        assert_eq!(parse("preset 9"), Some(Action::Preset(9)));
        for invalid in ["preset 10", "preset -1", "preset xyz", "quit now", ""] { assert_eq!(parse(invalid), None); }
    }
}
