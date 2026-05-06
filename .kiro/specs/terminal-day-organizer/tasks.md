# Plan de Implementación: Organizador de Día en Terminal

## Resumen

Este plan convierte el diseño aprobado en una secuencia de tareas incrementales de codificación. Cada paso se apoya en los anteriores y el último cierra el cableado entre la TUI (Rust + Ratatui), el Agente (Python + Strands Agents) y el Almacén SQLite, sin dejar código huérfano. Las tareas marcadas con `*` son opcionales (tests de propiedad, unitarios o de integración) y pueden omitirse en un MVP, aunque son necesarias para cerrar las propiedades P1–P26 del diseño.

Cada test de propiedad debe llevar en su cabecera la anotación exigida por el diseño:

```
Feature: terminal-day-organizer, Property <N>: <texto textual de la propiedad>
```

## Tareas

- [x] 1. Scaffolding del workspace
  - [x] 1.1 Crear workspace Cargo con los crates `tui` (binario) y `storage` (biblioteca) y declarar dependencias `ratatui`, `crossterm`, `rusqlite` con feature `bundled`, `serde`, `serde_json`, `uuid`, `directories`, `chrono`, `thiserror`, `proptest` (dev)
    - Añadir `rust-toolchain.toml` fijando edición estable
    - _Requisitos: 12.1, 12.4_
  - [x] 1.2 Crear paquete Python `agent/` con `pyproject.toml` declarando dependencias `strands-agents`, `strands-tools`, `hypothesis` (dev), `pytest` (dev), `ruff` (dev), `mypy` (dev)
    - Definir estructura `agent/__init__.py`, `agent/ipc.py`, `agent/storage_tools.py`, `agent/inference.py`, `agent/main.py`, `tests/`
    - _Requisitos: 12.2, 12.3_
  - [x] 1.3 Añadir archivos base de CI y linters (`.github/workflows/ci.yml` con jobs vacíos, configuración de `clippy`, `ruff.toml`, `mypy.ini`)
    - Configurar el job para ejecutarse con `PROPTEST_CASES=200` y `HYPOTHESIS_PROFILE=ci`
    - _Requisitos: 12.1, 12.2_

- [ ] 2. Esquema IPC compartido y round-trip de serialización
  - [x] 2.1 Definir en el crate `tui` los tipos Rust del envelope y payloads IPC con `serde::{Serialize, Deserialize}` siguiendo Data Models: `Envelope`, `Kind`, `MessageType`, `UserMessagePayload`, `AgentReplyPayload`, `AgentInitPayload`, `StorageError`, y los payloads de cada `storage.*`
    - Incluir validación de `v=1` al deserializar
    - _Requisitos: 10.1, 10.2_
  - [x] 2.2 Escribir property test P1 lado Rust con `proptest`: para cualquier `Envelope` generado, `from_json(to_json(m)) == m` bajo el contrato de igualdad del diseño
    - **Property 1: Round-trip de serialización IPC**
    - **Valida: Requisitos 10.1, 10.2, 10.3**
  - [ ] 2.3 Definir en el paquete Python los `TypedDict` equivalentes en `agent/ipc.py` con los mismos nombres de campo que en Rust, y funciones `encode(env) -> str` / `decode(line) -> Envelope`
    - _Requisitos: 10.1, 10.2_
  - [ ] 2.4 Escribir property test P1 lado Python con `hypothesis`: estrategia para `Envelope`, comprobar `decode(encode(m)) == m`
    - **Property 1: Round-trip de serialización IPC**
    - **Valida: Requisitos 10.1, 10.2, 10.3**
  - [ ] 2.5 Crear un conjunto de mensajes canónicos (fixtures JSONL) y exponerlo como recurso compartido entre `tui/tests/fixtures/ipc_samples.jsonl` y `agent/tests/fixtures/ipc_samples.jsonl`
    - _Requisitos: 10.1, 10.2_
  - [ ] 2.6 Escribir test cruzado de round-trip IPC entre procesos: un binario de test en Rust que serializa N envelopes generados con `proptest`, los envía por stdout a un proceso Python que los deserializa, reserializa y los devuelve por stdin al proceso Rust, donde se comparan
    - **Property 1: Round-trip de serialización IPC (cruzado)**
    - **Valida: Requisitos 10.1, 10.2, 10.3**

- [ ] 3. Almacén SQLite: esquema, migraciones y trait `Storage`
  - [ ] 3.1 En el crate `storage` implementar `resolve_db_path()` usando `directories::ProjectDirs::from("", "", "terminal-day-organizer")`, creando el directorio si falta
    - _Requisitos: 7.1, 9.2_
  - [ ] 3.2 Implementar motor de migraciones con tabla `schema_version`, colección ordenada de migraciones y aplicación dentro de transacción; registrar la migración 0→1 con el esquema definido en Data Models (groups, tasks, events, índices, FK `ON DELETE SET NULL`, `PRAGMA foreign_keys=ON`, WAL)
    - _Requisitos: 7.3, 7.4_
  - [ ] 3.3 Implementar tipo `HexColor(String)` con constructor que valide `^#[0-9A-Fa-f]{6}$`
    - _Requisitos: 15.1, 16.1_
  - [ ] 3.4 Implementar los structs Rust `Group`, `Task`, `Event`, enums `Priority`, `TaskStatus`, y los tipos `NewTask`, `NewEvent`, `NewGroup`, `TaskPatch`, `EventPatch`, `GroupPatch`, `TaskFilter`, `GroupsSnapshot`
    - _Requisitos: 2.1, 3.1, 15.1, 15.6_
  - [ ] 3.5 Definir el trait `Storage` con las operaciones listadas en Components and Interfaces y el enum `StorageError` con los códigos de Error Handling
    - _Requisitos: 2.1, 3.1, 7.2, 11.3, 15.1_
  - [ ] 3.6 Implementar `SqliteStorage` para Tareas: `create_task`, `list_tasks` (con filtro por estado/rango/Grupo y orden por Prioridad y fecha límite), `update_task`, `complete_task`, `delete_task`, todas dentro de `BEGIN IMMEDIATE ... COMMIT`
    - Asignar `priority = "media"` cuando el campo no venga en `NewTask`
    - _Requisitos: 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 2.7, 7.2, 13.2, 13.3, 13.4, 13.5_
  - [ ] 3.7 Implementar `SqliteStorage` para Eventos: `create_event`, `list_events(range)`, `update_event`, `delete_event` con orden por `(start_date, start_time)` ascendente
    - _Requisitos: 3.1, 3.3, 3.4, 3.5, 7.2, 13.7, 13.8, 13.9_
  - [ ] 3.8 Implementar `SqliteStorage` para Grupos: `create_group` (error `GROUP_NAME_NOT_UNIQUE` si colisiona), `rename_group`, `recolor_group`, `delete_group` (se apoya en `ON DELETE SET NULL`)
    - _Requisitos: 15.1, 15.2, 15.3, 15.4, 15.5, 15.6_
  - [ ] 3.9 Implementar `snapshot_for_inference` devolviendo por Grupo el id, el nombre y los títulos de Tareas y Eventos asociados
    - _Requisitos: 15.9_
  - [ ] 3.10 Property test P11: `NewTask` sin `priority` persiste con `priority = "media"` y toda `NewTask` válida persiste con `status = "pendiente"`
    - **Property 11: Creación con valores por defecto**
    - **Valida: Requisitos 2.1, 2.7**
  - [ ] 3.11 Property test P12: patch que modifica un único campo actualiza ese campo y deja los demás inalterados, para Tareas, Eventos y Grupos
    - **Property 12: Actualización de un único campo**
    - **Valida: Requisitos 2.5, 3.5, 15.3, 15.4**
  - [ ] 3.12 Property test P13: eliminar una entidad la retira de los listados; eliminar un Grupo deja en `NULL` el `group_id` de sus Tareas y Eventos y no toca otras
    - **Property 13: Eliminación y cascada a NULL**
    - **Valida: Requisitos 2.4, 3.4, 15.5**
  - [ ] 3.13 Property test P14: operación de escritura sobre un id inexistente devuelve `NOT_FOUND` y el estado queda intacto
    - **Property 14: Operación sobre entidad inexistente**
    - **Valida: Requisitos 2.6**
  - [ ] 3.14 Property test P15: secuencia de escrituras exitosas; cerrar y reabrir la base produce el mismo estado observable que tras el último `COMMIT`
    - **Property 15: Durabilidad transaccional al reabrir**
    - **Valida: Requisitos 7.2, 7.3**
  - [ ] 3.15 Property test P16: `create_group` con nombre ya presente devuelve `GROUP_NAME_NOT_UNIQUE` sin alterar el Almacén; con nombre nuevo persiste
    - **Property 16: Unicidad de nombre de Grupo**
    - **Valida: Requisitos 15.1, 15.2**
  - [ ] 3.16 Property test P17: tras cualquier secuencia válida, `group_id` de Tareas y Eventos es `NULL` o referencia a un Grupo existente
    - **Property 17: Invariante de Grupo único por entidad**
    - **Valida: Requisitos 15.6**
  - [ ] 3.17 Property test P6: `list_tasks(status="pendiente")` devuelve un orden total con `alta ≺ media ≺ baja`, y dentro de cada Prioridad las Tareas con fecha límite preceden a las que no la tienen, ordenadas ascendentemente por fecha límite
    - **Property 6: Orden total de listado y priorización de Tareas**
    - **Valida: Requisitos 2.2, 4.1**
  - [ ] 3.18 Property test P7: `list_events(d1, d2)` devuelve exactamente los Eventos con `start_date ∈ [d1, d2]`, ordenados por `(start_date, start_time)` ascendente
    - **Property 7: Listado de Eventos por rango**
    - **Valida: Requisitos 3.3**

- [ ] 4. Checkpoint - validar almacén e IPC
  - Asegurar que todos los tests pasan; preguntar al usuario si surgen dudas sobre el esquema o los payloads antes de continuar.

- [ ] 5. Esqueleto de la TUI con máquina de foco y viewport guard
  - [ ] 5.1 Definir `AppState` puro con campos `focused_panel: Panel`, `viewport: (u16,u16)`, `status_bar: String`, `modal: Option<Modal>` y función pura `reduce(state, event) -> state` para eventos de teclado
    - _Requisitos: 14.2, 14.3, 14.5_
  - [ ] 5.2 Implementar transición de foco `Tab`/`Shift+Tab` siguiendo el ciclo `Chat → Tareas → Calendario → Proximos → Chat` como función pura
    - _Requisitos: 14.3_
  - [ ] 5.3 Implementar viewport guard: si `cols < 100 || rows < 30` el estado pasa a `ViewportTooSmall` y la función de reducción descarta cualquier evento que desencadene operaciones sobre el Almacén
    - _Requisitos: 14.9_
  - [ ] 5.4 Implementar el layout Ratatui con los cuatro paneles y el indicador visual de foco (borde resaltado con `Modifier::BOLD` y sufijo `[ACTIVO]` en el título)
    - _Requisitos: 14.1, 14.4_
  - [ ] 5.5 Implementar la barra de estado global con capacidad de mostrar avisos y errores
    - _Requisitos: 11.1, 11.3, 13.11, 16.4_
  - [ ] 5.6 Cablear el dispatcher principal que consume eventos del hilo de entrada, aplica `reduce` y redibuja con `ratatui::backend::TestBackend` para tests
    - _Requisitos: 14.1, 14.5_
  - [ ] 5.7 Property test P18: tras cualquier secuencia de eventos aplicada a `AppState`, el número de paneles con foco es exactamente uno
    - **Property 18: Exactamente un Panel_Enfocado**
    - **Valida: Requisitos 14.2**
  - [ ] 5.8 Property test P19: desde `focused_panel = Chat`, aplicar `k` pulsaciones de `Tab` deja el foco en el panel en posición `k mod 4` del orden fijo
    - **Property 19: Ciclo determinista de foco con Tab**
    - **Valida: Requisitos 14.3**
  - [ ] 5.9 Property test P20: para cualquier pulsación no global, sólo el handler del panel enfocado procesa el evento (verificado sobre un `AppState` instrumentado que marca qué handler fue llamado)
    - **Property 20: Ruteo exclusivo al panel enfocado**
    - **Valida: Requisitos 14.5**
  - [ ] 5.10 Property test P21: para cualquier tamaño de terminal por debajo del mínimo, ninguna tecla configurada para CRUD altera el Almacén observado
    - **Property 21: Bloqueo por viewport insuficiente**
    - **Valida: Requisitos 14.9**

- [ ] 6. Estrategia de color y renderizado de estilos
  - [ ] 6.1 Implementar `detect_color_mode()` consultando `$COLORTERM`, `$TERM` y `tput colors`, devolviendo `ColorMode::{TrueColor, Xterm256, Monochrome}`
    - _Requisitos: 16.4, 16.5_
  - [ ] 6.2 Implementar función pura `resolve_style(group: Option<&Group>, mode: ColorMode, neutral: HexColor) -> StyledText` que mapea hex a color exacto en TrueColor, al más cercano en xterm-256 por distancia euclídea en RGB con desempate por índice ascendente, y a `None` en Monochrome
    - Garantizar que el color neutro para entradas sin Grupo difiere del color elegido para cualquier Grupo existente
    - _Requisitos: 16.1, 16.2, 16.4_
  - [ ] 6.3 Implementar prefijo textual `[nombre_grupo]` cuando `ColorMode::Monochrome`
    - _Requisitos: 16.5_
  - [ ] 6.4 Property test P22: para cualquier hex y cualquier modo ∈ {TrueColor, Xterm256}, el color elegido minimiza distancia en RGB con desempate por índice; el neutro difiere de todos los `Color_Grupo` del snapshot
    - **Property 22: Fallback de color y neutro distinguible**
    - **Valida: Requisitos 16.2, 16.4**
  - [ ] 6.5 Property test P23: en modo Monochrome, la línea renderizada de una entrada con Grupo contiene la subcadena `[<nombre_grupo>]`
    - **Property 23: Marcador textual en modo monocromo**
    - **Valida: Requisitos 16.5**

- [ ] 7. Esqueleto del Agente con `strands_tools` y `FakeIPC`
  - [ ] 7.1 Implementar `agent/ipc.py::StdioClient` que lee/escribe JSONL por stdin/stdout, correlacionando `request`/`response` por `ref` y exponiendo una llamada bloqueante `send_request(type, payload) -> payload | raises StorageError`
    - _Requisitos: 10.1, 10.2_
  - [ ] 7.2 Implementar `FakeIPC` en `agent/tests/fake_ipc.py` con respuestas programadas e inyección de errores; usar el mismo contrato que `StdioClient`
    - _Requisitos: 10.1, 11.3_
  - [ ] 7.3 Cablear el `Agent` de Strands en `agent/main.py` con el system prompt, registrando las herramientas prehechas `current_time`, `file_read`, `file_write`, `editor` importadas de `strands_tools`
    - _Requisitos: 5.1, 12.3_
  - [ ] 7.4 Implementar el bucle de turno: al recibir `user_message`, invocar `current_time` una sola vez, inyectar `NOW = <iso>` en el prompt del turno y ejecutar el razonamiento; emitir un único `agent_reply`
    - _Requisitos: 5.1, 5.2, 5.3, 9.4_
  - [ ] 7.5 Implementar notificación `agent_init_ack` con `provider_notice` cuando `model_provider == "remote"`
    - _Requisitos: 9.3, 9.4_
  - [ ] 7.6 Test unitario: introspección del `Agent` comprueba que `current_time`, `file_read` y `file_write` provienen del paquete `strands_tools`
    - _Requisitos: 12.3_
  - [ ] 7.7 Test de ejemplo con `FakeIPC`: un `user_message` con intención ambigua produce un `agent_reply` sin invocar Storage Tools
    - _Requisitos: 5.3, 11.2_

- [ ] 8. Inferencia de Grupo como función pura
  - [ ] 8.1 Implementar `agent/inference.py::normalize(s)` con NFKD + descarte de marcas combinantes + minúsculas + colapso de espacios
    - _Requisitos: 15.9_
  - [ ] 8.2 Implementar `ngrams(s, n=3)` devolviendo el conjunto de trigramas de caracteres con padding de espacio al inicio y fin
    - _Requisitos: 15.9_
  - [ ] 8.3 Implementar `infer_group_candidate(message, groups_snapshot) -> (group_id, score)` con Jaccard de trigramas y desempate estricto por `group_id` ascendente; registrarla como `@tool` de Strands
    - _Requisitos: 15.9, 15.13_
  - [ ] 8.4 Implementar en el Agente la lógica de umbralización: `≥0.75` asigna e informa; `0.25..0.75` propone y pide confirmación; `<0.25` propone crear un Grupo nuevo con nombre sugerido
    - _Requisitos: 15.10, 15.11, 15.12_
  - [ ] 8.5 Property test P4 con `hypothesis`: para cualquier mensaje y cualquier snapshot, `infer_group_candidate` es determinista y devuelve el Grupo con score máximo según Jaccard, con desempate por id ascendente
    - **Property 4: Determinismo y argmax de la inferencia de Grupo**
    - **Valida: Requisitos 15.9, 15.13**
  - [ ] 8.6 Property test P5 con `hypothesis`: para cualquier mensaje y snapshot, la acción del Agente corresponde al intervalo de la puntuación según el umbral (asignación / propuesta con confirmación / propuesta de nuevo Grupo)
    - **Property 5: Ruteo por umbral de la acción tras inferencia**
    - **Valida: Requisitos 15.10, 15.11, 15.12**

- [ ] 9. Storage Tools del Agente (proxies IPC)
  - [ ] 9.1 Implementar en `agent/storage_tools.py` las `@tool` proxy para Tareas: `list_tasks`, `create_task`, `update_task`, `complete_task`, `delete_task`, cada una construyendo el `Envelope` apropiado y bloqueando sobre `StdioClient.send_request`
    - _Requisitos: 2.1, 2.2, 2.3, 2.4, 2.5, 10.1, 10.2_
  - [ ] 9.2 Implementar las `@tool` proxy para Eventos: `list_events`, `create_event`, `update_event`, `delete_event`
    - _Requisitos: 3.1, 3.2, 3.3, 3.4, 3.5, 10.1_
  - [ ] 9.3 Implementar las `@tool` proxy para Grupos: `list_groups`, `create_group`, `rename_group`, `recolor_group`, `delete_group`
    - _Requisitos: 15.1, 15.2, 15.3, 15.4, 15.5_
  - [ ] 9.4 Implementar los proxies `export_markdown` y `export_sqlite` que delegan en el Exportador a través del IPC
    - _Requisitos: 8.1, 8.2, 8.3_
  - [ ] 9.5 Traducir cualquier `error` recibido por IPC a una excepción tool que el Agente convierta en texto natural preservando el `message` del error
    - _Requisitos: 11.3, 13.11_

- [ ] 10. Checkpoint - Agente aislado
  - Asegurar que los tests unitarios y de propiedad Python pasan con `FakeIPC`; preguntar al usuario si surgen dudas antes de cablear el proceso real.

- [ ] 11. Panel_Proximos
  - [ ] 11.1 Implementar función pura `proximos(snapshot, now) -> Vec<ProximosEntry>` que une Eventos con `start_datetime ∈ [now, now + 24h]` y Tareas con `deadline ∈ [now, now + 24h]`, ordenadas por su instante
    - _Requisitos: 6.1_
  - [ ] 11.2 Renderizar cada entrada con título, fecha, hora y, para Tareas, Prioridad; aplicar `resolve_style` para el color según Grupo y usar color neutro si no tiene Grupo
    - _Requisitos: 6.3, 6.4, 16.1, 16.2_
  - [ ] 11.3 Enlazar el Panel_Proximos con el contador de versión del Almacén de modo que el tick de 250 ms lo refresque cuando la versión cambia
    - _Requisitos: 6.2, 13.10, 16.3, 17.6_
  - [ ] 11.4 Property test P8: para cualquier snapshot y cualquier `now`, el conjunto mostrado es exactamente la unión definida en la Property
    - **Property 8: Selección del Panel_Proximos**
    - **Valida: Requisitos 6.1**
  - [ ] 11.5 Property test P9: cada línea renderizada contiene título, fecha, hora y (para Tareas) la Prioridad; el estilo aplicado a entradas con Grupo deriva del `Color_Grupo` según `resolve_style`
    - **Property 9: Contenido del render del Panel_Proximos**
    - **Valida: Requisitos 6.3, 6.4**

- [ ] 12. Panel_Calendario con marcadores de Tareas
  - [ ] 12.1 Implementar función pura `calendar_layout(snapshot, month) -> CalendarView` que ubica cada Evento en la celda de `start_date` y cada Tarea con `deadline` en la celda de su fecha límite; las Tareas sin fecha límite se omiten
    - _Requisitos: 17.1, 17.2, 17.3_
  - [ ] 12.2 Renderizar Eventos con prefijo `●` y Tareas con prefijo `▸`, preservando el prefijo al aplicar `Color_Grupo`
    - _Requisitos: 17.4, 17.5_
  - [ ] 12.3 Implementar atajos del panel (`n` nuevo, `e` editar, `d` eliminar, `←`/`→` día previo/siguiente, `PgUp`/`PgDn` mes previo/siguiente) y leyenda visible
    - _Requisitos: 14.7_
  - [ ] 12.4 Enlazar el refresco a la versión del Almacén tras cualquier cambio en Tareas o Eventos
    - _Requisitos: 17.6_
  - [ ] 12.5 Property test P10: el render del Panel_Calendario cumple las cinco condiciones de la propiedad para cualquier snapshot
    - **Property 10: Render del Panel_Calendario**
    - **Valida: Requisitos 17.1, 17.2, 17.3, 17.4, 17.5**

- [ ] 13. Panel_Tareas
  - [ ] 13.1 Renderizar la lista ordenada por (Prioridad, fecha límite) siguiendo el orden definido por `list_tasks` del Almacén
    - _Requisitos: 2.2, 4.1_
  - [ ] 13.2 Aplicar `resolve_style` para colorear según Grupo y usar neutro sin Grupo
    - _Requisitos: 16.1, 16.2_
  - [ ] 13.3 Implementar navegación `↑`/`↓` sobre la lista y leyenda visible con los atajos `n`, `e`, `c`, `d`
    - _Requisitos: 14.6_

- [ ] 14. Panel_Chat
  - [ ] 14.1 Renderizar el historial de mensajes etiquetados con el rol (`usuario` / `agente`) y el campo de entrada de texto
    - _Requisitos: 1.1, 1.3_
  - [ ] 14.2 Implementar el envío con `Enter`: serializar un `Envelope` con `type = "user_message"`, escribirlo en el stdin del Agente, añadir un mensaje pendiente al historial y arrancar un temporizador de 30 s
    - _Requisitos: 1.2, 10.1, 10.4_
  - [ ] 14.3 Implementar la guardia de mensaje vacío: si el texto tras eliminar whitespace Unicode es vacío, no se escribe nada en el Canal_IPC y la barra de estado muestra el aviso
    - _Requisitos: 11.1_
  - [ ] 14.4 Implementar el diálogo de timeout de 30 s con opciones "Reintentar" / "Cancelar"; "Reintentar" envía un nuevo `Envelope` con `id` nuevo
    - _Requisitos: 10.4_
  - [ ] 14.5 Implementar el manejo de error de IPC (EOF/broken pipe): marcar al Agente como caído, mostrar causa y ofrecer reinicio
    - _Requisitos: 1.4_
  - [ ] 14.6 Property test P25 con `proptest`: para cualquier cadena cuyo trim Unicode sea vacía, la TUI no escribe nada en el Canal_IPC y la barra de estado contiene el aviso
    - **Property 25: Guardia de mensaje vacío**
    - **Valida: Requisitos 11.1**

- [ ] 15. Formularios modales de edición manual
  - [ ] 15.1 Implementar el formulario modal de Tarea (crear/editar) con campos título, Prioridad, fecha límite opcional, Grupo y `Estado_Tarea` en edición; validación inline y focus interno con `Tab`
    - _Requisitos: 13.1, 13.2, 13.3_
  - [ ] 15.2 Implementar el formulario modal de Evento (crear/editar) con título, fecha, hora de inicio, duración opcional y Grupo
    - _Requisitos: 13.6, 13.7, 13.8_
  - [ ] 15.3 Implementar el formulario modal de Grupo (crear/editar) con nombre y Color_Grupo (input hex o selector de 16 presets)
    - _Requisitos: 15.1, 15.2, 15.3, 15.4_
  - [ ] 15.4 Implementar diálogo de confirmación para eliminar Tarea, Evento o Grupo, exigiendo `y` antes de persistir
    - _Requisitos: 13.5, 13.9, 15.5_
  - [ ] 15.5 Cablear la confirmación del formulario a una llamada sincrónica a la capa `Storage`; actualizar la vista sólo tras `COMMIT`; en caso de error mostrar la causa en la barra de estado y mantener el formulario abierto
    - _Requisitos: 7.2, 11.3, 13.2, 13.3, 13.4, 13.5, 13.7, 13.8, 13.9, 13.11_
  - [ ] 15.6 Enlazar la confirmación de la edición manual al refresco del Panel_Proximos dentro de 1 s usando el contador de versión
    - _Requisitos: 6.2, 13.10_
  - [ ] 15.7 Test de ejemplo con `TestBackend`: crear una Tarea desde el formulario la hace aparecer en el Panel_Tareas tras `COMMIT`
    - _Requisitos: 13.1, 13.2_
  - [ ] 15.8 Property test P26: para cualquier operación manual que termine en error al comprometerse en el Almacén, el estado observable queda igual al estado previo (usando un doble de `Storage` que inyecta errores)
    - **Property 26: Edición manual fallida no modifica el estado**
    - **Valida: Requisitos 13.11**

- [ ] 16. Exportador (Markdown + SQLite)
  - [ ] 16.1 Implementar `export_markdown(output_path)` en el crate `storage` generando un archivo con `# Tareas`, `# Eventos` y `# Grupos`, cada sección con una tabla que incluye todos los campos persistidos; Tareas ordenadas por (Prioridad, fecha límite), Eventos por (fecha, hora)
    - _Requisitos: 8.1, 8.3_
  - [ ] 16.2 Implementar `export_sqlite(output_path)` creando un archivo nuevo, aplicando el esquema actual y copiando Grupos → Tareas → Eventos
    - _Requisitos: 8.2, 8.3_
  - [ ] 16.3 Implementar `import_sqlite(path) -> Almacén` simétrica, usada por tests de round-trip y como base para futuras funcionalidades
    - _Requisitos: 8.5_
  - [ ] 16.4 Validar que la ruta de salida es escribible antes de crear el archivo; si no lo es devolver `IO_NOT_WRITABLE` sin escribir nada
    - _Requisitos: 8.4_
  - [ ] 16.5 Exponer `storage.export_markdown` y `storage.export_sqlite` como Storage Tool handlers en el proceso TUI
    - _Requisitos: 8.1, 8.2_
  - [ ] 16.6 Property test P2: generar un snapshot, exportarlo a SQLite, importar el archivo a un Almacén vacío y comprobar equivalencia de registros y FK
    - **Property 2: Round-trip de exportación SQLite**
    - **Valida: Requisitos 8.2, 8.5**
  - [ ] 16.7 Property test P3: generar un snapshot, exportarlo a Markdown y comprobar que contiene las tres secciones y una fila por entidad con todos los campos
    - **Property 3: Cobertura de la exportación a Markdown**
    - **Valida: Requisitos 8.1**
  - [ ] 16.8 Test de ejemplo: una ruta no escribible devuelve `IO_NOT_WRITABLE` y no deja archivos residuales
    - _Requisitos: 8.4_

- [ ] 17. Propagación de errores y bucle de presentación
  - [ ] 17.1 Mapear cada variante de `StorageError` a un `error` IPC con `code` y `message`; hacer `ROLLBACK` antes de responder
    - _Requisitos: 11.3, 13.11_
  - [ ] 17.2 En el Agente, traducir `error` del IPC a `agent_reply` en lenguaje natural preservando siempre el `message` original del Almacén
    - _Requisitos: 11.3_
  - [ ] 17.3 En la TUI, mostrar los errores de edición manual en la barra de estado preservando el `message` y mantener la vista previa
    - _Requisitos: 11.3, 13.11_
  - [ ] 17.4 Property test P24 (Rust + Python): para cualquier operación que termine con `{code, message}`, el texto final al usuario contiene la subcadena `message`
    - **Property 24: Propagación de errores del Almacén al usuario**
    - **Valida: Requisitos 11.3, 13.11**

- [ ] 18. Arranque y ciclo de vida del Organizador
  - [ ] 18.1 Implementar en `tui/main.rs` la secuencia: resolver ruta, abrir/crear Almacén, aplicar migraciones, preparar Ratatui, lanzar al Agente como subproceso con stdin/stdout conectados y stderr redirigido a `<config_dir>/agent.log`
    - _Requisitos: 7.1, 7.3, 7.4, 9.1, 9.2_
  - [ ] 18.2 Enviar `agent_init` con `timezone` del sistema y `model_provider`; tras `agent_init_ack`, si hay `provider_notice` añadir la nota en el Panel_Chat
    - _Requisitos: 9.3, 9.4_
  - [ ] 18.3 Implementar el shutdown grácil: al salir (`Ctrl+Q`), enviar `shutdown`, esperar cierre del Agente y cerrar el Almacén
    - _Requisitos: 9.1_

- [ ] 19. Integración extremo a extremo entre procesos
  - [ ] 19.1 Test de integración: lanzar binario real de TUI con un Agente stub que responde turnos preprogramados; enviar `user_message` "crea una tarea de prueba" y comprobar que la Tarea aparece en el Almacén y en el Panel_Tareas
    - _Requisitos: 1.1, 1.2, 1.3, 2.1, 10.1, 10.2_
  - [ ] 19.2 Test de integración: Agente stub que nunca responde; verificar que tras 30 s la TUI muestra el diálogo Reintentar/Cancelar
    - _Requisitos: 10.4_
  - [ ] 19.3 Test de integración: flujo completo de inferencia de Grupo a través del IPC real (mensaje sin Grupo + snapshot con tres Grupos) produce el `group_id` esperado y el `agent_reply` informa
    - _Requisitos: 15.9, 15.10, 15.11, 15.12_
  - [ ] 19.4 Test de integración: edición manual desde la TUI dispara un `COMMIT` y el Panel_Proximos se refresca dentro de 1 s (medición con `std::time::Instant`)
    - _Requisitos: 6.2, 13.10, 17.6_
  - [ ] 19.5 Test de integración de fallback de color: forzar `COLORTERM` no definido y `TERM=dumb` y comprobar que las entradas con Grupo incluyen el marcador `[nombre_grupo]`
    - _Requisitos: 16.4, 16.5_

- [ ] 20. CI y scripts de consistencia
  - [ ] 20.1 Completar el job `rust-tests` que ejecuta `cargo test --workspace` con `PROPTEST_CASES=200` y guarda regresiones
    - _Requisitos: 12.1_
  - [ ] 20.2 Completar el job `python-tests` que ejecuta `pytest` con `HYPOTHESIS_PROFILE=ci` (`max_examples=200`)
    - _Requisitos: 12.2_
  - [ ] 20.3 Completar el job `cross-ipc` que compila el binario de la TUI y ejecuta el test cruzado de round-trip IPC entre procesos reales
    - _Requisitos: 10.1, 10.2, 10.3_
  - [ ] 20.4 Completar el job `linters` con `cargo clippy -- -D warnings`, `ruff check` y `mypy`
    - _Requisitos: 12.1, 12.2_
  - [ ] 20.5 Añadir script `scripts/check_property_tags.py` que verifica que todo test marcado como property-based lleva la cabecera `Feature: terminal-day-organizer, Property <N>: <texto>` y que existe la propiedad correspondiente en `design.md`
    - _Requisitos: 10.3, 15.13, 8.5_

- [ ] 21. Checkpoint final
  - Ejecutar `cargo test`, `pytest`, los linters y los jobs `rust-tests`, `python-tests`, `cross-ipc`, `linters`. Asegurar que todos pasan. Preguntar al usuario si surgen dudas antes de dar por cerrado el ciclo de implementación.

## Notas

- Las tareas marcadas con `*` son opcionales. Saltarlas produce un MVP más rápido, pero las Propiedades P1–P26 sólo quedan verificadas cuando se implementan los tests de propiedad correspondientes.
- Cada tarea hace referencia a los requisitos granulares que valida para mantener trazabilidad con `requirements.md`.
- Los checkpoints (tareas 4, 10, 21) sirven para detener y consultar con el usuario en caso de dudas antes de avanzar.
- Los tests de propiedad usan `proptest` en Rust (`PROPTEST_CASES=200`) y `hypothesis` en Python (`max_examples=200`), según el mapeo de la sección Testing Strategy del diseño.
- La inferencia de Grupo, la serialización IPC y la exportación SQLite son funciones puras y se prueban por propiedades antes de cablearse al flujo del Agente y a la TUI.
- Este plan cubre únicamente la creación de artefactos de código y tests. La ejecución manual, la toma de métricas, el despliegue o la validación con usuarios finales quedan fuera del alcance de estas tareas.
