# Documento de Requisitos

## Introducción

El Organizador de Día en Terminal es una aplicación local que combina una interfaz de terminal (TUI) construida en Rust con Ratatui y un agente conversacional escrito en Python con Strands Agents. El usuario, un usuario habitual de terminal, interactúa con el Organizador de dos formas complementarias: mediante un chat en lenguaje natural con el Agente y mediante una TUI dividida en paneles enfocables —Panel_Chat, Panel_Tareas, Panel_Calendario y Panel_Proximos— desde los que puede editar directamente las Tareas y los Eventos con atajos de teclado. El Agente interpreta referencias temporales relativas (por ejemplo, "hoy en dos horas"), persiste los datos localmente en SQLite, ayuda a priorizar pendientes y muestra los próximos compromisos junto al chat para evitar perder el hilo del día. Las Tareas y los Eventos pueden organizarse en Grupos configurables (por ejemplo "trabajo" o "vida personal"), cada uno con su propio Color_Grupo para identificarlos visualmente en los paneles. Los datos pueden exportarse a Markdown o a SQLite para respaldo o para compartirlos.

## Glosario

- **Organizador**: Aplicación completa que integra la TUI, el Agente y el Almacén.
- **TUI**: Interfaz de usuario en terminal, implementada en Rust con Ratatui. Incluye el chat y los paneles de visualización.
- **Agente**: Agente conversacional implementado en Python con el framework Strands Agents. Interpreta los mensajes del usuario y ejecuta acciones sobre el Almacén.
- **Almacén**: Base de datos local SQLite donde se persisten Tareas, Eventos y Grupos.
- **Exportador**: Componente que genera archivos de salida en formato Markdown o SQLite a partir del contenido del Almacén.
- **Canal_IPC**: Mecanismo de comunicación entre la TUI y el Agente (por ejemplo, JSON sobre stdio o socket local Unix).
- **Tarea**: Pendiente registrado por el usuario. Campos: identificador, título, Prioridad, Estado_Tarea, fecha de creación, fecha límite opcional e identificador de Grupo opcional.
- **Evento**: Entrada del calendario. Campos: identificador, título, fecha de inicio, hora de inicio, duración opcional e identificador de Grupo opcional.
- **Prioridad**: Valor del conjunto {alta, media, baja}.
- **Estado_Tarea**: Valor del conjunto {pendiente, completada, cancelada}.
- **Grupo**: Categoría nombrada (por ejemplo "trabajo" o "vida personal") usada para clasificar Tareas y Eventos. Campos: identificador, nombre único y Color_Grupo.
- **Color_Grupo**: Valor de color asociado a un Grupo, expresado como un código admitido por la TUI (por ejemplo, código de color ANSI o código hexadecimal convertible al espacio de color del terminal).
- **Panel_Chat**: Sección de la TUI que muestra el historial del chat y el campo de entrada de texto dirigido al Agente.
- **Panel_Tareas**: Sección de la TUI que muestra la lista de Tareas con sus campos principales y permite operar sobre cada Tarea mediante atajos de teclado.
- **Panel_Calendario**: Sección de la TUI que muestra los Eventos y las fechas límite de las Tareas en formato de calendario y permite operar sobre cada Evento mediante atajos de teclado.
- **Panel_Proximos**: Sección de la TUI que muestra los Eventos y Tareas con vencimiento inmediato.
- **Panel_Enfocado**: Panel de la TUI que en un instante dado recibe la entrada de teclado del usuario.

## Requisitos

### Requisito 1: Chat conversacional con el Agente

**Historia de Usuario:** Como usuario de terminal, quiero conversar con un agente mediante un chat dentro de la TUI, para poder pedir acciones sobre mis pendientes y eventos en lenguaje natural.

#### Criterios de Aceptación

1. WHEN el usuario inicia el Organizador, THE TUI SHALL mostrar el Panel_Chat con un historial de mensajes y un campo de entrada de texto.
2. WHEN el usuario envía un mensaje desde el campo de entrada del Panel_Chat, THE TUI SHALL transmitir el mensaje al Agente a través del Canal_IPC dentro de 200 ms.
3. WHEN el Agente produce una respuesta, THE TUI SHALL mostrar la respuesta en el historial del Panel_Chat etiquetada con el rol "agente".
4. IF la entrega del mensaje al Agente falla, THEN THE TUI SHALL mostrar un mensaje de error con la causa y ofrecer reintentar el envío.

### Requisito 2: Gestión de Tareas

**Historia de Usuario:** Como usuario, quiero crear, listar, completar, modificar y eliminar Tareas desde el chat, para gestionar mi lista de pendientes sin salir de la terminal.

#### Criterios de Aceptación

1. WHEN el usuario solicita crear una Tarea en lenguaje natural, THE Agente SHALL extraer título, Prioridad, fecha límite y Grupo cuando se mencionen, y persistir la Tarea en el Almacén con Estado_Tarea igual a "pendiente".
2. WHEN el usuario solicita listar las Tareas pendientes, THE Agente SHALL recuperar del Almacén todas las Tareas con Estado_Tarea igual a "pendiente" y presentarlas ordenadas primero por Prioridad (alta, media, baja) y después por fecha límite ascendente.
3. WHEN el usuario indica que una Tarea está completada, THE Agente SHALL actualizar en el Almacén el Estado_Tarea de esa Tarea a "completada".
4. WHEN el usuario solicita eliminar una Tarea, THE Agente SHALL eliminar esa Tarea del Almacén.
5. WHEN el usuario solicita modificar el título, la Prioridad, la fecha límite o el Grupo de una Tarea, THE Agente SHALL actualizar el campo indicado de esa Tarea en el Almacén.
6. IF el usuario solicita una operación sobre una Tarea que no existe en el Almacén, THEN THE Agente SHALL responder con un mensaje indicando que la Tarea no fue encontrada y no modificar el Almacén.
7. IF el usuario crea una Tarea sin especificar Prioridad, THEN THE Agente SHALL asignar Prioridad igual a "media".

### Requisito 3: Gestión de Eventos de calendario

**Historia de Usuario:** Como usuario, quiero crear, listar, modificar y eliminar Eventos de mi calendario desde el chat, para tener claridad sobre mis compromisos.

#### Criterios de Aceptación

1. WHEN el usuario solicita crear un Evento indicando fecha y hora, THE Agente SHALL persistir el Evento en el Almacén con título, fecha de inicio, hora de inicio y Grupo cuando se mencione.
2. WHEN el usuario solicita crear un Evento usando una referencia temporal relativa, THE Agente SHALL calcular la fecha y la hora absolutas a partir de la fecha y hora actuales del sistema y persistir el Evento con esos valores absolutos.
3. WHEN el usuario solicita listar los Eventos de un rango de fechas, THE Agente SHALL recuperar del Almacén los Eventos cuya fecha de inicio esté dentro de ese rango y presentarlos ordenados por fecha y hora de inicio ascendentes.
4. WHEN el usuario solicita eliminar un Evento, THE Agente SHALL eliminar ese Evento del Almacén.
5. WHEN el usuario solicita modificar el título, la fecha, la hora de inicio, la duración o el Grupo de un Evento, THE Agente SHALL actualizar el campo indicado de ese Evento en el Almacén.
6. IF el usuario solicita crear un Evento sin fecha o sin hora, THEN THE Agente SHALL pedir al usuario los datos faltantes antes de persistir el Evento.

### Requisito 4: Priorización de Tareas

**Historia de Usuario:** Como usuario, quiero que el Agente me ayude a priorizar mis Tareas pendientes, para decidir en qué trabajar a continuación.

#### Criterios de Aceptación

1. WHEN el usuario solicita una priorización de Tareas pendientes, THE Agente SHALL generar un orden sugerido en el que las Tareas con Prioridad "alta" aparecen antes que las de Prioridad "media" y estas antes que las de Prioridad "baja", usando la fecha límite ascendente como desempate cuando exista.
2. WHEN el usuario acepta una reasignación de Prioridades propuesta por el Agente, THE Agente SHALL actualizar en el Almacén la Prioridad de cada Tarea afectada según la propuesta aceptada.

### Requisito 5: Conciencia temporal del Agente

**Historia de Usuario:** Como usuario, quiero que el Agente entienda referencias a fechas y horas relativas, para no tener que escribir fechas completas cada vez.

#### Criterios de Aceptación

1. THE Agente SHALL obtener la fecha y la hora actuales del sistema antes de procesar cada mensaje del usuario.
2. WHEN el mensaje del usuario contiene una referencia temporal relativa (por ejemplo "hoy", "mañana", "en dos horas", "el viernes"), THE Agente SHALL convertir esa referencia en una fecha y hora absolutas usando la fecha y hora actuales del sistema antes de crear o actualizar una Tarea o un Evento.
3. IF el Agente no puede resolver de forma unívoca una referencia temporal del mensaje, THEN THE Agente SHALL pedir al usuario que aclare la fecha y la hora antes de persistir cambios.

### Requisito 6: Panel de próximos pendientes y eventos

**Historia de Usuario:** Como usuario, quiero ver en la TUI mis próximos Eventos y Tareas junto al chat, para no perder el hilo del día.

#### Criterios de Aceptación

1. THE TUI SHALL mostrar el Panel_Proximos junto al Panel_Chat con los Eventos cuya fecha y hora de inicio estén dentro de las próximas 24 horas y las Tareas cuya fecha límite esté dentro de las próximas 24 horas.
2. WHEN una Tarea o un Evento es creado, modificado o eliminado en el Almacén, THE TUI SHALL actualizar el Panel_Proximos dentro de 1 segundo.
3. THE TUI SHALL mostrar cada entrada del Panel_Proximos con título, fecha y hora, y, para Tareas, la Prioridad.
4. WHERE una Tarea o un Evento mostrado en el Panel_Proximos tiene un Grupo asignado, THE TUI SHALL renderizar esa entrada usando el Color_Grupo del Grupo asignado.

### Requisito 7: Persistencia local en SQLite

**Historia de Usuario:** Como usuario, quiero que mis Tareas y Eventos queden guardados localmente entre sesiones, para no perder información al cerrar la aplicación.

#### Criterios de Aceptación

1. THE Almacén SHALL residir en un archivo SQLite dentro del directorio de configuración del usuario.
2. WHEN el Agente o la TUI crea, actualiza o elimina una Tarea, un Evento o un Grupo, THE Almacén SHALL confirmar la transacción en el archivo SQLite antes de que el Agente o la TUI envíe la respuesta correspondiente al usuario.
3. WHEN el Organizador se inicia, THE Almacén SHALL cargar las Tareas, Eventos y Grupos previamente guardados y ponerlos a disposición del Agente y de la TUI.
4. IF el archivo SQLite no existe al iniciar el Organizador, THEN THE Almacén SHALL crear el archivo y aplicar el esquema inicial.

### Requisito 8: Exportación de datos

**Historia de Usuario:** Como usuario, quiero exportar mis Tareas y Eventos a Markdown o a SQLite, para respaldar o compartir mi información.

#### Criterios de Aceptación

1. WHEN el usuario solicita exportar sus datos a Markdown, THE Exportador SHALL generar un archivo Markdown con una sección de Tareas, una sección de Eventos y una sección de Grupos, incluyendo todos los campos almacenados de cada entrada.
2. WHEN el usuario solicita exportar sus datos a SQLite, THE Exportador SHALL producir un archivo SQLite con el mismo esquema que el Almacén y los mismos registros, incluidos los Grupos.
3. WHERE el usuario especifica una ruta de salida, THE Exportador SHALL escribir el archivo exportado en esa ruta.
4. IF la ruta de salida no es escribible, THEN THE Exportador SHALL devolver un error indicando la causa y no escribir ningún archivo.
5. FOR ALL conjuntos de Tareas, Eventos y Grupos del Almacén, exportar a SQLite con el Exportador y luego importar ese archivo a un Almacén vacío SHALL producir un conjunto de registros equivalente al original (propiedad round-trip).

### Requisito 9: Ejecución local

**Historia de Usuario:** Como usuario preocupado por la privacidad, quiero que el Organizador funcione localmente, para mantener control sobre mis datos.

#### Criterios de Aceptación

1. THE Organizador SHALL arrancar la TUI y el Agente como procesos locales en la máquina del usuario.
2. THE Almacén SHALL residir únicamente en el sistema de archivos local del usuario.
3. WHERE el usuario configura un modelo de lenguaje local para el Agente, THE Agente SHALL usar ese modelo local sin enviar datos del Almacén a servicios remotos.
4. IF el Agente está configurado para usar un proveedor de modelo remoto, THEN THE Agente SHALL notificar al usuario esa configuración al iniciar la sesión de chat.

### Requisito 10: Comunicación entre TUI y Agente

**Historia de Usuario:** Como usuario, quiero que la TUI en Rust y el Agente en Python se coordinen de forma fiable, para que las acciones pedidas en el chat se reflejen en los datos y en los paneles.

#### Criterios de Aceptación

1. THE TUI y THE Agente SHALL comunicarse a través del Canal_IPC usando mensajes con un formato acordado de serialización JSON.
2. WHEN la TUI envía un mensaje al Agente a través del Canal_IPC, THE Canal_IPC SHALL entregar el mensaje al Agente preservando su contenido sin alteraciones.
3. FOR ALL mensajes válidos emitidos por la TUI, serializar el mensaje, enviarlo por el Canal_IPC y deserializarlo en el Agente SHALL producir un mensaje equivalente al original (propiedad round-trip).
4. IF el Agente no envía una respuesta dentro de 30 segundos tras recibir un mensaje, THEN THE TUI SHALL mostrar un mensaje de tiempo de espera y ofrecer al usuario reintentar o cancelar la petición.

### Requisito 11: Manejo de entradas inválidas y errores

**Historia de Usuario:** Como usuario, quiero recibir mensajes claros cuando mi entrada es inválida o una operación falla, para saber cómo corregir.

#### Criterios de Aceptación

1. IF el usuario envía un mensaje vacío desde el campo de entrada del Panel_Chat, THEN THE TUI SHALL descartar el envío y mostrar un aviso en la barra de estado.
2. IF el Agente no puede determinar la intención del mensaje, THEN THE Agente SHALL pedir al usuario una reformulación indicando qué información falta.
3. IF una operación del Agente o de la TUI sobre el Almacén falla, THEN THE Almacén SHALL devolver un error con código y descripción, y THE Agente o THE TUI SHALL comunicar al usuario la causa en lenguaje natural.

### Requisito 12: Restricciones tecnológicas

**Historia de Usuario:** Como usuario y desarrollador, quiero que el Organizador use el stack tecnológico acordado, para alinear la implementación con los objetivos del proyecto.

#### Criterios de Aceptación

1. THE TUI SHALL estar implementada en Rust usando la biblioteca Ratatui.
2. THE Agente SHALL estar implementado en Python usando el framework Strands Agents.
3. WHERE existan herramientas en `strands_tools` que cubran una capacidad requerida por el Agente (por ejemplo obtención de fecha y hora actuales o acceso a archivos locales), THE Agente SHALL usar esas herramientas prehechas en lugar de implementaciones propias equivalentes.
4. THE Almacén SHALL usar SQLite como motor de persistencia.

### Requisito 13: Edición manual desde la TUI

**Historia de Usuario:** Como usuario, quiero crear, editar, completar y eliminar Tareas y Eventos directamente desde la TUI sin pasar por el chat, para actuar de forma rápida sobre una entrada concreta usando el teclado.

#### Criterios de Aceptación

1. WHEN el usuario activa desde el Panel_Tareas la acción de crear Tarea, THE TUI SHALL abrir un formulario que permita capturar título, Prioridad, fecha límite y Grupo.
2. WHEN el usuario confirma el formulario de creación de Tarea en el Panel_Tareas, THE Almacén SHALL persistir la Tarea con Estado_Tarea igual a "pendiente" y confirmar la transacción antes de que la TUI muestre la Tarea en el Panel_Tareas.
3. WHEN el usuario activa desde el Panel_Tareas la acción de editar una Tarea seleccionada, THE TUI SHALL permitir modificar título, Prioridad, fecha límite, Grupo y Estado_Tarea, y THE Almacén SHALL persistir los cambios confirmando la transacción antes de que la TUI refleje la actualización.
4. WHEN el usuario activa desde el Panel_Tareas la acción de completar una Tarea seleccionada, THE Almacén SHALL actualizar el Estado_Tarea de esa Tarea a "completada" y confirmar la transacción antes de que la TUI refleje el cambio.
5. WHEN el usuario activa desde el Panel_Tareas la acción de eliminar una Tarea seleccionada, THE TUI SHALL pedir confirmación explícita y, tras recibirla, THE Almacén SHALL eliminar la Tarea y confirmar la transacción antes de que la TUI refleje el cambio.
6. WHEN el usuario activa desde el Panel_Calendario la acción de crear Evento, THE TUI SHALL abrir un formulario que permita capturar título, fecha, hora de inicio, duración y Grupo.
7. WHEN el usuario confirma el formulario de creación de Evento en el Panel_Calendario, THE Almacén SHALL persistir el Evento y confirmar la transacción antes de que la TUI muestre el Evento en el Panel_Calendario.
8. WHEN el usuario activa desde el Panel_Calendario la acción de editar un Evento seleccionado, THE TUI SHALL permitir modificar título, fecha, hora de inicio, duración y Grupo, y THE Almacén SHALL persistir los cambios confirmando la transacción antes de que la TUI refleje la actualización.
9. WHEN el usuario activa desde el Panel_Calendario la acción de eliminar un Evento seleccionado, THE TUI SHALL pedir confirmación explícita y, tras recibirla, THE Almacén SHALL eliminar el Evento y confirmar la transacción antes de que la TUI refleje el cambio.
10. WHEN una edición manual desde la TUI se confirma en el Almacén, THE TUI SHALL actualizar el Panel_Proximos dentro de 1 segundo siguiendo las reglas del Requisito 6.
11. IF una operación manual desde la TUI sobre el Almacén falla, THEN THE Almacén SHALL devolver un error con código y descripción y THE TUI SHALL mostrar la causa en la barra de estado y mantener la vista previa sin aplicar el cambio.

### Requisito 14: Interfaz dividida con paneles enfocables

**Historia de Usuario:** Como usuario, quiero una TUI dividida que me permita ver al mismo tiempo el chat, la lista de Tareas y el calendario, y cambiar el foco entre esos paneles con el teclado, para decidir dónde interactuar en cada momento.

#### Criterios de Aceptación

1. THE TUI SHALL mostrar simultáneamente el Panel_Chat, el Panel_Tareas, el Panel_Calendario y el Panel_Proximos en una disposición dividida.
2. WHILE el Organizador está en ejecución, THE TUI SHALL designar exactamente un panel como Panel_Enfocado.
3. WHEN el usuario pulsa la combinación de teclas configurada para cambiar de panel, THE TUI SHALL asignar el Panel_Enfocado al siguiente panel siguiendo un orden fijo y determinista entre el Panel_Chat, el Panel_Tareas, el Panel_Calendario y el Panel_Proximos.
4. WHILE un panel está designado como Panel_Enfocado, THE TUI SHALL mostrar en ese panel un indicador visual diferenciado (por ejemplo, borde resaltado o etiqueta activa) respecto a los demás paneles.
5. WHILE un panel está designado como Panel_Enfocado, THE TUI SHALL enrutar las pulsaciones de teclado únicamente a las acciones definidas para ese panel.
6. WHILE el Panel_Tareas está designado como Panel_Enfocado, THE TUI SHALL ofrecer atajos de teclado para crear, editar, completar y eliminar Tareas, y mostrar esos atajos en una leyenda visible dentro del Panel_Tareas.
7. WHILE el Panel_Calendario está designado como Panel_Enfocado, THE TUI SHALL ofrecer atajos de teclado para crear, editar y eliminar Eventos y para navegar entre fechas, y mostrar esos atajos en una leyenda visible dentro del Panel_Calendario.
8. WHILE el Panel_Chat está designado como Panel_Enfocado, THE TUI SHALL dirigir la entrada de texto al campo de mensaje del chat y enviar el mensaje al Agente según el Requisito 1.
9. IF el tamaño del terminal es menor que el tamaño mínimo necesario para renderizar todos los paneles, THEN THE TUI SHALL mostrar un mensaje indicando el tamaño mínimo requerido y no ejecutar operaciones sobre el Almacén.

### Requisito 15: Grupos de Tareas y Eventos

**Historia de Usuario:** Como usuario, quiero organizar mis Tareas y Eventos en Grupos (por ejemplo "trabajo" o "vida personal"), para separar contextos y que el Agente entienda esa organización.

#### Criterios de Aceptación

1. THE Almacén SHALL persistir cada Grupo con identificador, nombre único y Color_Grupo.
2. WHEN el usuario solicita crear un Grupo desde la TUI o desde el Panel_Chat indicando nombre y Color_Grupo, THE Almacén SHALL persistir el Grupo y confirmar la transacción antes de que la TUI o el Agente comunique la creación al usuario.
3. WHEN el usuario solicita renombrar un Grupo, THE Almacén SHALL actualizar el nombre de ese Grupo sin modificar las Tareas ni los Eventos asociados a ese Grupo.
4. WHEN el usuario solicita cambiar el Color_Grupo de un Grupo, THE Almacén SHALL actualizar el Color_Grupo de ese Grupo.
5. WHEN el usuario solicita eliminar un Grupo, THE Agente o THE TUI SHALL pedir confirmación explícita y, tras recibirla, THE Almacén SHALL eliminar el Grupo y dejar sin Grupo a todas las Tareas y Eventos previamente asociados a ese Grupo.
6. THE Almacén SHALL permitir que una Tarea o un Evento tenga a lo sumo un Grupo asignado o ningún Grupo.
7. WHEN el usuario crea una Tarea o un Evento desde el chat mencionando un Grupo existente por nombre, THE Agente SHALL asignar esa Tarea o ese Evento a ese Grupo.
8. WHEN el usuario crea una Tarea o un Evento desde el chat mencionando una categoría que no corresponde a ningún Grupo existente, THE Agente SHALL proponer al usuario crear un nuevo Grupo con ese nombre y, tras recibir confirmación, THE Almacén SHALL persistir el nuevo Grupo antes de asignarlo a la Tarea o al Evento.
9. WHEN el usuario crea una Tarea o un Evento desde el chat sin mencionar un Grupo y existen Grupos en el Almacén, THE Agente SHALL calcular para cada Grupo existente una puntuación de coincidencia entre el contenido del mensaje y el nombre del Grupo junto con los títulos de las Tareas y Eventos ya asociados a ese Grupo, y seleccionar el Grupo con la puntuación máxima como Grupo_Candidato.
10. IF la puntuación de coincidencia del Grupo_Candidato es mayor o igual a 0.75 en una escala de 0 a 1, THEN THE Agente SHALL asignar la Tarea o el Evento al Grupo_Candidato e informar al usuario del Grupo asignado en la respuesta.
11. IF la puntuación de coincidencia del Grupo_Candidato es menor que 0.75 y mayor o igual a 0.25 en una escala de 0 a 1, THEN THE Agente SHALL proponer al usuario el Grupo_Candidato y pedir confirmación antes de asignar la Tarea o el Evento a ese Grupo.
12. IF ninguna puntuación de coincidencia de un Grupo existente alcanza 0.25 en una escala de 0 a 1, THEN THE Agente SHALL proponer al usuario crear un nuevo Grupo con un nombre sugerido a partir del contenido del mensaje y no asignará la Tarea o el Evento a ningún Grupo hasta recibir confirmación.
13. FOR ALL estados fijos del Almacén y mensajes del usuario, repetir el cálculo de coincidencia del Agente SHALL producir el mismo Grupo_Candidato y la misma puntuación de coincidencia (propiedad determinista de la inferencia de Grupo).

### Requisito 16: Colores por Grupo

**Historia de Usuario:** Como usuario, quiero que cada Grupo tenga un color configurable que se refleje en los paneles de la TUI, para identificar de un vistazo a qué contexto pertenece cada Tarea o Evento.

#### Criterios de Aceptación

1. WHERE una Tarea o un Evento tiene un Grupo asignado, THE TUI SHALL renderizar esa entrada usando el Color_Grupo del Grupo asignado en el Panel_Tareas, el Panel_Calendario y el Panel_Proximos.
2. WHERE una Tarea o un Evento no tiene un Grupo asignado, THE TUI SHALL renderizar esa entrada con un color neutro predeterminado distinto de cualquier Color_Grupo definido por el usuario.
3. WHEN el usuario cambia el Color_Grupo de un Grupo, THE TUI SHALL actualizar dentro de 1 segundo el color de todas las Tareas y Eventos asociados a ese Grupo en el Panel_Tareas, el Panel_Calendario y el Panel_Proximos.
4. IF el terminal del usuario no puede renderizar con precisión el Color_Grupo configurado, THEN THE TUI SHALL seleccionar el color más cercano admitido por las capacidades reportadas del terminal y mostrar una advertencia descriptiva en la barra de estado.
5. IF el terminal del usuario no admite color, THEN THE TUI SHALL distinguir los Grupos mediante un marcador textual (por ejemplo, una etiqueta entre corchetes con el nombre del Grupo) en lugar del Color_Grupo.

### Requisito 17: Visualización de fechas límite de Tareas en el calendario

**Historia de Usuario:** Como usuario, quiero ver en el calendario también las fechas límite de mis Tareas, para planificar el día mirando un único panel.

#### Criterios de Aceptación

1. THE Panel_Calendario SHALL mostrar cada Evento en la fecha correspondiente a la fecha de inicio de ese Evento.
2. WHERE una Tarea tiene fecha límite, THE Panel_Calendario SHALL mostrar esa Tarea en la fecha correspondiente a su fecha límite junto a los Eventos de la misma fecha.
3. WHERE una Tarea no tiene fecha límite, THE Panel_Calendario SHALL omitir esa Tarea.
4. THE Panel_Calendario SHALL diferenciar visualmente las Tareas de los Eventos usando un marcador o estilo distinto para Tareas (por ejemplo, un prefijo específico o un glifo reservado a Tareas).
5. WHERE una Tarea mostrada en el Panel_Calendario tiene un Grupo asignado, THE Panel_Calendario SHALL renderizar esa Tarea usando el Color_Grupo del Grupo asignado, manteniendo el marcador de Tarea definido en el criterio anterior.
6. WHEN una Tarea con fecha límite es creada, modificada o eliminada en el Almacén, THE Panel_Calendario SHALL actualizar la vista dentro de 1 segundo.
