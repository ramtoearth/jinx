# Plan de Implementación — Mejora de la experiencia del chat (Issues #2, #13, #14)

## Resumen

Este plan implementa las tres mejoras de UX del Panel_Chat: cursor visible con navegación y atajos readline, campo de entrada multilínea con word-wrap, scroll en el historial, y scroll con mouse en todos los paneles. Las tareas se ordenan bottom-up: primero el modelo puro (`TextEditor`), luego la integración con el render y los eventos, y finalmente el soporte de mouse.

## Tareas

- [x] 1. Crear el módulo `TextEditor` con buffer y cursor básico
  - [x] 1.1 Crear archivo `tui/src/text_editor.rs` con el struct `TextEditor { lines: Vec<String>, cursor_row: usize, cursor_col: usize }` y constructores `new()`, `from_string(s)`, `to_string()`, `is_empty()`, `clear()`
    - _Requisitos: 1.1, 1.6, 3.1_
  - [x] 1.2 Implementar `insert_char(c)`: insertar en `lines[cursor_row]` en la posición `cursor_col`, avanzar cursor; manejar char boundaries correctamente para UTF-8
    - _Requisitos: 1.6_
  - [x] 1.3 Implementar `backspace()`: si cursor_col > 0, borrar el char anterior y retroceder cursor; si cursor_col == 0 y cursor_row > 0, unir la línea actual con la anterior (merge lines)
    - _Requisitos: 1.7_
  - [x] 1.4 Implementar `delete()`: si cursor_col < len de la línea actual, borrar el char en cursor_col; si cursor_col == len y hay línea siguiente, unir la siguiente línea con la actual
    - _Requisitos: 1.8_
  - [x] 1.5 Implementar `insert_newline()`: partir `lines[cursor_row]` en cursor_col, la parte derecha se convierte en una nueva línea en cursor_row+1, avanzar cursor_row y poner cursor_col a 0
    - _Requisitos: 2.8_
  - [x] 1.6 Registrar el módulo en `tui/src/lib.rs` con `pub mod text_editor;`
    - _Requisitos: 1.1_

- [x] 2. Navegación básica del cursor
  - [x] 2.1 Implementar `move_left()`: retroceder cursor_col un char; si está al inicio de línea y hay línea anterior, subir al final de la línea anterior
    - _Requisitos: 1.2_
  - [x] 2.2 Implementar `move_right()`: avanzar cursor_col un char; si está al final de línea y hay línea siguiente, bajar al inicio de la línea siguiente
    - _Requisitos: 1.3_
  - [x] 2.3 Implementar `move_up()`: si cursor_row > 0, subir a la línea anterior manteniendo cursor_col (clamped a la longitud de la nueva línea)
    - _Requisitos: 1.4_
  - [x] 2.4 Implementar `move_down()`: si cursor_row < lines.len() - 1, bajar a la línea siguiente manteniendo cursor_col (clamped)
    - _Requisitos: 1.5_
  - [x] 2.5 Implementar `move_home()`: poner cursor_row=0, cursor_col=0 (inicio del buffer completo)
    - _Requisitos: 2.1_
  - [x] 2.6 Implementar `move_end()`: poner cursor_row al última línea, cursor_col al final de esa línea
    - _Requisitos: 2.2_

- [x] 3. Atajos readline
  - [x] 3.1 Implementar `kill_to_start()`: borrar desde inicio de la línea actual hasta cursor_col; poner cursor_col a 0
    - _Requisitos: 2.3_
  - [x] 3.2 Implementar `kill_to_end()`: borrar desde cursor_col hasta el final de la línea actual
    - _Requisitos: 2.4_
  - [x] 3.3 Implementar `kill_word_back()`: encontrar el inicio de la palabra anterior (retroceder whitespace, luego retroceder non-whitespace) y borrar desde ahí hasta cursor_col
    - _Requisitos: 2.5_
  - [x] 3.4 Implementar `move_word_back()`: mover cursor al inicio de la palabra anterior sin borrar (retroceder whitespace, luego retroceder non-whitespace); detenerse al inicio de la línea
    - _Requisitos: 2.6_
  - [x] 3.5 Implementar `move_word_forward()`: mover cursor al final de la siguiente palabra sin borrar (avanzar whitespace, luego avanzar non-whitespace); detenerse al final de la línea
    - _Requisitos: 2.7_

- [x] 4. Tests unitarios del TextEditor
  - [x] 4.1 Tests de inserción: insertar en posición intermedia, al inicio, al final, caracteres multibyte (emoji, acentos)
    - _Requisitos: 1.6_
  - [x] 4.2 Tests de backspace/delete: en bordes de línea (merge), al inicio (no-op para backspace), al final (no-op para delete), entre líneas
    - _Requisitos: 1.7, 1.8_
  - [x] 4.3 Tests de navegación: left/right cruzando líneas, up/down con clamping de columna, home/end
    - _Requisitos: 1.2, 1.3, 1.4, 1.5, 2.1, 2.2_
  - [x] 4.4 Tests de kill: Ctrl+U, Ctrl+K, Ctrl+W con distintos contenidos y posiciones de cursor
    - _Requisitos: 2.3, 2.4, 2.5_
  - [x] 4.5 Tests de movimiento por palabras: Alt+B, Alt+F con múltiples palabras, whitespace, inicio/fin de línea
    - _Requisitos: 2.6, 2.7_
  - [x] 4.6 Tests de newline: insert_newline parte correctamente la línea, to_string preserva los \n, from_string round-trip
    - _Requisitos: 2.8_
  - [x] 4.7 Test de is_empty: con whitespace-only y con contenido real
    - _Requisitos: 7.2_

- [x] 5. Integrar TextEditor en RuntimeState y handle_chat_key
  - [x] 5.1 Reemplazar `chat_input: String` por `chat_editor: TextEditor` en `RuntimeState`; actualizar la inicialización
    - _Requisitos: 1.1_
  - [x] 5.2 Reescribir `handle_chat_key` para despachar todas las teclas nuevas al TextEditor: Enter (enviar), Ctrl+J (newline), ←/→/↑/↓ (nav), Ctrl+A/E/U/K/W/L, Alt+B/F, Backspace, Delete, Char(c)
    - _Requisitos: 1.2–1.8, 2.1–2.9, 7.1–7.5_
  - [x] 5.3 Actualizar `send_user_message` para usar `chat_editor.to_string()` como texto del mensaje y `chat_editor.clear()` tras enviar
    - _Requisitos: 7.1_
  - [x] 5.4 Asegurar que la guardia de mensaje vacío usa `chat_editor.to_string().trim().is_empty()`
    - _Requisitos: 7.2_

- [x] 6. Renderizado del Campo_Entrada multilínea con cursor visible
  - [x] 6.1 En `render_chat`, calcular el alto dinámico del Campo_Entrada: min(max(3, líneas_visuales + 2), min(8, 40% del área)); usar ese valor en el `Constraint::Length(...)`
    - _Requisitos: 3.2, 3.3_
  - [x] 6.2 Implementar el renderizado del contenido del editor con word-wrap visual línea a línea; mapear la posición lógica del cursor (row, col) a coordenadas visuales (x, y) dentro del widget
    - _Requisitos: 3.1, 3.5_
  - [x] 6.3 Llamar a `frame.set_cursor_position(x, y)` con las coordenadas absolutas del cursor para que el terminal muestre el cursor parpadeante
    - _Requisitos: 1.1_
  - [x] 6.4 Implementar scroll interno del input: si las líneas visuales exceden el alto máximo, mantener un `input_scroll_offset` que se ajusta automáticamente para que el cursor quede visible
    - _Requisitos: 3.4, 3.5_

- [x] 7. Scroll en el historial del chat
  - [x] 7.1 Añadir `chat_scroll: usize` (offset desde el fondo, 0 = pegado al fondo) al `RuntimeState`
    - _Requisitos: 4.1_
  - [x] 7.2 Implementar PgUp en handle_chat_key: incrementar `chat_scroll` en `page_height - 2`, clampeado al máximo posible
    - _Requisitos: 4.1_
  - [x] 7.3 Implementar PgDn en handle_chat_key: decrementar `chat_scroll` en `page_height - 2`, mínimo 0
    - _Requisitos: 4.2_
  - [x] 7.4 Implementar Shift+PgUp (jump to top): poner `chat_scroll` al máximo
    - _Requisitos: 4.6_
  - [x] 7.5 Implementar Shift+PgDn (jump to bottom): poner `chat_scroll` a 0
    - _Requisitos: 4.7_
  - [x] 7.6 En `render_chat`, calcular el rango de líneas visibles como `[total - height - offset .. total - offset]` en lugar de `[total - height .. total]`
    - _Requisitos: 4.1, 4.2_
  - [x] 7.7 Resetear `chat_scroll` a 0 cuando se añade un nuevo mensaje al historial (envío del usuario o respuesta del agente)
    - _Requisitos: 4.3_
  - [x] 7.8 Renderizar indicador "↑ mensajes anteriores" en el tope del historial cuando `chat_scroll` > 0
    - _Requisitos: 4.5_

- [x] 8. Soporte de mouse
  - [x] 8.1 Añadir `crossterm::event::EnableMouseCapture` en el setup del terminal (`main()`) y `DisableMouseCapture` en el cleanup
    - _Requisitos: 6.1_
  - [x] 8.2 En el event loop, capturar `Event::Mouse(MouseEvent { kind: MouseEventKind::ScrollUp | ScrollDown, column, row, .. })` y determinar en qué panel cae según los `Rect` del layout actual
    - _Requisitos: 6.2, 6.3_
  - [x] 8.3 Almacenar los `Rect` de cada panel tras cada render (en un campo `panel_rects` del RuntimeState o calculado inline) para hit-testing del mouse
    - _Requisitos: 6.2_
  - [x] 8.4 Implementar scroll del historial con mouse: ScrollUp incrementa `chat_scroll` en 3, ScrollDown lo decrementa en 3
    - _Requisitos: 6.2, 6.3_
  - [x] 8.5 Implementar scroll del Campo_Entrada con mouse: ScrollUp/Down ajusta `input_scroll_offset` en ±1
    - _Requisitos: 6.4_
  - [x] 8.6 Implementar scroll en Panel_Tareas: ScrollUp decrementa `task_cursor`, ScrollDown lo incrementa (con clamping)
    - _Requisitos: 6.5_
  - [x] 8.7 Implementar scroll en Panel_Calendario: ScrollUp decrementa `calendar_cursor`, ScrollDown lo incrementa (con clamping)
    - _Requisitos: 6.6_
  - [x] 8.8 Implementar scroll en Panel_Proximos: ScrollUp decrementa `group_cursor`, ScrollDown lo incrementa (con clamping)
    - _Requisitos: 6.7_

- [ ] 9. Tests de integración y pulido
  - [ ] 9.1 Test: enviar mensaje multilínea (con Ctrl+J) y verificar que llega completo al agente con los \n preservados
    - _Requisitos: 2.8, 7.1_
  - [ ] 9.2 Test: verificar que PgUp/PgDn adjustan el offset correctamente y que el clamping funciona en los extremos
    - _Requisitos: 4.1, 4.2_
  - [ ] 9.3 Test: verificar que nuevo mensaje reseta el scroll a 0
    - _Requisitos: 4.3_
  - [ ] 9.4 Verificación manual: compilar y probar en terminal real que el cursor es visible, los atajos funcionan, el wrapping es correcto, y el mouse scroll funciona en todos los paneles
    - _Requisitos: 1.1, 3.1, 4.1, 5.1, 5.2, 6.1–6.7_
  - [ ] 9.5 Verificar que Tab/Shift+Tab, Ctrl+Q, Ctrl+P siguen funcionando sin conflicto
    - _Requisitos: 7.3, 7.4_
  - [ ] 9.6 Verificar en macOS que Option+B y Option+F funcionan cuando el emulador tiene "Option as Meta" habilitado
    - _Requisitos: 5.1, 5.2_

## Notas

- El `TextEditor` es un struct puro sin dependencia de Ratatui, lo que permite testearlo exhaustivamente sin terminal.
- El word-wrap solo se aplica en render (no modifica el buffer), simplificando la lógica de cursor.
- El scroll del historial es un offset desde el fondo (0 = más reciente visible), lo que hace trivial el auto-scroll al recibir mensajes nuevos.
- La captura de mouse se habilita opcionalmente; si falla, la TUI funciona normalmente solo con teclado.
- Los atajos readline (Ctrl+U/K/W, Alt+B/F) no implementan kill-ring (yank/paste del texto cortado). Esto se podría añadir en el futuro pero no es parte de este scope.
- `Ctrl+E` actualmente no tiene conflicto porque el diseño original lo reservaba para el diálogo de exportación, pero ese atajo se activa solo como tecla global fuera del Panel_Chat. Dentro del chat, Ctrl+E es "ir al final".
