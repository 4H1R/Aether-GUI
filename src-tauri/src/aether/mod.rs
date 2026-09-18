pub mod orphan;
pub mod profiles;
pub mod prompts;
pub mod pty;
pub mod status;

use crate::error::AetherError;
use crate::events::{now_millis, LogEvent, LOG_EVENT, STATUS_EVENT};
use crate::state::ConnectionState;
use profiles::ConnectionProfile;
use pty::PtySession;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

pub struct AetherManager {
    session: Option<PtySession>,
    state: ConnectionState,
    user_requested_stop: bool,
    /// Consecutive auto-retry attempts for the current connection lineage.
    /// Reset to 0 on a fresh user-initiated connect, on reaching Connected
    /// (a proven-working connection earns a full retry budget for whatever
    /// drops it next), and on a user-requested disconnect.
    retry_count: u32,
    // Cancellation invalidates every monitor and delayed retry from that run.
    generation: u64,
}

impl AetherManager {
    pub fn new() -> Self {
        Self {
            session: None,
            state: ConnectionState::Idle,
            user_requested_stop: false,
            retry_count: 0,
            generation: 0,
        }
    }

    pub fn status(&self) -> ConnectionState {
        self.state.clone()
    }
}

fn app_data_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
}

fn resolve_binary(app: &AppHandle) -> Result<PathBuf, AetherError> {
    let dir = app
        .path()
        .resource_dir()
        .map_err(|e| AetherError::Internal(e.to_string()))?;
    let name = if cfg!(windows) {
        "aether.exe"
    } else {
        "aether"
    };
    let path = if cfg!(debug_assertions) {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("binaries/core")
            .join(name)
    } else {
        dir.join("binaries").join(name)
    };
    if !path.exists() {
        return Err(AetherError::BinaryMissing(path.display().to_string()));
    }
    // Bundlers don't reliably preserve the exec bit on resource files, and a
    // non-executable core binary would fail every spawn with a cryptic error.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
    }
    Ok(path)
}

/// Kicks off a connection attempt and returns as soon as Aether is spawned
/// (or a synchronous precondition fails: already running / port already
/// bound / binary missing). The actual Launching -> Connecting -> Connected
/// transitions happen on a background thread and reach the frontend via the
/// `aether://status` event, matching the IPC contract in the approved plan.
pub fn start_connect(
    app: AppHandle,
    manager: Arc<Mutex<AetherManager>>,
    profile_override: Option<ConnectionProfile>,
) -> Result<(), AetherError> {
    // Resolve everything fallible that doesn't touch AetherManager's state
    // first, so that once we transition to Launching below, the only
    // remaining failure mode is pty::spawn itself — which is handled
    // explicitly inside spawn_and_monitor rather than ever leaving the
    // state machine stuck in Launching with no process behind it.
    let profile = profile_override.unwrap_or_else(|| profiles::load(&app));
    profile.validate().map_err(AetherError::InvalidProfile)?;
    let binary = resolve_binary(&app)?;
    let data_dir = app_data_dir(&app);
    std::fs::create_dir_all(&data_dir).map_err(|e| AetherError::Internal(e.to_string()))?;

    let generation = {
        let mut mgr = manager.lock().unwrap();
        if !matches!(
            mgr.state,
            ConnectionState::Idle | ConnectionState::Error { .. }
        ) {
            return Err(AetherError::AlreadyRunning);
        }
        // Defensive guard independent of the pid-file mechanism in orphan.rs
        // (covers a manually-started Aether or a missing/corrupted pid file),
        // checked under the same lock as the state check above so a rapid
        // double-click can't race two connect() calls past this guard before
        // the first transitions to Launching.
        let socks = status::parse_bind_address(&profile.bind_address);
        if status::port_is_live(&socks) {
            return Err(AetherError::PortInUse(socks.port()));
        }
        if !profile.http_proxy.trim().is_empty() {
            let http = status::parse_bind_address(profile.http_proxy.trim());
            if status::port_is_live(&http) {
                return Err(AetherError::PortInUse(http.port()));
            }
        }
        mgr.state = ConnectionState::Launching;
        // A fresh user-initiated connect always gets a full retry budget,
        // independent of whatever happened on a previous, unrelated attempt.
        mgr.retry_count = 0;
        mgr.user_requested_stop = false;
        mgr.generation += 1;
        let _ = app.emit(STATUS_EVENT, &ConnectionState::Launching);
        mgr.generation
    };

    spawn_and_monitor(app, manager, binary, data_dir, profile, generation)
}

/// Spawns the PTY session and the log-forwarding + monitor threads. Shared
/// by the initial user-initiated connect and by `handle_unexpected_failure`'s
/// auto-retry — both start from the same place (a fresh PTY, `Launching`
/// already set by the caller) and only differ in what led here.
fn spawn_and_monitor(
    app: AppHandle,
    manager: Arc<Mutex<AetherManager>>,
    binary: PathBuf,
    data_dir: PathBuf,
    profile: ConnectionProfile,
    generation: u64,
) -> Result<(), AetherError> {
    let mut mgr = manager.lock().unwrap();
    if mgr.generation != generation || mgr.user_requested_stop {
        return Ok(());
    }
    let (log_tx, log_rx) = mpsc::channel::<LogEvent>();
    let session_or_err = pty::spawn(&binary, &data_dir, profile.clone(), log_tx);
    let session = match session_or_err {
        Ok(session) => session,
        Err(e) => {
            // Must not leave the state machine stuck in Launching with no
            // process behind it. A spawn failure is an OS/environment-level
            // problem (not a network drop), so it is not auto-retried —
            // retrying blindly here would just mask a real setup issue.
            mgr.state = ConnectionState::Error {
                message: e.to_string(),
                phase: "launching".into(),
            };
            let _ = app.emit(STATUS_EVENT, &mgr.state);
            return Err(e);
        }
    };
    orphan::write_pid(&data_dir, session.pid());

    mgr.session = Some(session);
    mgr.state = ConnectionState::Connecting;
    let _ = app.emit(STATUS_EVENT, &ConnectionState::Connecting);
    drop(mgr);

    // Forward every log line to the frontend's advanced/log panel as it
    // arrives, independent of whether status classification succeeds.
    {
        let app_for_logs = app.clone();
        let manager_for_logs = Arc::clone(&manager);
        std::thread::spawn(move || {
            for log in log_rx {
                let mgr = manager_for_logs.lock().unwrap();
                if mgr.generation != generation {
                    break;
                }
                let _ = app_for_logs.emit(LOG_EVENT, &log);
            }
        });
    }

    {
        let app = app.clone();
        let manager = Arc::clone(&manager);
        let binary = binary.clone();
        let data_dir = data_dir.clone();
        std::thread::spawn(move || {
            monitor_connect(app, manager, binary, data_dir, profile, generation)
        });
    }

    Ok(())
}

/// Common landing spot for every unexpected failure (process exit before
/// connecting, scan timeout, or process exit after being connected) that
/// was NOT a user-requested disconnect. Retries with backoff up to
/// `status::MAX_AUTO_RETRIES` before giving up with a real `Error` — this
/// is what turns a mid-session drop (the "stops all of a sudden" case,
/// worst on gool since it's two nested tunnels, but not exclusive to it)
/// into a brief, visible "Reconnecting" instead of dumping the user back to
/// Idle every time.
fn handle_unexpected_failure(
    app: AppHandle,
    manager: Arc<Mutex<AetherManager>>,
    binary: PathBuf,
    data_dir: PathBuf,
    profile: ConnectionProfile,
    failure_message: String,
    phase: &'static str,
    generation: u64,
) {
    let attempt = {
        let mut mgr = manager.lock().unwrap();
        if mgr.user_requested_stop || mgr.generation != generation {
            // request_disconnect is already handling this exit; don't race
            // it with a retry or an Error state it didn't ask for.
            return;
        }
        mgr.session = None;
        mgr.retry_count += 1;
        orphan::clear_pid(&data_dir);
        let attempt = mgr.retry_count;
        mgr.state = if attempt > status::MAX_AUTO_RETRIES {
            ConnectionState::Error {
                message: format!(
                    "{failure_message} (gave up after {} retries)",
                    status::MAX_AUTO_RETRIES
                ),
                phase: phase.into(),
            }
        } else {
            ConnectionState::Reconnecting {
                attempt,
                max_attempts: status::MAX_AUTO_RETRIES,
            }
        };
        let _ = app.emit(STATUS_EVENT, &mgr.state);
        attempt
    };
    if attempt > status::MAX_AUTO_RETRIES {
        return;
    }

    let backoff = status::RETRY_BACKOFF[(attempt - 1) as usize];
    std::thread::spawn(move || {
        std::thread::sleep(backoff);
        {
            let mgr = manager.lock().unwrap();
            if mgr.user_requested_stop || mgr.generation != generation {
                return;
            }
        }
        // spawn_and_monitor already lands its own failure in Error/retry —
        // nothing further to do with its Result here.
        let _ = spawn_and_monitor(app, manager, binary, data_dir, profile, generation);
    });
}

fn monitor_connect(
    app: AppHandle,
    manager: Arc<Mutex<AetherManager>>,
    binary: PathBuf,
    data_dir: PathBuf,
    profile: ConnectionProfile,
    generation: u64,
) {
    let hops = if matches!(
        profile.protocol,
        profiles::Protocol::Mim | profiles::Protocol::Gool
    ) {
        2
    } else {
        1
    };
    let deadline = Instant::now() + status::connect_timeout(&profile.scan_mode) * hops;
    let socks = status::parse_bind_address(&profile.bind_address);

    loop {
        std::thread::sleep(Duration::from_millis(400));
        let mut mgr = manager.lock().unwrap();
        if mgr.user_requested_stop || mgr.generation != generation {
            return;
        }

        if let Some(exit) = mgr.session.as_mut().and_then(|s| s.try_wait()) {
            mgr.session = None;
            drop(mgr);
            handle_unexpected_failure(
                app,
                manager,
                binary,
                data_dir,
                profile,
                format!("Aether exited before connecting ({exit})"),
                "connecting",
                generation,
            );
            return;
        }

        if status::socks_is_ready(&socks) {
            let new_state = ConnectionState::Connected {
                socks_addr: profile.bind_address.clone(),
                connected_at_ms: now_millis(),
            };
            mgr.state = new_state.clone();
            // Proven working — a future drop earns a fresh full retry budget
            // rather than inheriting whatever it took to get here.
            mgr.retry_count = 0;
            let _ = app.emit(STATUS_EVENT, &new_state);
            // Only persisted as "last successful" once actually proven to
            // work, never on a mere attempt (see profiles::save's doc-comment).
            profiles::save(&app, &profile);
            drop(mgr);
            monitor_connected(app, manager, binary, data_dir, profile, generation);
            return;
        }

        if Instant::now() >= deadline {
            if let Some(session) = mgr.session.as_mut() {
                session.kill();
            }
            mgr.session = None;
            drop(mgr);
            handle_unexpected_failure(
                app,
                manager,
                binary,
                data_dir,
                profile,
                "Timed out waiting for Aether to find a working route".into(),
                "connecting",
                generation,
            );
            return;
        }
    }
}

/// Watches an established connection purely for an unexpected process exit —
/// there is no polling needed beyond that once `Connected` is reached.
fn monitor_connected(
    app: AppHandle,
    manager: Arc<Mutex<AetherManager>>,
    binary: PathBuf,
    data_dir: PathBuf,
    profile: ConnectionProfile,
    generation: u64,
) {
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let mut mgr = manager.lock().unwrap();
        if mgr.user_requested_stop || mgr.generation != generation {
            return;
        }
        if let Some(exit) = mgr.session.as_mut().and_then(|s| s.try_wait()) {
            mgr.session = None;
            drop(mgr);
            handle_unexpected_failure(
                app,
                manager,
                binary,
                data_dir,
                profile,
                format!("Lost connection unexpectedly ({exit})"),
                "connected",
                generation,
            );
            return;
        }
    }
}

pub fn request_disconnect(
    app: &AppHandle,
    manager: &Arc<Mutex<AetherManager>>,
) -> Result<(), AetherError> {
    let had_session = {
        let mut mgr = manager.lock().unwrap();
        if matches!(mgr.state, ConnectionState::Disconnecting) {
            return Ok(());
        }
        // Reconnecting has no live session (the old one already exited; the
        // retry's replacement hasn't spawned yet) — still a valid thing to
        // cancel, it just means there's nothing to send Ctrl-C to.
        let reconnecting = matches!(
            mgr.state,
            ConnectionState::Reconnecting { .. } | ConnectionState::Launching
        );
        if mgr.session.is_none() && !reconnecting {
            return Err(AetherError::NotConnected);
        }
        mgr.user_requested_stop = true;
        mgr.generation += 1;
        mgr.retry_count = 0;
        if let Some(session) = mgr.session.as_ref() {
            session.send_ctrl_c();
        }
        let had_session = mgr.session.is_some();
        mgr.state = if had_session {
            ConnectionState::Disconnecting
        } else {
            ConnectionState::Idle
        };
        let _ = app.emit(STATUS_EVENT, &mgr.state);
        had_session
    };

    if !had_session {
        // Mid-backoff: the retry thread checks user_requested_stop (just set
        // above) before respawning, so setting the flag is enough — there is
        // no process to wait on, so reflect Idle immediately.
        return Ok(());
    }

    let app = app.clone();
    let manager = Arc::clone(manager);
    std::thread::spawn(move || {
        let deadline = Instant::now() + status::GRACEFUL_SHUTDOWN_GRACE;
        loop {
            std::thread::sleep(Duration::from_millis(200));
            let mut mgr = manager.lock().unwrap();
            let exited = mgr.session.as_mut().and_then(|s| s.try_wait()).is_some();
            if exited || Instant::now() >= deadline {
                if !exited {
                    if let Some(session) = mgr.session.as_mut() {
                        session.kill();
                    }
                }
                mgr.session = None;
                mgr.user_requested_stop = false;
                orphan::clear_pid(&app_data_dir(&app));
                mgr.state = ConnectionState::Idle;
                let _ = app.emit(STATUS_EVENT, &mgr.state);
                return;
            }
        }
    });

    Ok(())
}

/// Supplies the Cloudflare Access one-time code requested by Aether 1.5.0
/// during a Zero Trust email enrolment. It is deliberately a narrow command
/// instead of a generic PTY write endpoint, so the webview can never inject
/// arbitrary terminal input into the bundled core.
pub fn submit_access_code(
    manager: &Arc<Mutex<AetherManager>>,
    code: String,
) -> Result<(), AetherError> {
    let manager = manager
        .lock()
        .map_err(|_| AetherError::Internal("Aether state is unavailable".into()))?;
    let session = manager.session.as_ref().ok_or(AetherError::NotConnected)?;
    session.send_access_code(&code)
}

/// Called from `RunEvent::Exit` — the app is quitting regardless, so this
/// blocks briefly rather than spawning a thread, and skips emitting events
/// nobody is left to receive.
pub fn shutdown_blocking(manager: &Arc<Mutex<AetherManager>>, data_dir: &Path) {
    let mut mgr = manager.lock().unwrap();
    mgr.user_requested_stop = true;
    mgr.generation += 1;
    if let Some(session) = mgr.session.as_mut() {
        session.send_ctrl_c();
        std::thread::sleep(Duration::from_millis(500));
        session.kill();
    }
    mgr.session = None;
    drop(mgr);
    orphan::clear_pid(data_dir);
}
