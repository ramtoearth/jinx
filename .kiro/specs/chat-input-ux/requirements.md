# Documento de Requisitos — Mejora de la experiencia del chat (Issues #2, #13, #14)

## Introducción

La experiencia actual del Panel_Chat presenta tres carencias que degradan la usabilidad para un usuario habitual de terminal:

1. **Sin cursor visible ni navegación** (Issue #13): No existe un cursor visible en el campo de entrada. El usuario no puede mover el punto de inserción con las flechas ni usar atajos de edición de línea estándar de terminal (Ctrl+A, Ctrl+E, Ctrl+U, Ctrl+K, Alt+B, Alt+F, etc.).
2. **Texto de entrada se corta** (Issue #2): Cuando el usuario escribe un mensaje largo en el campo de entrada, el texto se extiende hacia la derecha más allá del área visible sin saltar a la siguiente línea, haciéndolo ilegible.
3. **Sin scroll en el historial del chat** (Issue #14): La ventana del historial del chat no tiene mecanismo de scroll; el usuario solo puede ver los mensajes más recientes y pierde acceso a los anteriores.

Estas tres mejoras comparten el mismo componente (Panel_Chat) y se refuerzan mutuamente: un buen editor de línea necesita un campo multilinea visible, y un historial largo necesita scroll.

## Glosario

- **Cursor**: Posición actual de inserción de texto dentro del campo de entrada, con representación visual (bloque o barra).
- **Campo_Entrada**: Widget del Panel_Chat donde el usuario escribe su mensaje antes de enviarlo.
- **Historial_Chat**: Área del Panel_Chat que muestra los mensajes enviados y recibidos, por encima del Campo_Entrada.
- **Viewport_Input**: Ventana visible del Campo_Entrada; cuando el contenido excede el ancho o alto disponible, el viewport se desplaza para mantener el cursor visible.
- **Scroll_Historial**: Desplazamiento vertical controlado por el usuario sobre el Historial_Chat.
- **Atajos_Readline**: Conjunto de combinaciones de teclas estándar de terminales Unix para edición de línea (emacs keybindings).

## Requisitos

### Requisito 1: Cursor visible y navegación básica

**Historia de Usuario:** Como usuario de terminal, quiero ver dónde está mi cursor en el campo de entrada y poder moverlo con las flechas, para corregir errores sin tener que borrar todo lo escrito.

#### Criterios de Aceptación

1. WHILE el Panel_Chat está designado como Panel_Enfocado, THE TUI SHALL mostrar un cursor visual (bloque parpadeante o barra vertical) en la posición actual de inserción dentro del Campo_Entrada.
2. WHEN el usuario pulsa la tecla `←` (Left), THE TUI SHALL mover el cursor una posición hacia la izquierda sin borrar caracteres, hasta alcanzar el inicio del texto.
3. WHEN el usuario pulsa la tecla `→` (Right), THE TUI SHALL mover el cursor una posición hacia la derecha sin insertar caracteres, hasta alcanzar el final del texto.
4. WHEN el usuario pulsa la tecla `↑` (Up) dentro del Campo_Entrada, THE TUI SHALL mover el cursor a la línea anterior si el contenido tiene múltiples líneas; si el cursor ya está en la primera línea, el evento se descarta.
5. WHEN el usuario pulsa la tecla `↓` (Down) dentro del Campo_Entrada, THE TUI SHALL mover el cursor a la línea siguiente si el contenido tiene múltiples líneas; si el cursor ya está en la última línea, el evento se descarta.
6. WHEN el usuario inserta un carácter, THE TUI SHALL insertarlo en la posición actual del cursor y avanzar el cursor una posición a la derecha.
7. WHEN el usuario pulsa Backspace, THE TUI SHALL borrar el carácter inmediatamente anterior al cursor (a la izquierda) y retroceder el cursor una posición; si el cursor está al inicio, no se realiza ninguna acción.
8. WHEN el usuario pulsa Delete, THE TUI SHALL borrar el carácter en la posición actual del cursor (a la derecha); si el cursor está al final, no se realiza ninguna acción.

### Requisito 2: Atajos de edición estilo readline

**Historia de Usuario:** Como usuario experimentado de terminal, quiero usar los atajos de teclado habituales de readline/emacs para editar mi mensaje, para no tener que cambiar mi flujo de trabajo al escribir prompts.

#### Criterios de Aceptación

1. WHEN el usuario pulsa `Ctrl+A`, THE TUI SHALL mover el cursor al inicio del Campo_Entrada.
2. WHEN el usuario pulsa `Ctrl+E`, THE TUI SHALL mover el cursor al final del Campo_Entrada.
3. WHEN el usuario pulsa `Ctrl+U`, THE TUI SHALL borrar todo el texto desde el inicio del Campo_Entrada hasta la posición actual del cursor (kill backward).
4. WHEN el usuario pulsa `Ctrl+K`, THE TUI SHALL borrar todo el texto desde la posición actual del cursor hasta el final del Campo_Entrada (kill forward).
5. WHEN el usuario pulsa `Ctrl+W`, THE TUI SHALL borrar la palabra anterior al cursor (desde el cursor hasta el espacio o inicio anterior).
6. WHEN el usuario pulsa `Alt+B` (u `Option+B` en macOS), THE TUI SHALL mover el cursor al inicio de la palabra anterior.
7. WHEN el usuario pulsa `Alt+F` (u `Option+F` en macOS), THE TUI SHALL mover el cursor al final de la siguiente palabra.
8. WHEN el usuario pulsa `Ctrl+J`, THE TUI SHALL insertar un salto de línea (`\n`) en la posición actual del cursor, permitiendo al usuario escribir mensajes multilínea.
9. WHEN el usuario pulsa `Ctrl+L`, THE TUI SHALL borrar todo el contenido del Campo_Entrada (clear).

### Requisito 3: Campo de entrada multilínea con wrapping visible

**Historia de Usuario:** Como usuario, quiero que mi mensaje largo se muestre en múltiples líneas dentro del campo de entrada, para poder leer lo que estoy escribiendo sin que el texto se salga de la pantalla.

#### Criterios de Aceptación

1. THE Campo_Entrada SHALL renderizar el texto con word-wrap visual, ajustando las líneas al ancho disponible del widget sin truncar ni ocultar contenido.
2. WHEN el contenido del Campo_Entrada excede una línea de ancho, THE TUI SHALL mostrar el texto en las líneas siguientes dentro del Campo_Entrada, expandiendo el alto del widget dinámicamente hasta un máximo configurable.
3. THE Campo_Entrada SHALL tener un alto mínimo de 3 líneas y un alto máximo de 8 líneas (o el 40% del área del Panel_Chat, lo que sea menor).
4. WHEN el contenido del Campo_Entrada excede el alto máximo, THE TUI SHALL habilitar scroll interno dentro del Campo_Entrada, manteniendo el cursor siempre visible dentro del viewport.
5. THE TUI SHALL mantener el cursor siempre visible dentro del Viewport_Input: si el cursor se mueve fuera del área visible, el viewport se desplaza para mostrarlo.

### Requisito 4: Scroll en el historial del chat

**Historia de Usuario:** Como usuario, quiero poder desplazarme hacia arriba y abajo en el historial de mi conversación con el agente, para releer mensajes anteriores.

#### Criterios de Aceptación

1. WHILE el Panel_Chat está designado como Panel_Enfocado, WHEN el usuario pulsa `PgUp` (Page Up), THE TUI SHALL desplazar el Historial_Chat hacia arriba una página (la altura visible del historial menos 2 líneas de solapamiento).
2. WHILE el Panel_Chat está designado como Panel_Enfocado, WHEN el usuario pulsa `PgDn` (Page Down), THE TUI SHALL desplazar el Historial_Chat hacia abajo una página.
3. WHEN el Historial_Chat está desplazado hacia arriba y llega un nuevo mensaje (del usuario o del agente), THE TUI SHALL desplazar automáticamente el historial hasta el final para mostrar el mensaje más reciente.
4. WHEN el Historial_Chat está en el punto más bajo (mostrando los mensajes más recientes), el indicador de scroll no se muestra.
5. WHEN el Historial_Chat está desplazado hacia arriba (no está en el fondo), THE TUI SHALL mostrar un indicador visual (por ejemplo, "↑ más mensajes" o una flecha) que informe al usuario de que hay contenido más arriba.
6. WHILE el Panel_Chat está designado como Panel_Enfocado, WHEN el usuario pulsa `Shift+PgUp`, THE TUI SHALL desplazar el Historial_Chat al inicio (primer mensaje).
7. WHILE el Panel_Chat está designado como Panel_Enfocado, WHEN el usuario pulsa `Shift+PgDn`, THE TUI SHALL desplazar el Historial_Chat al final (mensaje más reciente).

### Requisito 5: Compatibilidad con macOS

**Historia de Usuario:** Como usuario de macOS, quiero que los atajos que usan Alt funcionen con la tecla Option, para que la experiencia sea nativa.

#### Criterios de Aceptación

1. THE TUI SHALL interpretar `Option+B` en macOS de la misma manera que `Alt+B` en Linux, moviendo el cursor una palabra hacia atrás.
2. THE TUI SHALL interpretar `Option+F` en macOS de la misma manera que `Alt+F` en Linux, moviendo el cursor una palabra hacia adelante.
3. IF el terminal del usuario no emite códigos de escape para Alt/Option (por configuración del emulador), THEN THE TUI SHALL documentar en la barra de estado o en la ayuda que el usuario debe habilitar "Option as Meta key" en su emulador de terminal.

### Requisito 6: Scroll con mouse

**Historia de Usuario:** Como usuario, quiero poder hacer scroll con la rueda del ratón en cualquier panel que tenga contenido scrollable, para navegar de forma natural sin recordar atajos.

#### Criterios de Aceptación

1. THE TUI SHALL habilitar la captura de eventos de mouse al iniciar.
2. WHEN el usuario hace scroll hacia arriba con la rueda del ratón sobre el Historial_Chat, THE TUI SHALL desplazar el historial hacia arriba (equivalente a PgUp pero con granularidad de 3 líneas por tick de rueda).
3. WHEN el usuario hace scroll hacia abajo con la rueda del ratón sobre el Historial_Chat, THE TUI SHALL desplazar el historial hacia abajo (equivalente a PgDn con granularidad de 3 líneas por tick de rueda).
4. WHEN el usuario hace scroll con la rueda del ratón sobre el Campo_Entrada (cuando tiene scroll interno), THE TUI SHALL desplazar el viewport del input sin mover el cursor.
5. WHEN el usuario hace scroll con la rueda del ratón sobre el Panel_Tareas, THE TUI SHALL mover el cursor de selección arriba o abajo en la lista de tareas.
6. WHEN el usuario hace scroll con la rueda del ratón sobre el Panel_Calendario, THE TUI SHALL mover el cursor de selección arriba o abajo en la lista de eventos.
7. WHEN el usuario hace scroll con la rueda del ratón sobre el Panel_Proximos, THE TUI SHALL mover el cursor de selección arriba o abajo en la lista de grupos/próximos.

### Requisito 7: Preservación de compatibilidad existente

**Historia de Usuario:** Como usuario actual, quiero que los flujos que ya funcionan no se rompan con estos cambios.

#### Criterios de Aceptación

1. WHEN el usuario pulsa `Enter` y el Campo_Entrada contiene texto no vacío (trim), THE TUI SHALL enviar el mensaje al Agente exactamente como antes (Requisito 1 del documento original).
2. WHEN el usuario pulsa `Enter` y el Campo_Entrada contiene solo whitespace, THE TUI SHALL descartar el envío y mostrar el aviso "Mensaje vacío" en la barra de estado.
3. THE TUI SHALL mantener `Tab` / `Shift+Tab` como cambio de panel sin conflicto con los nuevos atajos del Campo_Entrada.
4. THE TUI SHALL mantener `Ctrl+Q` como salida global sin conflicto.
5. THE combinación `Ctrl+J` para salto de línea SHALL no enviar el mensaje (diferenciando Enter de Ctrl+J).
6. THE TUI SHALL seguir funcionando correctamente si el terminal no soporta mouse o si el usuario deshabilita la captura de mouse.
