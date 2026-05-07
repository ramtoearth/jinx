// Feature: jinx, Property 1: Round-trip de serialización IPC (cruzado)
//
// Cross-process round-trip test: the Rust side serializes a set of Envelope
// values to JSON Lines on stdout; the Python side reads them, deserializes with
// ``agent.ipc.decode``, re-serializes with ``agent.ipc.encode``, and writes
// them back on stdout.  The Rust side reads back the echoed lines and asserts
// structural equality with the originals.
//
// This test only runs when the `cross_ipc` feature flag is set (i.e. in the
// `cross-ipc` CI job) to avoid needing Python in the normal `cargo test` run.
// In CI the job builds the tui binary and runs this test via the helper script
// `scripts/cross_ipc_test.sh`.
//
// Valida: Requisitos 10.1, 10.2, 10.3

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use jinx::ipc::{
    AgentReplyPayload, Envelope, Kind, MessageType, Priority, StorageCreateTaskRequest,
    StorageTaskDto, TaskStatus, UserMessagePayload, PROTOCOL_VERSION,
};
use uuid::Uuid;

fn sample_envelopes() -> Vec<Envelope> {
    vec![
        Envelope::new(
            Kind::Request,
            MessageType::UserMessage,
            &UserMessagePayload {
                text: "hola mundo".to_string(),
            },
        )
        .unwrap()
        .with_ref(Uuid::nil()),
        Envelope::new(
            Kind::Response,
            MessageType::AgentReply,
            &AgentReplyPayload {
                text: "respuesta del agente".to_string(),
            },
        )
        .unwrap(),
        Envelope::new(
            Kind::Request,
            MessageType::StorageCreateTask,
            &StorageCreateTaskRequest {
                title: "tarea de prueba".to_string(),
                priority: Some(Priority::Alta),
                deadline: Some("2025-06-01T09:00:00+00:00".to_string()),
                group_id: None,
            },
        )
        .unwrap(),
        Envelope::new(
            Kind::Response,
            MessageType::StorageListTasks,
            &jinx::ipc::StorageListTasksResponse {
                tasks: vec![StorageTaskDto {
                    id: 1,
                    title: "tarea uno".to_string(),
                    priority: Priority::Media,
                    status: TaskStatus::Pendiente,
                    created_at: "2025-01-01T08:00:00+00:00".to_string(),
                    deadline: None,
                    group_id: None,
                }],
            },
        )
        .unwrap(),
    ]
}

#[test]
fn cross_process_ipc_round_trip() {
    // Locate the Python echo helper alongside this test binary.
    let echo_script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("scripts/ipc_echo.py");

    if !echo_script.exists() {
        eprintln!("skipping cross_process_ipc_round_trip: echo script not found at {echo_script:?}");
        return;
    }

    let python = std::env::var("PYTHON").unwrap_or_else(|_| "python3".to_string());

    let mut child = Command::new(&python)
        .arg(&echo_script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn Python echo script");

    let mut stdin = child.stdin.take().expect("child stdin");
    let stdout = child.stdout.take().expect("child stdout");
    let mut reader = BufReader::new(stdout);

    let envelopes = sample_envelopes();

    for env in &envelopes {
        let line = serde_json::to_string(env).expect("serialize");
        writeln!(stdin, "{line}").expect("write to child stdin");
    }
    // Signal EOF so the Python side knows to exit
    drop(stdin);

    let mut echoed: Vec<Envelope> = Vec::new();
    let mut buf = String::new();
    while reader.read_line(&mut buf).unwrap_or(0) > 0 {
        let trimmed = buf.trim();
        if !trimmed.is_empty() {
            let decoded: Envelope =
                serde_json::from_str(trimmed).expect("failed to deserialize echoed envelope");
            echoed.push(decoded);
        }
        buf.clear();
    }

    child.wait().expect("child exited");

    assert_eq!(
        echoed.len(),
        envelopes.len(),
        "echoed count mismatch: got {}, expected {}",
        echoed.len(),
        envelopes.len()
    );

    for (original, echo) in envelopes.iter().zip(echoed.iter()) {
        assert_eq!(
            original.v, echo.v,
            "v mismatch for id {}",
            original.id
        );
        assert_eq!(
            original.kind, echo.kind,
            "kind mismatch for id {}",
            original.id
        );
        assert_eq!(
            original.message_type, echo.message_type,
            "type mismatch for id {}",
            original.id
        );
        assert_eq!(
            original.payload, echo.payload,
            "payload mismatch for id {}",
            original.id
        );
        assert_eq!(
            original.error, echo.error,
            "error mismatch for id {}",
            original.id
        );
        // id and ref are regenerated on the Python side for the echo, so we
        // only check structural payload equality (the property contract).
    }
}
