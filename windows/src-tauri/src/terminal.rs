//! Real pseudo-terminal (PTY) backend for the Nouve Notching terminal panel.
//!
//! A `portable-pty` master/slave pair is created per session, a shell is spawned
//! into the slave end, and the master end is streamed back to the frontend over
//! Tauri events:
//!
//! * `terminal://output` -> `{ id, data }` (raw stdout/stderr chunks)
//! * `terminal://exit`   -> `{ id, code }` (shell terminated)
//!
//! Keystrokes travel the other direction through the `terminal_write` command.
//! `CommandBuilder::new` seeds the environment from the process env **and** the
//! HKLM/HKCU registry (merging the user PATH), which is what makes `agy`,
//! `codex`, `npm` and friends resolvable even when Nouve is launched from a
//! stale Explorer environment.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Mutex;

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

const DEFAULT_COLS: u16 = 100;
const DEFAULT_ROWS: u16 = 28;
const MIN_COLS: u16 = 20;
const MIN_ROWS: u16 = 5;

#[derive(Clone, Serialize)]
struct OutputPayload {
    id: String,
    data: String,
}

#[derive(Clone, Serialize)]
struct ExitPayload {
    id: String,
    code: u32,
}

/// Live resources for one terminal session.
struct Session {
    /// Keeps the ConPTY master alive for as long as the session exists.
    master: Box<dyn MasterPty + Send>,
    /// Write end feeding keystrokes into the shell.
    writer: Box<dyn Write + Send>,
    /// Detached handle used to terminate the shell on close.
    killer: Box<dyn ChildKiller + Send + Sync>,
}

/// Managed state holding every live terminal session keyed by frontend id.
#[derive(Default)]
pub struct TerminalState(pub(crate) Mutex<HashMap<String, Session>>);

/// Resolve an executable against the merged PATH exposed by `CommandBuilder`
/// (process env + registry), honouring `PATHEXT` exactly like Windows does.
fn search_path(probe: &CommandBuilder, exe: &str) -> Option<String> {
    let path = probe.get_env("PATH")?;
    let extensions = probe
        .get_env("PATHEXT")
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_string());
    let stem = Path::new(exe).file_stem()?.to_string_lossy().into_owned();
    for dir in std::env::split_paths(path) {
        let direct = dir.join(exe);
        if direct.is_file() {
            return Some(direct.to_string_lossy().into_owned());
        }
        for extension in extensions.split(';').filter(|entry| !entry.is_empty()) {
            let candidate = dir.join(format!("{stem}{extension}"));
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// Resolve the default shell for the Nouve terminal.
///
/// Nouve's autonomous CLIs (`agy`, `codex`, `opencode`, `oh-my-pi`) are shipped
/// as batch/cmd shims. They misbehave or fail to resolve under PowerShell, so the
/// terminal intentionally defaults to `cmd.exe` (via `ComSpec`). We never pick
/// pwsh/powershell here — the agent shell is a locked product decision.
fn resolve_shell(probe: &CommandBuilder) -> String {
    probe
        .get_env("ComSpec")
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.trim().is_empty())
        .or_else(|| search_path(probe, "cmd.exe"))
        .unwrap_or_else(|| "cmd.exe".to_string())
}

/// Start in the user's home directory when the caller did not request one.
fn default_working_dir() -> String {
    std::env::var("USERPROFILE")
        .ok()
        .filter(|path| Path::new(path).is_dir())
        .or_else(|| std::env::var("HOME").ok().filter(|path| Path::new(path).is_dir()))
        .unwrap_or_else(|| "C:\\".to_string())
}

#[tauri::command]
pub async fn terminal_open(
    app: AppHandle,
    state: State<'_, TerminalState>,
    id: String,
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<(), String> {
    if id.is_empty() {
        return Err("Sesi terminal butuh id".into());
    }
    // Re-opening the same id (e.g. React StrictMode double mount) is a no-op.
    {
        let sessions = state.0.lock().map_err(|_| "State terminal terkunci".to_string())?;
        if sessions.contains_key(&id) {
            return Ok(());
        }
    }

    let cols = cols.unwrap_or(DEFAULT_COLS).max(MIN_COLS);
    let rows = rows.unwrap_or(DEFAULT_ROWS).max(MIN_ROWS);

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|error| format!("Gagal membuat PTY: {error}"))?;

    // The probe loads the full merged environment (process + registry PATH).
    let probe = CommandBuilder::new("cmd.exe");
    let shell = resolve_shell(&probe);

    let mut command = CommandBuilder::new(shell.clone());
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    let working_dir = cwd
        .filter(|path| Path::new(path).is_dir())
        .unwrap_or_else(default_working_dir);
    command.cwd(working_dir);

    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| format!("Gagal menjalankan {shell}: {error}"))?;

    // The slave end is only needed for the spawn; dropping it is what lets the
    // shell receive EOF when the master closes.
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| format!("Gagal membaca PTY: {error}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| format!("Gagal menulis ke PTY: {error}"))?;
    let killer = child.clone_killer();

    // Stream stdout/stderr back to the webview on a dedicated thread. The child
    // handle is moved in so the exit code can be reported once the stream ends.
    let output_app = app.clone();
    let output_id = id.clone();
    std::thread::Builder::new()
        .name(format!("nouve-pty-{output_id}"))
        .spawn(move || {
            let mut reader = reader;
            let mut child = child;
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        let data = String::from_utf8_lossy(&buffer[..read]).into_owned();
                        let _ = output_app.emit("terminal://output", OutputPayload { id: output_id.clone(), data });
                    }
                    Err(_) => break,
                }
            }
            let code = child.wait().map(|status| status.exit_code()).unwrap_or(0);
            let _ = output_app.emit("terminal://exit", ExitPayload { id: output_id.clone(), code });
        })
        .map_err(|error| format!("Gagal memulai pembaca PTY: {error}"))?;

    let mut sessions = state.0.lock().map_err(|_| "State terminal terkunci".to_string())?;
    // A second concurrent open may have raced us; keep the first session.
    if sessions.contains_key(&id) {
        let mut killer = killer;
        let _ = killer.kill();
        return Ok(());
    }
    sessions.insert(id, Session { master: pair.master, writer, killer });
    Ok(())
}

#[tauri::command]
pub async fn terminal_write(state: State<'_, TerminalState>, id: String, data: String) -> Result<(), String> {
    let mut sessions = state.0.lock().map_err(|_| "State terminal terkunci".to_string())?;
    let session = sessions
        .get_mut(&id)
        .ok_or_else(|| "Sesi terminal sudah tidak aktif".to_string())?;
    session
        .writer
        .write_all(data.as_bytes())
        .map_err(|error| error.to_string())?;
    session.writer.flush().map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn terminal_resize(
    state: State<'_, TerminalState>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let sessions = state.0.lock().map_err(|_| "State terminal terkunci".to_string())?;
    let session = sessions
        .get(&id)
        .ok_or_else(|| "Sesi terminal sudah tidak aktif".to_string())?;
    session
        .master
        .resize(PtySize {
            rows: rows.max(MIN_ROWS),
            cols: cols.max(MIN_COLS),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn terminal_close(state: State<'_, TerminalState>, id: String) -> Result<(), String> {
    let removed = {
        let mut sessions = state.0.lock().map_err(|_| "State terminal terkunci".to_string())?;
        sessions.remove(&id)
    };
    if let Some(mut session) = removed {
        // Killing the shell closes the ConPTY, which unblocks the reader thread.
        let _ = session.killer.kill();
    }
    Ok(())
}

#[tauri::command]
pub async fn terminal_close_all(state: State<'_, TerminalState>) -> Result<(), String> {
    let drained = {
        let mut sessions = state.0.lock().map_err(|_| "State terminal terkunci".to_string())?;
        sessions.drain().map(|(_, session)| session).collect::<Vec<_>>()
    };
    for mut session in drained {
        let _ = session.killer.kill();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_resolution_falls_back_to_a_real_binary() {
        let probe = CommandBuilder::new("cmd.exe");
        let shell = resolve_shell(&probe);
        assert!(!shell.is_empty());
        assert!(Path::new(&shell).is_file() || !shell.contains('\\'));
    }

    #[test]
    fn working_directory_is_always_a_real_directory() {
        assert!(Path::new(&default_working_dir()).is_dir());
    }
}
