# Arquitectura — Terminal Day Organizer

## 1. Arquitectura general del sistema

```mermaid
graph TB
    subgraph usuario["Usuario"]
        KB[Teclado]
    end

    subgraph tui["TUI — Rust (Ratatui)"]
        direction TB
        RENDER[Render Loop<br/>60 fps]
        APP[AppState<br/>reduce&#40;state, event&#41;]
        MAIN[main.rs<br/>Event Loop]
        STORE[SqliteStorage<br/>storage crate]
        IPC_H[ipc_handler.rs<br/>Storage Dispatcher]
        READER["Reader Thread<br/>mpsc::channel"]
    end

    subgraph canal["Canal_IPC — JSON Lines over stdio"]
        STDIN[stdin del agente]
        STDOUT[stdout del agente]
    end

    subgraph agente["Agente — Python (Strands Agents)"]
        direction TB
        LOOP[Turn Loop<br/>main.py]
        STRAND[Strands Agent<br/>Claude LLM]
        STDIO[StdioClient<br/>ipc.py]
        STOOLS[Storage Tools<br/>storage_tools.py]
        BTOOLS[Built-in Tools<br/>strands_tools]
        INFER[Inference Engine<br/>inference.py]
    end

    subgraph disk["Disco"]
        DB[("SQLite<br/>~/.local/share/<br/>terminal-day-organizer/db.sqlite")]
        LOG["/tmp/tui_agent.log"]
    end

    KB --> MAIN
    MAIN --> APP
    APP --> RENDER
    MAIN --> IPC_H
    IPC_H --> STORE
    STORE --> DB
    MAIN --> READER

    READER -->|parsea JSON Lines| MAIN
    MAIN -->|escribe JSON Lines| STDIN
    STDOUT -->|lee líneas| READER

    LOOP --> STRAND
    STRAND --> STOOLS
    STRAND --> BTOOLS
    STRAND --> INFER
    STOOLS --> STDIO
    STDIO -->|escribe request| STDOUT
    STDIO -->|lee response| STDIN
    STRAND -->|streaming stderr| LOG
```

---

## 2. Protocolo IPC — Canal_IPC (JSON Lines)

```mermaid
sequenceDiagram
    participant TUI as TUI (Rust)
    participant AG as Agente (Python)

    TUI->>AG: agent_init {timezone, model_provider}
    AG->>TUI: agent_init_ack {provider_notice?}

    loop Por cada mensaje del usuario
        TUI->>AG: user_message {text}
        
        opt El agente necesita persistir datos
            AG->>TUI: storage.create_task {title, priority, ...}
            TUI->>AG: response {task: {...}} o error {code, message}
        end

        opt El agente razona sobre grupos
            AG->>TUI: storage.list_groups {}
            TUI->>AG: response {groups: [...]}
        end

        AG->>TUI: agent_reply {text}
    end

    TUI->>AG: shutdown
```

---

## 3. Estructura del Envelope IPC

```mermaid
classDiagram
    class Envelope {
        +u8 v = 1
        +Uuid id
        +Kind kind
        +MessageType type
        +Option~Value~ payload
        +Option~Uuid~ ref_id
        +Option~StorageError~ error
    }

    class Kind {
        <<enumeration>>
        request
        response
        event
    }

    class MessageType {
        <<enumeration>>
        user_message
        agent_reply
        agent_init
        agent_init_ack
        shutdown
        storage.list_tasks
        storage.create_task
        storage.update_task
        storage.complete_task
        storage.delete_task
        storage.list_events
        storage.create_event
        storage.update_event
        storage.delete_event
        storage.list_groups
        storage.create_group
        storage.rename_group
        storage.recolor_group
        storage.delete_group
        storage.export_markdown
        storage.export_sqlite
    }

    class StorageError {
        +String code
        +String message
    }

    Envelope --> Kind
    Envelope --> MessageType
    Envelope --> StorageError
```

---

## 4. Modelo de datos (Almacén SQLite)

```mermaid
erDiagram
    GROUP {
        i64 id PK
        string name "único"
        string color "#RRGGBB"
    }

    TASK {
        i64 id PK
        string title
        Priority priority "alta | media | baja"
        TaskStatus status "pendiente | completada | cancelada"
        string created_at "ISO 8601"
        string deadline "ISO 8601, nullable"
        i64 group_id FK "nullable"
    }

    EVENT {
        i64 id PK
        string title
        string start_date "YYYY-MM-DD"
        string start_time "HH:MM"
        u32 duration_minutes "nullable"
        i64 group_id FK "nullable"
    }

    GROUP ||--o{ TASK : "clasifica"
    GROUP ||--o{ EVENT : "clasifica"
```

---

## 5. Flujo de estado del TUI — máquina de reducción pura

```mermaid
stateDiagram-v2
    [*] --> Chat : inicio

    state "Panel activo" as PA {
        Chat
        Tareas
        Calendario
        Proximos
    }

    Chat --> Tareas : Tab
    Tareas --> Calendario : Tab
    Calendario --> Proximos : Tab
    Proximos --> Chat : Tab

    Tareas --> Chat : Shift+Tab
    Calendario --> Tareas : Shift+Tab
    Proximos --> Calendario : Shift+Tab
    Chat --> Proximos : Shift+Tab

    state "Modal activo" as MOD {
        NewTask
        EditTask
        DeleteTask
        NewEvent
        EditEvent
        DeleteEvent
        NewGroup
        EditGroup
        DeleteGroup
        Error
    }

    PA --> MOD : n/e/d/g (teclas de panel)
    MOD --> PA : Esc / Enter
    
    state "Viewport demasiado pequeño" as SMALL
    PA --> SMALL : Resize < 60x20
    SMALL --> PA : Resize ≥ 60x20
```

---

## 6. Motor de inferencia de Grupos

```mermaid
flowchart TD
    MSG[Mensaje del usuario] --> NORM[normalize\nlowercase + strip acentos]
    NORM --> TRI[ngrams n=3\ncon padding de espacios]
    
    TRI --> JAC{Para cada Grupo}
    
    subgraph por_grupo["Por cada Grupo en el snapshot"]
        GNORM[normalize nombre + títulos miembros]
        GTRI[ngrams del Grupo]
        SCORE[Jaccard similarity\n|A ∩ B| / |A ∪ B|]
        GNORM --> GTRI --> SCORE
    end
    
    JAC --> por_grupo
    
    por_grupo --> ARGMAX[argmax score\ntie-break → id más bajo]
    
    ARGMAX --> THRESH{score ≥ umbral?}
    
    THRESH -->|score ≥ 0.35 AUTO| ASSIGN[Auto-asignar Grupo]
    THRESH -->|0.20 ≤ score < 0.35| SUGGEST[Sugerir con confirmación]
    THRESH -->|score < 0.20| NEW[Proponer crear Grupo nuevo]
```

---

## 7. Capas del proyecto (workspace Cargo + paquete Python)

```mermaid
graph BT
    subgraph workspace["Cargo Workspace"]
        subgraph storage_crate["crate: storage"]
            MODELS[models.rs\nTask, Event, Group\nPriority, TaskStatus\nHexColor]
            DB[db.rs\nSqliteStorage]
            EXPORT[export.rs\nExporter MD + SQLite]
            ERROR[error.rs\nStorageError]
            PATH[path.rs\nresolve_db_path]
            LIB_S[lib.rs\nStorage trait]
        end

        subgraph tui_crate["crate: tui"]
            IPC_RS[ipc.rs\nEnvelope, MessageType\nDTOs de la capa IPC]
            IPC_HAND[ipc_handler.rs\nDispatcher storage.*]
            APP_RS[app.rs\nAppState, Panel\nreduce&#40;&#41;]
            MAIN_RS[main.rs\nEvent loop, render\nmodales, formularios]
            COLOR_RS[color.rs]
            CAL_RS[calendario.rs]
            PROX_RS[proximos.rs]
        end
    end

    subgraph py_pkg["Paquete Python: agent/"]
        IPC_PY[ipc.py\nStdioClient\nEnvelope TypedDicts]
        STOR_T[storage_tools.py\n16 @tools proxy]
        INFER_PY[inference.py\ninfer_group_candidate]
        MAIN_PY[main.py\nTurn loop\n_build_agent]
        STRANDS_T[strands_tools\ncurrent_time, file_read\nfile_write, editor, think]
    end

    DB --> LIB_S
    MODELS --> LIB_S
    EXPORT --> LIB_S
    PATH --> LIB_S

    LIB_S --> IPC_HAND
    IPC_RS --> IPC_HAND
    IPC_RS --> APP_RS
    IPC_HAND --> MAIN_RS
    APP_RS --> MAIN_RS

    STOR_T --> IPC_PY
    INFER_PY --> MAIN_PY
    STRANDS_T --> MAIN_PY
    STOR_T --> MAIN_PY
    IPC_PY --> MAIN_PY
```
