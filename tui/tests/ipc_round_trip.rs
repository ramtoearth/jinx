// Feature: jinx, Property 1: Round-trip de serialización IPC
//
// Property-based test that exercises the whole Canal_IPC envelope surface:
// for any well-formed `Envelope`, `from_json(to_json(env)) == env` under the
// equality contract defined in `design.md` → "Contrato de igualdad para
// round-trip IPC" (same `v`, `id`, `kind`, `type`, `ref`, structural `payload`
// equality, same `error` or both absent).
//
// The derived `PartialEq` on `Envelope` already implements that contract:
// `Option<serde_json::Value>` compares structurally and all scalar fields
// compare by value.
//
// Strategies cover:
//   * every `Kind` and every `MessageType` variant;
//   * typed payloads for a representative subset of chat/lifecycle and
//     storage messages (user_message, agent_reply, agent_init, agent_init_ack,
//     agent_tool_progress, shutdown, storage.create_task req/resp,
//     storage.list_tasks req/resp, storage.create_event req/resp,
//     storage.create_group req/resp, storage.export_markdown req/resp);
//   * envelopes with no payload (e.g. `shutdown` requests or empty responses);
//   * `ref_id` either `Some(Uuid)` or `None`;
//   * `error` either `Some(StorageError)` or `None`; when `error` is present
//     the payload is forced to `None`, matching the wire invariant.

use proptest::option;
use proptest::prelude::*;
use serde_json::Value;
use jinx::ipc::{
    AgentInitAckPayload, AgentInitPayload, AgentReplyPayload, AgentToolProgressPayload, Envelope,
    Kind, MessageType, ModelProvider, Priority, ShutdownPayload, StorageCreateEventRequest,
    StorageCreateEventResponse, StorageCreateGroupRequest, StorageCreateGroupResponse,
    StorageCreateTaskRequest, StorageCreateTaskResponse, StorageError, StorageEventDto,
    StorageExportMarkdownRequest, StorageExportMarkdownResponse, StorageGroupDto,
    StorageListTasksRequest, StorageListTasksResponse, StorageTaskDto, TaskStatus, ToolProgressPhase,
    UserMessagePayload, PROTOCOL_VERSION,
};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Primitive strategies
// ---------------------------------------------------------------------------

/// Build an RFC 4122 v4 UUID from 16 random bytes. Using `Builder` instead of
/// `Uuid::new_v4` keeps the values deterministic under proptest's rng so
/// shrinking and replay work as expected.
fn arb_uuid() -> impl Strategy<Value = Uuid> {
    any::<[u8; 16]>().prop_map(|bytes| uuid::Builder::from_random_bytes(bytes).into_uuid())
}

fn arb_kind() -> impl Strategy<Value = Kind> {
    prop_oneof![
        Just(Kind::Request),
        Just(Kind::Response),
        Just(Kind::Event),
    ]
}

fn arb_message_type_chat() -> impl Strategy<Value = MessageType> {
    prop_oneof![
        Just(MessageType::UserMessage),
        Just(MessageType::AgentReply),
        Just(MessageType::AgentInit),
        Just(MessageType::AgentInitAck),
        Just(MessageType::Shutdown),
        Just(MessageType::AgentToolProgress),
    ]
}

fn arb_message_type_tasks() -> impl Strategy<Value = MessageType> {
    prop_oneof![
        Just(MessageType::StorageListTasks),
        Just(MessageType::StorageCreateTask),
        Just(MessageType::StorageUpdateTask),
        Just(MessageType::StorageCompleteTask),
        Just(MessageType::StorageDeleteTask),
    ]
}

fn arb_message_type_events() -> impl Strategy<Value = MessageType> {
    prop_oneof![
        Just(MessageType::StorageListEvents),
        Just(MessageType::StorageCreateEvent),
        Just(MessageType::StorageUpdateEvent),
        Just(MessageType::StorageDeleteEvent),
    ]
}

fn arb_message_type_groups() -> impl Strategy<Value = MessageType> {
    prop_oneof![
        Just(MessageType::StorageListGroups),
        Just(MessageType::StorageCreateGroup),
        Just(MessageType::StorageRenameGroup),
        Just(MessageType::StorageRecolorGroup),
        Just(MessageType::StorageDeleteGroup),
    ]
}

fn arb_message_type_export() -> impl Strategy<Value = MessageType> {
    prop_oneof![
        Just(MessageType::StorageExportMarkdown),
        Just(MessageType::StorageExportSqlite),
    ]
}

/// Every `MessageType` variant has the same probability of being picked.
fn arb_message_type() -> impl Strategy<Value = MessageType> {
    prop_oneof![
        arb_message_type_chat(),
        arb_message_type_tasks(),
        arb_message_type_events(),
        arb_message_type_groups(),
        arb_message_type_export(),
    ]
}

/// Short free-text string covering letters, digits, punctuation and
/// whitespace from any Unicode block. Non-empty on its own isn't required:
/// empty strings are also valid JSON strings and must round-trip.
fn arb_text() -> impl Strategy<Value = String> {
    r"[\p{L}\p{N}\p{P}\p{Zs}]{0,64}".prop_map(|s| s)
}

/// Plausible ISO 8601 timestamp with timezone offset (e.g.
/// `2025-02-14T10:12:00+01:00`). The IPC layer treats these as opaque
/// strings, but generating realistic shapes keeps counterexamples readable.
fn arb_iso_ts() -> impl Strategy<Value = String> {
    r"2[0-9]{3}-[01][0-9]-[0-3][0-9]T[0-2][0-9]:[0-5][0-9]:[0-5][0-9][+-][01][0-9]:[0-5][0-9]"
        .prop_map(|s| s)
}

fn arb_date() -> impl Strategy<Value = String> {
    r"2[0-9]{3}-[01][0-9]-[0-3][0-9]".prop_map(|s| s)
}

fn arb_time() -> impl Strategy<Value = String> {
    r"[0-2][0-9]:[0-5][0-9]".prop_map(|s| s)
}

fn arb_hex_color() -> impl Strategy<Value = String> {
    r"#[0-9a-fA-F]{6}".prop_map(|s| s)
}

fn arb_priority() -> impl Strategy<Value = Priority> {
    prop_oneof![
        Just(Priority::Alta),
        Just(Priority::Media),
        Just(Priority::Baja),
    ]
}

fn arb_task_status() -> impl Strategy<Value = TaskStatus> {
    prop_oneof![
        Just(TaskStatus::Pendiente),
        Just(TaskStatus::Completada),
        Just(TaskStatus::Cancelada),
    ]
}

fn arb_model_provider() -> impl Strategy<Value = ModelProvider> {
    prop_oneof![Just(ModelProvider::Local), Just(ModelProvider::Remote)]
}

fn arb_tool_progress_phase() -> impl Strategy<Value = ToolProgressPhase> {
    prop_oneof![Just(ToolProgressPhase::Start), Just(ToolProgressPhase::End)]
}

// ---------------------------------------------------------------------------
// DTO strategies
// ---------------------------------------------------------------------------

fn arb_task_dto() -> impl Strategy<Value = StorageTaskDto> {
    (
        any::<i64>(),
        arb_text(),
        arb_priority(),
        arb_task_status(),
        arb_iso_ts(),
        option::of(arb_iso_ts()),
        option::of(any::<i64>()),
    )
        .prop_map(
            |(id, title, priority, status, created_at, deadline, group_id)| StorageTaskDto {
                id,
                title,
                priority,
                status,
                created_at,
                deadline,
                group_id,
            },
        )
}

fn arb_event_dto() -> impl Strategy<Value = StorageEventDto> {
    (
        any::<i64>(),
        arb_text(),
        arb_date(),
        arb_time(),
        option::of(any::<u32>()),
        option::of(any::<i64>()),
    )
        .prop_map(
            |(id, title, start_date, start_time, duration_minutes, group_id)| StorageEventDto {
                id,
                title,
                start_date,
                start_time,
                duration_minutes,
                group_id,
            },
        )
}

fn arb_group_dto() -> impl Strategy<Value = StorageGroupDto> {
    (any::<i64>(), arb_text(), arb_hex_color())
        .prop_map(|(id, name, color)| StorageGroupDto { id, name, color })
}

// ---------------------------------------------------------------------------
// Payload arms — each arm returns a (Kind, MessageType, Option<Value>) tuple
// so they can be combined with `prop_oneof!` on a uniform output type.
// ---------------------------------------------------------------------------

type Content = (Kind, MessageType, Option<Value>);

fn typed<P: serde::Serialize>(kind: Kind, mt: MessageType, payload: P) -> Content {
    let value = serde_json::to_value(&payload).expect("typed payload serializes");
    (kind, mt, Some(value))
}

fn arb_user_message_arm() -> impl Strategy<Value = Content> {
    arb_text().prop_map(|text| {
        typed(
            Kind::Request,
            MessageType::UserMessage,
            UserMessagePayload { text },
        )
    })
}

fn arb_agent_reply_arm() -> impl Strategy<Value = Content> {
    arb_text().prop_map(|text| {
        typed(
            Kind::Response,
            MessageType::AgentReply,
            AgentReplyPayload { text },
        )
    })
}

fn arb_agent_init_arm() -> impl Strategy<Value = Content> {
    (arb_text(), arb_model_provider()).prop_map(|(timezone, model_provider)| {
        typed(
            Kind::Request,
            MessageType::AgentInit,
            AgentInitPayload {
                timezone,
                model_provider,
                ollama_model: String::new(),
                ollama_host: String::new(),
                bedrock_model_id: None,
            },
        )
    })
}

fn arb_agent_init_ack_arm() -> impl Strategy<Value = Content> {
    option::of(arb_text()).prop_map(|provider_notice| {
        typed(
            Kind::Response,
            MessageType::AgentInitAck,
            AgentInitAckPayload { provider_notice },
        )
    })
}

fn arb_agent_tool_progress_arm() -> impl Strategy<Value = Content> {
    (arb_text(), arb_tool_progress_phase()).prop_map(|(tool, phase)| {
        typed(
            Kind::Event,
            MessageType::AgentToolProgress,
            AgentToolProgressPayload { tool, phase },
        )
    })
}

fn arb_shutdown_arm() -> impl Strategy<Value = Content> {
    Just(typed(
        Kind::Request,
        MessageType::Shutdown,
        ShutdownPayload {},
    ))
    .boxed()
}

fn arb_storage_create_task_request_arm() -> impl Strategy<Value = Content> {
    (
        arb_text(),
        option::of(arb_priority()),
        option::of(arb_iso_ts()),
        option::of(any::<i64>()),
    )
        .prop_map(|(title, priority, deadline, group_id)| {
            typed(
                Kind::Request,
                MessageType::StorageCreateTask,
                StorageCreateTaskRequest {
                    title,
                    priority,
                    deadline,
                    group_id,
                },
            )
        })
}

fn arb_storage_create_task_response_arm() -> impl Strategy<Value = Content> {
    arb_task_dto().prop_map(|task| {
        typed(
            Kind::Response,
            MessageType::StorageCreateTask,
            StorageCreateTaskResponse { task },
        )
    })
}

fn arb_storage_list_tasks_request_arm() -> impl Strategy<Value = Content> {
    (
        option::of(arb_task_status()),
        option::of(option::of(any::<i64>())),
        option::of(arb_date()),
        option::of(arb_date()),
    )
        .prop_map(|(status, group_id, from_date, to_date)| {
            typed(
                Kind::Request,
                MessageType::StorageListTasks,
                StorageListTasksRequest {
                    status,
                    group_id,
                    from_date,
                    to_date,
                },
            )
        })
}

fn arb_storage_list_tasks_response_arm() -> impl Strategy<Value = Content> {
    proptest::collection::vec(arb_task_dto(), 0..5).prop_map(|tasks| {
        typed(
            Kind::Response,
            MessageType::StorageListTasks,
            StorageListTasksResponse { tasks },
        )
    })
}

fn arb_storage_create_event_request_arm() -> impl Strategy<Value = Content> {
    (
        arb_text(),
        arb_date(),
        arb_time(),
        option::of(any::<u32>()),
        option::of(any::<i64>()),
    )
        .prop_map(
            |(title, start_date, start_time, duration_minutes, group_id)| {
                typed(
                    Kind::Request,
                    MessageType::StorageCreateEvent,
                    StorageCreateEventRequest {
                        title,
                        start_date,
                        start_time,
                        duration_minutes,
                        group_id,
                    },
                )
            },
        )
}

fn arb_storage_create_event_response_arm() -> impl Strategy<Value = Content> {
    arb_event_dto().prop_map(|event| {
        typed(
            Kind::Response,
            MessageType::StorageCreateEvent,
            StorageCreateEventResponse { event },
        )
    })
}

fn arb_storage_create_group_request_arm() -> impl Strategy<Value = Content> {
    (arb_text(), arb_hex_color()).prop_map(|(name, color)| {
        typed(
            Kind::Request,
            MessageType::StorageCreateGroup,
            StorageCreateGroupRequest { name, color },
        )
    })
}

fn arb_storage_create_group_response_arm() -> impl Strategy<Value = Content> {
    arb_group_dto().prop_map(|group| {
        typed(
            Kind::Response,
            MessageType::StorageCreateGroup,
            StorageCreateGroupResponse { group },
        )
    })
}

fn arb_storage_export_markdown_request_arm() -> impl Strategy<Value = Content> {
    arb_text().prop_map(|output_path| {
        typed(
            Kind::Request,
            MessageType::StorageExportMarkdown,
            StorageExportMarkdownRequest { output_path },
        )
    })
}

fn arb_storage_export_markdown_response_arm() -> impl Strategy<Value = Content> {
    arb_text().prop_map(|written_path| {
        typed(
            Kind::Response,
            MessageType::StorageExportMarkdown,
            StorageExportMarkdownResponse { written_path },
        )
    })
}

/// Envelopes with no payload. Covers `shutdown`, empty responses
/// (e.g. `storage.delete_task`) and the no-payload half of the wire space.
fn arb_no_payload_arm() -> impl Strategy<Value = Content> {
    (arb_kind(), arb_message_type()).prop_map(|(kind, mt)| (kind, mt, None))
}

// ---------------------------------------------------------------------------
// Grouped payload strategies. `prop_oneof!` supports up to ~10 arms at a
// time, so the full space is composed from smaller buckets.
// ---------------------------------------------------------------------------

fn arb_content_lifecycle() -> impl Strategy<Value = Content> {
    prop_oneof![
        arb_user_message_arm(),
        arb_agent_reply_arm(),
        arb_agent_init_arm(),
        arb_agent_init_ack_arm(),
        arb_agent_tool_progress_arm(),
        arb_shutdown_arm(),
    ]
}

fn arb_content_tasks() -> impl Strategy<Value = Content> {
    prop_oneof![
        arb_storage_create_task_request_arm(),
        arb_storage_create_task_response_arm(),
        arb_storage_list_tasks_request_arm(),
        arb_storage_list_tasks_response_arm(),
    ]
}

fn arb_content_events() -> impl Strategy<Value = Content> {
    prop_oneof![
        arb_storage_create_event_request_arm(),
        arb_storage_create_event_response_arm(),
    ]
}

fn arb_content_groups() -> impl Strategy<Value = Content> {
    prop_oneof![
        arb_storage_create_group_request_arm(),
        arb_storage_create_group_response_arm(),
    ]
}

fn arb_content_export() -> impl Strategy<Value = Content> {
    prop_oneof![
        arb_storage_export_markdown_request_arm(),
        arb_storage_export_markdown_response_arm(),
    ]
}

fn arb_content() -> impl Strategy<Value = Content> {
    prop_oneof![
        arb_content_lifecycle(),
        arb_content_tasks(),
        arb_content_events(),
        arb_content_groups(),
        arb_content_export(),
        arb_no_payload_arm(),
    ]
}

// ---------------------------------------------------------------------------
// Error strategy
// ---------------------------------------------------------------------------

fn arb_error_code() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("NOT_FOUND".to_string()),
        Just("GROUP_NAME_NOT_UNIQUE".to_string()),
        Just("VALIDATION".to_string()),
        Just("IO_NOT_WRITABLE".to_string()),
        Just("DB_IO_ERROR".to_string()),
        Just("SCHEMA_MIGRATION_FAILED".to_string()),
    ]
}

fn arb_error() -> impl Strategy<Value = StorageError> {
    (arb_error_code(), arb_text()).prop_map(|(code, message)| StorageError { code, message })
}

// ---------------------------------------------------------------------------
// Envelope strategy
// ---------------------------------------------------------------------------

fn arb_envelope() -> impl Strategy<Value = Envelope> {
    (
        arb_uuid(),
        arb_content(),
        option::of(arb_uuid()),
        option::of(arb_error()),
    )
        .prop_map(|(id, (kind, message_type, payload), ref_id, error)| {
            // Wire invariant from design.md: error responses carry no payload.
            let payload = if error.is_some() { None } else { payload };
            Envelope {
                v: PROTOCOL_VERSION,
                id,
                kind,
                message_type,
                payload,
                ref_id,
                error,
            }
        })
}

// ---------------------------------------------------------------------------
// Property 1: Round-trip de serialización IPC
// ---------------------------------------------------------------------------

// Feature: jinx, Property 1: Round-trip de serialización IPC
proptest! {
    #[test]
    fn round_trip_serialization_ipc(envelope in arb_envelope()) {
        let json = serde_json::to_string(&envelope).expect("envelope serializes");
        let decoded: Envelope = serde_json::from_str(&json).expect("envelope deserializes");
        prop_assert_eq!(decoded, envelope);
    }
}

// ---------------------------------------------------------------------------
// Task 2.5 — canonical fixtures JSONL
// ---------------------------------------------------------------------------

#[test]
fn canonical_fixtures_deserialize_correctly() {
    let fixtures = include_str!("fixtures/ipc_samples.jsonl");
    let mut count = 0usize;
    for line in fixtures.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let env: Envelope = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("fixture line failed to deserialize: {e}\nLine: {line}"));
        let re_serialized = serde_json::to_string(&env).expect("re-serializes");
        let decoded_again: Envelope =
            serde_json::from_str(&re_serialized).expect("second deserialize");
        assert_eq!(env, decoded_again, "round-trip failed for fixture line: {line}");
        count += 1;
    }
    assert!(count > 0, "no fixture lines found");
}
