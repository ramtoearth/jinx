use std::fs::OpenOptions;
use std::io::Write as _;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use jinx::config::{self as app_config};
use jinx::ipc::{
    AgentInitAckPayload, AgentInitPayload, AgentReplyPayload, Envelope, Kind, MessageType,
    ModelProvider, UserMessagePayload,
};
use jinx::ipc_handler;

use crate::state::*;

// ---------------------------------------------------------------------------
// Platform-aware log path
// ---------------------------------------------------------------------------

pub(crate) fn agent_log_path() -> std::path::PathBuf {
    std::env::temp_dir().join("tui_agent.log")
}

// ---------------------------------------------------------------------------
// Embedded Python agent — bundled at compile time so the binary is self-contained
// ---------------------------------------------------------------------------

const AGENT_PYPROJECT: &str = include_str!("../../pyproject.toml");
const AGENT_INIT:      &str = include_str!("../../agent/__init__.py");
const AGENT_IPC:       &str = include_str!("../../agent/ipc.py");
const AGENT_STORAGE:   &str = include_str!("../../agent/storage_tools.py");
const AGENT_MAIN:      &str = include_str!("../../agent/main.py");
const AGENT_LOCALE:    &str = include_str!("../../agent/locale.py");
const AGENT_LOCALE_EN: &str = include_str!("../../agent/locales/en.toml");
const AGENT_LOCALE_ES: &str = include_str!("../../agent/locales/es.toml");


/// Extract the embedded agent files to the OS data directory and return the
/// project root path (the directory that contains `pyproject.toml`).
///
/// Files are only written when their content has changed, so subsequent calls
/// are nearly free (a few `read_to_string` comparisons).
pub(crate) fn extract_agent() -> std::path::PathBuf {
    let data_dir = directories::ProjectDirs::from("", "", "jinx")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from(".jinx"));

    let pkg_dir = data_dir.join("agent"); // contains *.py modules

    let _ = std::fs::create_dir_all(&pkg_dir);

    let write_if_changed = |path: &std::path::Path, content: &str| {
        let needs_write = std::fs::read_to_string(path)
            .map(|existing| existing != content)
            .unwrap_or(true);
        if needs_write {
            if let Err(e) = std::fs::write(path, content) {
                eprintln!("[extract_agent] could not write {}: {e}", path.display());
            }
        }
    };

    write_if_changed(&data_dir.join("pyproject.toml"),  AGENT_PYPROJECT);
    write_if_changed(&pkg_dir.join("__init__.py"),       AGENT_INIT);
    write_if_changed(&pkg_dir.join("ipc.py"),            AGENT_IPC);
    write_if_changed(&pkg_dir.join("storage_tools.py"),  AGENT_STORAGE);
    write_if_changed(&pkg_dir.join("main.py"),           AGENT_MAIN);
    write_if_changed(&pkg_dir.join("locale.py"),         AGENT_LOCALE);

    let locale_dir = pkg_dir.join("locales");
    let _ = std::fs::create_dir_all(&locale_dir);
    write_if_changed(&locale_dir.join("en.toml"), AGENT_LOCALE_EN);
    write_if_changed(&locale_dir.join("es.toml"), AGENT_LOCALE_ES);


    data_dir
}

// ---------------------------------------------------------------------------
// Agent lifecycle
// ---------------------------------------------------------------------------

pub(crate) fn restart_agent(state: &mut RuntimeState) {
    send_shutdown(state);
    state.agent_stdin = None;
    state.agent_child = None;
    state.agent_rx = None;
    state.app.agent_alive = false;
    spawn_agent(state);
}

// ---------------------------------------------------------------------------
// Agent IPC
// ---------------------------------------------------------------------------

pub(crate) fn spawn_agent(state: &mut RuntimeState) {
    let agent_project = extract_agent();

    let log_path = agent_log_path();
    let agent_stderr = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null());

    let mut cmd = Command::new("uv");
    cmd.args([
            "run",
            "--project", agent_project.to_str().unwrap_or("."),
            "python", "-m", "agent.main",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(agent_stderr);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn()
        .unwrap_or_else(|e| {
            eprintln!("{}", state.locale.errors.agent_start.replace("{error}", &e.to_string()));
            eprintln!("Install uv: brew install uv  (or https://astral.sh/uv)");
            std::process::exit(1);
        });

    let mut stdin = child.stdin.take().expect("child stdin");
    let child_stdout = child.stdout.take().expect("child stdout");

    // Background reader thread: parses JSON lines and sends Envelopes over mpsc
    let (tx, rx) = mpsc::channel::<Envelope>();
    std::thread::spawn(move || {
        use std::io::BufRead;
        let mut log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(agent_log_path())
            .ok();
        let reader = std::io::BufReader::new(child_stdout);
        for line in reader.lines() {
            match line {
                Ok(line) if !line.trim().is_empty() => {
                    match serde_json::from_str::<Envelope>(&line) {
                        Ok(env) => {
                            if tx.send(env).is_err() {
                                break; // main thread dropped receiver — TUI is shutting down
                            }
                        }
                        Err(e) => {
                            if let Some(ref mut f) = log {
                                let _ = writeln!(f, "[agent reader] parse error: {e}: {line}");
                            }
                        }
                    }
                }
                Ok(_) => {} // blank line
                Err(e) => {
                    if let Some(ref mut f) = log {
                        let _ = writeln!(f, "[agent reader] read error: {e}");
                    }
                    break;
                }
            }
        }
    });

    // Send agent_init
    let cfg = app_config::load();
    let timezone = iana_timezone();
    let (model_provider, backend, model_id, host) = match cfg.provider {
        app_config::Provider::Local => (
            ModelProvider::Local,
            "ollama".to_string(),
            cfg.local.model,
            Some(cfg.local.host),
        ),
        app_config::Provider::Remote => {
            let (be, mid) = match cfg.remote.backend {
                app_config::RemoteBackend::Bedrock => ("bedrock", cfg.remote.bedrock_model),
                app_config::RemoteBackend::Openai => ("openai", cfg.remote.openai_model),
                app_config::RemoteBackend::Anthropic => ("anthropic", cfg.remote.anthropic_model),
                app_config::RemoteBackend::Gemini => ("gemini", cfg.remote.gemini_model),
                app_config::RemoteBackend::Llamaapi => ("llamaapi", cfg.remote.llamaapi_model),
            };
            (ModelProvider::Remote, be.to_string(), mid, None)
        }
    };
    let init_env = Envelope::new(
        Kind::Request,
        MessageType::AgentInit,
        &AgentInitPayload {
            timezone,
            language: cfg.language,
            model_provider,
            backend,
            model_id,
            host,
        },
    )
    .expect("agent_init serializes");
    let line = serde_json::to_string(&init_env).expect("serialize") + "\n";
    let _ = stdin.write_all(line.as_bytes());
    let _ = stdin.flush();

    state.agent_child = Some(child);
    state.agent_stdin = Some(stdin);
    state.agent_rx = Some(rx);
    state.app.agent_alive = true;
}

// ---------------------------------------------------------------------------
// Agent communication
// ---------------------------------------------------------------------------

pub(crate) fn send_user_message(state: &mut RuntimeState, text: String) {
    if let Some(ref mut stdin) = state.agent_stdin {
        let env = Envelope::new(
            Kind::Request,
            MessageType::UserMessage,
            &UserMessagePayload { text },
        )
        .expect("user_message serializes");
        let req_id = env.id;
        let line = serde_json::to_string(&env).expect("serialize") + "\n";
        if stdin.write_all(line.as_bytes()).is_ok() && stdin.flush().is_ok() {
            state.pending_request = Some((req_id, Instant::now()));
        } else {
            state.app.status_bar = state.locale.errors.agent_send.clone();
            state.app.agent_alive = false;
        }
    }
}

pub(crate) fn kill_process_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        unsafe { libc::kill(-pid, libc::SIGTERM); }
        std::thread::sleep(Duration::from_millis(100));
        if let Ok(Some(_)) = child.try_wait() { return; }
        unsafe { libc::kill(-pid, libc::SIGKILL); }
        let _ = child.wait();
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub(crate) fn send_shutdown(state: &mut RuntimeState) {
    if let Some(ref mut stdin) = state.agent_stdin {
        let env = Envelope::new_empty(Kind::Request, MessageType::Shutdown);
        let line = serde_json::to_string(&env).expect("serialize") + "\n";
        let _ = stdin.write_all(line.as_bytes());
        let _ = stdin.flush();
    }
    state.agent_stdin = None;

    if let Some(ref mut child) = state.agent_child {
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() >= deadline => {
                    kill_process_tree(child);
                    break;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(_) => break,
            }
        }
    }
    state.agent_child = None;
}

pub(crate) fn read_agent_output(state: &mut RuntimeState) {
    while let Some(env) = state.agent_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
        handle_agent_envelope(state, env);
    }
}

pub(crate) fn handle_agent_envelope(state: &mut RuntimeState, env: Envelope) {
    match env.message_type {
        MessageType::AgentInitAck => {
            if let Ok(Some(p)) = env.payload_as::<AgentInitAckPayload>() {
                if let Some(notice) = p.provider_notice {
                    state.chat_history.push(ChatMsg { role: ChatRole::System, text: notice, note_results: None });
                }
            }
        }
        MessageType::AgentReply => {
            if let Ok(Some(p)) = env.payload_as::<AgentReplyPayload>() {
                let note_results = state.last_note_results.take()
                    .filter(|v| !v.is_empty());
                let msg_idx = state.chat_history.len();
                state.chat_history.push(ChatMsg {
                    role: ChatRole::Agent,
                    text: p.text,
                    note_results: note_results.clone(),
                });
                if note_results.is_some() {
                    state.note_picker_active = true;
                    state.note_picker_cursor = 0;
                    state.note_picker_msg_idx = Some(msg_idx);
                }
                state.chat_scroll = 0;
            }
            state.pending_request = None;
            state.app.status_bar = state.locale.status.ready.clone();
        }
        mt if is_storage_message_type(mt) => {
            let response = ipc_handler::handle_storage_request(&env, &state.storage);

            // Capture note results for the interactive picker (only search, not list)
            if matches!(mt, MessageType::StorageSearchNotes) {
                if let Some(payload) = response.payload.as_ref() {
                    if let Some(notes_arr) = payload.get("notes").and_then(|v| v.as_array()) {
                        let entries: Vec<NotePickerEntry> = notes_arr.iter().filter_map(|n| {
                            Some(NotePickerEntry {
                                id: n.get("id")?.as_i64()?,
                                title: n.get("title")?.as_str()?.to_string(),
                                updated_at: n.get("updated_at")?.as_str()?.to_string(),
                            })
                        }).collect();
                        state.last_note_results = Some(entries);
                    }
                }
            }

            if let Some(ref mut stdin) = state.agent_stdin {
                if let Ok(line) = serde_json::to_string(&response) {
                    let _ = stdin.write_all(line.as_bytes());
                    let _ = stdin.write_all(b"\n");
                    let _ = stdin.flush();
                }
            }
        }
        _ => {}
    }
}

pub(crate) fn is_storage_message_type(mt: MessageType) -> bool {
    matches!(
        mt,
        MessageType::StorageListTasks
            | MessageType::StorageSearchTasks
            | MessageType::StorageCreateTask
            | MessageType::StorageUpdateTask
            | MessageType::StorageCompleteTask
            | MessageType::StorageDeleteTask
            | MessageType::StorageListEvents
            | MessageType::StorageCreateEvent
            | MessageType::StorageUpdateEvent
            | MessageType::StorageDeleteEvent
            | MessageType::StorageListGroups
            | MessageType::StorageCreateGroup
            | MessageType::StorageRenameGroup
            | MessageType::StorageRecolorGroup
            | MessageType::StorageDeleteGroup
            | MessageType::StorageExportMarkdown
            | MessageType::StorageExportSqlite
            | MessageType::StorageListNotes
            | MessageType::StorageSearchNotes
            | MessageType::StorageCreateNote
            | MessageType::StorageUpdateNote
            | MessageType::StorageDeleteNote
            | MessageType::StorageExportNote
    )
}

pub(crate) fn iana_timezone() -> String {
    if let Ok(tz) = std::env::var("TZ") {
        if !tz.is_empty() {
            return tz;
        }
    }
    if let Ok(target) = std::fs::read_link("/etc/localtime") {
        let path = target.to_string_lossy().to_string();
        if let Some(idx) = path.find("zoneinfo/") {
            return path[idx + 9..].to_string();
        }
    }
    "UTC".to_string()
}
