# Documento de Diseño — Mejora de la experiencia del chat

## Overview

Esta mejora reemplaza el campo de entrada actual del Panel_Chat (un `String` plano sin cursor) con un componente `TextEditor` completo que modela el buffer como un `Vec<String>` de líneas con posición de cursor `(row, col)`, soporta atajos readline, wrapping visual y scroll tanto en el input como en el historial. El diseño se mantiene fiel a los principios del Organizador: la lógica de edición es pura y testable, sin I/O; la integración con Ratatui ocurre solo en render.

## Architecture

### Componentes nuevos

1. **`TextEditor`** (struct puro, en `tui/src/text_editor.rs`): Modelo del buffer de texto con cursor. Encapsula toda la lógica de edición (inserción, borrado, navegación, atajos readline). No depende de Ratatui ni de I/O.

2. **`ChatScrollState`** (struct puro, campo en `RuntimeState`): Mantiene el offset vertical del historial del chat para scroll controlado por el usuario.

3. **Modificaciones a `render_chat`** y **`handle_chat_key`** en `main.rs`: Adaptan la integración existente para usar `TextEditor` y `ChatScrollState`.

### Decisiones de diseño

1. **Buffer orientado a líneas (`Vec<String>`)**: Cada elemento es una línea lógica separada por `\n`. Esto simplifica la navegación vertical (Up/Down), el manejo de Ctrl+J (insertar newline) y el cálculo de wrapping visual. Las líneas vacías se representan como `String::new()`.

2. **Cursor como `(row: usize, col: usize)`**: `row` indexa en el `Vec<String>`, `col` indexa el byte offset válido para char boundary en la línea actual. Se mantiene siempre clampado a posiciones válidas.

3. **Word-wrap visual en render, no en el modelo**: El `TextEditor` almacena el texto sin wrapping. El render calcula las líneas visuales al dibujar. Esto evita sincronizar estado de wrapping ante cada edición y simplifica los tests del modelo.

4. **Scroll del historial como offset simple**: Un `usize` que indica cuántas líneas visuales "por arriba" del fondo está el viewport del historial. Valor 0 = pegado al fondo (comportamiento actual). Al llegar un nuevo mensaje se resetea a 0.

5. **Alt/Option como `KeyModifiers::ALT`**: crossterm ya emite `ALT` para Option en macOS cuando el emulador está configurado con "Option as Meta". No necesitamos detección especial de plataforma; solo documentamos el requisito del emulador.

## Components and Interfaces

### `TextEditor`

```rust
// tui/src/text_editor.rs

pub struct TextEditor {
    lines: Vec<String>,
    cursor_row: usize,
    cursor_col: usize,  // byte offset (char-boundary safe)
}

impl TextEditor {
    pub fn new() -> Self;
    pub fn from_string(s: &str) -> Self;
    pub fn to_string(&self) -> String;  // joins lines with \n
    pub fn is_empty(&self) -> bool;     // true if all lines are empty/whitespace
    pub fn clear(&mut self);

    // Cursor position
    pub fn cursor(&self) -> (usize, usize);
    pub fn cursor_row(&self) -> usize;
    pub fn cursor_col(&self) -> usize;
    pub fn line_count(&self) -> usize;
    pub fn current_line(&self) -> &str;

    // Basic editing
    pub fn insert_char(&mut self, c: char);
    pub fn insert_newline(&mut self);  // Ctrl+J
    pub fn backspace(&mut self);
    pub fn delete(&mut self);

    // Navigation
    pub fn move_left(&mut self);
    pub fn move_right(&mut self);
    pub fn move_up(&mut self);
    pub fn move_down(&mut self);
    pub fn move_home(&mut self);       // Ctrl+A — start of buffer
    pub fn move_end(&mut self);        // Ctrl+E — end of buffer

    // Readline shortcuts
    pub fn kill_to_start(&mut self);   // Ctrl+U — delete from start to cursor
    pub fn kill_to_end(&mut self);     // Ctrl+K — delete from cursor to end
    pub fn kill_word_back(&mut self);  // Ctrl+W — delete previous word
    pub fn move_word_back(&mut self);  // Alt+B — cursor to start of prev word
    pub fn move_word_forward(&mut self); // Alt+F — cursor to end of next word
}
```

**Reglas de navegación por palabras:**
- Una "palabra" se define como una secuencia contigua de caracteres no-whitespace.
- `move_word_back` coloca el cursor al inicio de la palabra anterior (o al inicio del buffer si no hay palabra anterior).
- `move_word_forward` coloca el cursor al final de la siguiente palabra (o al final del buffer si no hay siguiente palabra).
- El movimiento por palabras NO cruza límites de línea para mantener predecibilidad (se detiene al inicio/final de la línea actual). Esto simplifica la implementación y coincide con el comportamiento de muchos editores de terminal.

**Invariantes del cursor:**
- `cursor_row < lines.len()` siempre.
- `cursor_col <= lines[cursor_row].len()` siempre.
- `cursor_col` es siempre un char boundary válido de `lines[cursor_row]`.

### `ChatScrollState`

```rust
// Dentro de RuntimeState en main.rs

struct ChatScrollState {
    offset: usize,  // líneas visuales desde el fondo; 0 = pegado al fondo
}

impl ChatScrollState {
    fn page_up(&mut self, page_height: usize, total_lines: usize);
    fn page_down(&mut self, page_height: usize);
    fn jump_to_top(&mut self, total_lines: usize, page_height: usize);
    fn jump_to_bottom(&mut self);
    fn is_at_bottom(&self) -> bool;
    fn reset_to_bottom(&mut self);
}
```

### Cambios en `RuntimeState`

```rust
struct RuntimeState {
    // Reemplaza: chat_input: String
    chat_editor: TextEditor,
    chat_scroll: ChatScrollState,
    // ... todo lo demás sin cambios
}
```

### Cambios en `handle_chat_key`

El dispatch de teclas del Panel_Chat se expande para manejar los nuevos atajos:

| Tecla | Acción |
|-------|--------|
| `Enter` | Enviar mensaje (si no vacío) |
| `Ctrl+J` | `editor.insert_newline()` |
| `←` | `editor.move_left()` |
| `→` | `editor.move_right()` |
| `↑` | `editor.move_up()` |
| `↓` | `editor.move_down()` |
| `Ctrl+A` | `editor.move_home()` |
| `Ctrl+E` | `editor.move_end()` |
| `Ctrl+U` | `editor.kill_to_start()` |
| `Ctrl+K` | `editor.kill_to_end()` |
| `Ctrl+W` | `editor.kill_word_back()` |
| `Ctrl+L` | `editor.clear()` |
| `Alt+B` | `editor.move_word_back()` |
| `Alt+F` | `editor.move_word_forward()` |
| `Backspace` | `editor.backspace()` |
| `Delete` | `editor.delete()` |
| `PgUp` | `chat_scroll.page_up(...)` |
| `PgDn` | `chat_scroll.page_down(...)` |
| `Shift+PgUp` | `chat_scroll.jump_to_top(...)` |
| `Shift+PgDn` | `chat_scroll.jump_to_bottom()` |
| `Char(c)` | `editor.insert_char(c)` |

### Cambios en `render_chat`

1. **Campo_Entrada dinámico**: El campo de entrada se redimensiona verticalmente. Se calcula el número de líneas visuales que ocupa el contenido (aplicando word-wrap al ancho del widget) y se ajusta el `Constraint::Length(...)` entre el mínimo (3) y el máximo (8 o 40% del área).

2. **Cursor visual**: Tras renderizar el Paragraph del input, se llama a `frame.set_cursor_position(x, y)` con las coordenadas absolutas del cursor (calculadas a partir del offset de wrapping y la posición del cursor en el modelo).

3. **Scroll en el historial**: En lugar de `all_lines[start..]` con `start = total - height`, se usa `all_lines[start..start+height]` donde `start = total - height - scroll_offset`. Se clampea para no salir de rango.

4. **Indicador de scroll**: Si `chat_scroll.offset > 0`, se renderiza una línea indicadora `"↑ mensajes anteriores"` en la parte superior del historial.

### Word-wrap del Campo_Entrada para renderizado

El word-wrap del Campo_Entrada en render sigue el mismo algoritmo `word_wrap` ya existente pero aplicado línea a línea del buffer. Para calcular la posición visual del cursor:

1. Iterar por las líneas lógicas del buffer hasta `cursor_row`.
2. Para `cursor_row`, aplicar word-wrap y determinar en qué línea visual y columna visual cae `cursor_col`.
3. Sumar las líneas visuales acumuladas para obtener la posición Y relativa dentro del widget.

### Scroll interno del Campo_Entrada

Si el total de líneas visuales excede el alto máximo del widget:
- Se mantiene un `input_scroll_offset: usize` (líneas visuales desde el tope).
- Tras cada edición, se recalcula la posición visual del cursor y se ajusta `input_scroll_offset` para que el cursor quede visible (dentro del rango `[offset, offset + height)`).

### Soporte de mouse

crossterm ya soporta captura de eventos de mouse con `enable_mouse_capture()` / `disable_mouse_capture()`. Se habilita al iniciar la TUI y se deshabilita al salir (junto con `LeaveAlternateScreen`).

**Evento `MouseEvent::ScrollUp` / `ScrollDown`:**
- Se detecta la posición `(column, row)` del evento para determinar en qué panel ocurre (comparando contra los `Rect` de cada panel del layout actual).
- Según el panel:
  - **Historial_Chat**: ajusta `chat_scroll.offset` en ±3 líneas.
  - **Campo_Entrada** (si tiene scroll interno): ajusta `input_scroll_offset` en ±1 línea.
  - **Panel_Tareas**: mueve `task_cursor` ±1.
  - **Panel_Calendario**: mueve `calendar_cursor` ±1.
  - **Panel_Proximos**: mueve `group_cursor` ±1.

**Granularidad**: 3 líneas por tick de rueda para el historial (coincide con la experiencia habitual de terminales); 1 posición para listas de elementos discretos.

**Fallback sin mouse**: Si `enable_mouse_capture()` falla, se ignora silenciosamente y la TUI funciona solo con teclado. Los eventos `Event::Mouse` simplemente no llegarán.

## Error Handling

- Si `cursor_col` queda fuera de rango por un bug, se clampea silenciosamente a `lines[row].len()`. No se produce un crash.
- Si `cursor_row` queda fuera de rango (no debería ocurrir por las invariantes), se clampea a `lines.len() - 1`.
- El scroll del historial se clampea en `[0, max(0, total_lines - page_height)]`.

## Testing Strategy

### Tests unitarios del `TextEditor`

Tests deterministas (no property-based) que cubren:
- Inserción de caracteres en posición intermedia.
- Backspace y Delete en bordes (inicio, final, entre líneas).
- Navegación Left/Right con clamping.
- Navegación Up/Down entre líneas de distinto largo.
- Ctrl+A, Ctrl+E: salto a inicio/fin.
- Ctrl+U, Ctrl+K: kill backward/forward.
- Ctrl+W: kill word (distintos casos de frontera).
- Alt+B, Alt+F: movimiento por palabras.
- Ctrl+J: inserción de newline y split de línea.
- `to_string()` / `from_string()` round-trip.
- `is_empty()` con whitespace-only content.

### Tests del scroll del historial

- Page up/down con offset correcto.
- Clamping en los extremos.
- Reset automático al recibir nuevo mensaje.

### Tests de integración visual (manuales)

- Verificar que el cursor parpadea en la posición correcta.
- Verificar que el text-wrapping del input es visualmente correcto.
- Verificar que PgUp/PgDn en el historial funciona fluidamente.
