# Análisis de memoria de ParticleWall en Linux / Omarchy

Fecha: 2026-10-07. Análisis del árbol de trabajo actual, incluidos los cambios locales existentes. No se modificó código de producto ni configuración del escritorio.

## Correcciones implementadas después del análisis

Los hallazgos siguientes documentan el estado previo al parche. Se corrigieron los cuatro defectos de memoria:

- `web.rs`: se reutiliza el WebView de cada monitor. Al recargar se renuevan los scripts de pausa/FPS sin acumularlos. Al abandonar web se detiene la carga, se retira el hijo GTK y se vacía el vector.
- Los callbacks de carga y el hook de cambio usan `Weak` para evitar ciclos con `DaemonState`.
- `gpu.rs`: los buffers compartidos reservan capacidad para el mayor modelo desde el inicio. La misma función de inicialización se usa al crear y cambiar modelos. El coste adicional frente al buffer pequeño anterior es aproximadamente 1,10 MiB por presenter.
- `wayland.rs`: `GpuLayerSurface` toma propiedad de los objetos de protocolo antes de comprobar handles o esperar configure. Todas esas salidas de error ejecutan su destructor.

Validación completada:

```sh
cd linux
cargo test --workspace --no-fail-fast
PARTICLEWALL_GUI_TEST=1 cargo test --test web-lifecycle
cargo build --bin particlewall
```

La prueba GPU sube los datos para 9 → 5 → 4 → 5 sobre las mismas asignaciones y comprueba que no hay errores de validación. La prueba GTK/WebKit reaplica fondos 30 veces, comprueba que hay un solo WebView por salida y verifica mediante una referencia débil que el widget anterior se destruye al retirarlo. Después crea un host nuevo. Su ventana tiene una superficie realizada, pero no se mapea sobre el escritorio.

La prueba GUI usa un ejecutable sin el harness estándar para inicializar y cerrar GTK/WebKit en el hilo principal. El primer intento con un test convencional pasó sus comprobaciones pero abortó durante la destrucción global de WebKit; el ejecutable independiente también termina correctamente.

El parche no reinicia el daemon activo ni sustituye su binario release. No se realizó todavía una prueba de errores de configure en un compositor aislado ni una medición prolongada del daemon corregido. La observación sobre el límite FPS del renderer GPU queda pendiente: es un defecto de consumo energético fuera de estas correcciones de memoria.

## Resultado

Hay defectos de gestión de memoria en las transiciones de renderers y un error de capacidad de buffer GPU. La observación breve del fondo GPU activo no muestra crecimiento de memoria. No hay evidencia para atribuir una fuga del sistema a Omarchy.

Se usó CodeGraph CLI 1.5.0: sincronización del índice, exploración de `switch_renderer` / `spawn_web_child`, callers y lectura de `set_model` / `create_surface`. Las relaciones del grafo se contrastaron con el código: algunos nombres genéricos se resuelven a símbolos de otras plataformas. Se aplicaron las instrucciones de Caveman e investigación antes de editar.

## Hallazgos por prioridad

### 1. Alta: acumulación de WebViews al reaplicar fondos web

Ubicación: `linux/crates/particlewall-linux/src/web.rs:391`, `:411`, `:454`.

La rama web llama a `spawn_web_child` por monitor en cada aplicación. Esta función crea otro WebView, reemplaza el hijo visible de la ventana y añade una referencia fuerte a `state.webviews`. La rama no elimina las referencias anteriores. Después de N aplicaciones web y M monitores, puede haber N × M WebViews retenidos aunque solo M sean visibles. Cada documento conserva sus recursos mientras siga referenciado; el coste exacto de helpers WebKit requiere medición.

La ruta es alcanzable: la biblioteca incluye `SpiderManASCIIWallpaper`, cuyo ID no está en `gpu_model_for`. Reaplicar ese fondo también recorre esta rama.

Corrección propuesta: mantener un único WebView por monitor y reutilizarlo con `load_uri`, o retirar explícitamente el anterior antes de añadir su reemplazo. Verificar cambios web repetidos y contar WebViews vivos y memoria de todos los helpers WebKit.

Evidencia: defecto demostrado por propiedad y crecimiento del vector; no se ejecutó una reproducción cambiando el fondo del usuario.

### 2. Media: el último WebView sigue retenido al volver a GPU

Ubicación: `linux/crates/particlewall-linux/src/web.rs:265`, `:366`, `:453`.

La transición vacía `state.webviews`, pero no elimina el hijo de cada `ApplicationWindow`. La ventana sigue conteniendo el último WebView y solo se oculta con `set_visible(false)`. Por tanto, soltar el vector no completa la destrucción anunciada en el comentario. Persisten documentos web junto al renderer GPU.

Corrección propuesta: retirar el hijo con `set_child(None::<&gtk4::Widget>)` al abandonar web y liberar las referencias del estado; usar referencias débiles en callbacks que capturan ese estado. Verificar destrucción del widget con una referencia débil. No exigir que desaparezca todo helper WebKit, ya que puede existir reutilización de procesos.

Evidencia: retención por el árbol de widgets; consumo adicional no cuantificado.

### 3. Alta: escritura fuera de capacidad al cambiar a Espiral Prima

Ubicación: `linux/crates/particlewall-render/src/gpu.rs:363`, `:480`, `:495`.

`GpuPresenter::new` reserva 6.480 × 16 = 103.680 bytes en `flow_particles` cuando el modelo inicial no es 5. `set_model(5)` reutiliza ese buffer e intenta escribir 78.498 × 16 = 1.255.968 bytes. No redimensiona el buffer ni reconstruye sus bind groups.

El arranque actual usa `JellyfishPointsWallpaper` (modelo 9), por lo que cambiar después a `PrimeSpiralWallpaper` recorre esta situación. El manejador de errores registra los fallos de wgpu; el código actualiza el modelo aunque la escritura no sea válida.

Esto es un error de capacidad de memoria GPU y validación, no evidencia de corrupción de memoria del proceso: la dependencia local `wgpu-core 25.0.2`, `src/device/queue.rs:577`, comprueba el límite y devuelve `BufferOverrun`. La [documentación de Queue](https://docs.rs/wgpu/latest/wgpu/struct.Queue.html) también exige escrituras dentro de los límites.

Corrección propuesta: reservar capacidad máxima al construir el presenter, o redimensionar el buffer y reconstruir todos los bind groups asociados. Comprobar modelo 9 → 5 → 4 → 5 y un arranque directo en modelo 5.

Evidencia: tamaños y ruta demostrados estáticamente; no reproducido sobre el daemon activo.

### 4. Media: superficies Wayland sin limpieza en errores de creación

Ubicación: `linux/crates/particlewall-linux/src/wayland.rs:236`, `:241`, `:245`, `:323`.

Se crean `WlSurface` y `ZwlrLayerSurfaceV1` antes de construir `GpuLayerSurface`. Si los handles son nulos o `wait_configure` falla, se retorna sin ejecutar las llamadas `destroy` que existen únicamente en `Drop for GpuLayerSurface`. Ese propietario todavía no existe. Los proxies generados por `wayland-client` no envían automáticamente estas peticiones de destrucción al soltar una variable.

Los objetos de protocolo pueden permanecer en la conexión persistente y en el compositor. Reintentar aplicaciones GPU durante fallos puede acumularlos.

Corrección propuesta: introducir un propietario temporal con limpieza desde la primera creación, o destruir explícitamente ambos objetos en cada salida de error. Probar error de configure y reintentos en una sesión aislada.

Evidencia: rutas de error y falta de limpieza demostradas; acumulación durante fallos no medida.

## Observaciones adicionales

- `web.rs:538`: `DaemonState.switch` guarda una closure que captura un `Rc` del propio estado. Existe un ciclo de referencias. Tiene tamaño fijo en la sesión normal; no explica por sí solo crecimiento por frame. Usar `Weak` permitiría liberar el estado al desmontar la sesión.
- `web.rs:462–468`: el bucle GPU lee `fps_cap` y lo descarta. El valor 30 reportado por `--status` no limita ese bucle; se solicita trabajo aproximadamente cada 16 ms, sujeto al compositor. Es un defecto de consumo energético, no prueba de fuga.
- Los presenters GPU aparcados se reutilizan por ID de salida. Su retención está deliberadamente documentada para evitar problemas de NVIDIA/Wayland; no se identificó crecimiento por cada cambio normal GPU → GPU.
- La pausa Linux conserva los recursos del renderer. No equivale al reposo de macOS que destruye el WebView.

## Observación del sistema

Daemon observado: PID 2152, ejecutable `linux/target/release/particlewall`, compilado el 2026-09-06. La equivalencia exacta entre ese binario y el árbol de trabajo actual no está demostrada.

Fondo: `JellyfishPointsWallpaper`; una salida HDMI-A-1, 1920 × 1080; renderer GPU según journal. Memoria GPU atribuida por `nvidia-smi`: 88 MiB. GPU completa: 857 / 6.144 MiB.

En la lectura inicial: 48.031 MiB de RAM total, 36.955 MiB disponibles y cero swap usado. PSI de memoria (`avg10`, `avg60`, `avg300`) en cero; sin eventos OOM encontrados en el journal del kernel del arranque actual.

La diferencia entre RSS (~313 MiB), PSS (~228 MiB) y memoria de cgroup (~102 MiB) refleja métricas distintas. No deben sumarse ni interpretarse como pruebas de fuga. Parte considerable del mapeo es memoria de archivos/bibliotecas; memoria anónima observada ~50 MiB.

Medición temporal: 13 muestras cada 10 segundos, de 08:52:15 a 08:54:15 (America/Lima), sin cambiar el fondo ni la reproducción. Resultado en esos 120 segundos:

| Métrica | Resultado |
| --- | --- |
| RSS | 320.484 KiB, constante (~313 MiB) |
| PSS | ~233.594 KiB (~228 MiB), variación mínima |
| Memoria anónima | 50.860 KiB, constante |
| Private dirty | 53.284 KiB, constante |
| Swap del proceso | 0 KiB |
| Descriptores abiertos | 110–112; inicio y final 110 |
| Hilos | 36, constante |

Muestras originales guardadas en `/tmp/particlewall-memory-analysis.json` (archivo temporal).

## Límites y validación siguiente

El análisis identifica mecanismos concretos, pero no demuestra una fuga sostenida en el renderer GPU activo ni descarta crecimiento a largo plazo. No se cambió el wallpaper, se reinició el servicio ni se sometió la sesión a fallos de Wayland.

La validación de las correcciones debe realizarse en una sesión aislada: reaplicar web 20–50 veces, alternar web/GPU, cambiar a Espiral Prima y provocar fallos de configure. Medir PSS/RSS, memoria anónima, VRAM, descriptores, widgets vivos y todos los helpers WebKit, después de permitir que las tareas pendientes terminen. Comparar mesetas y recursos vivos, no exigir que el allocator devuelva inmediatamente toda la RAM.


## Validación en esta PC: versión release reabierta

Fecha: 2026-10-07, 09:10–09:18 America/Lima. Se compiló release, se inició el servicio que estaba detenido y se abrió Ajustes. El ejecutable corregido quedó activo con PID 59091, sin reinicios automáticos. No se cambió la configuración de Hyprland ni se habilitó el servicio para el arranque.

### Comprobaciones que pasan

- Medusa de Puntos se ve y anima en HDMI-A-1 a 1920 × 1080. Su superficie aparece en la capa inferior correcta, por encima del fondo estático de Omarchy.
- Pausa visual: dos capturas separadas por un segundo son idénticas con tolerancia de 2 %. Antes y después de la pausa muestran cambios. La reanudación funciona.
- Medusa → Espiral Prima → Lluvia de Ruido → Espiral Prima mantiene el proceso vivo. Espiral Prima ya no produce el error de escritura fuera de capacidad. Lluvia presenta el defecto separado indicado abajo.
- SpiderMan ASCII carga y anima mediante WebKit; se inspeccionó una captura real del escritorio.
- Se completaron 30 reaplicaciones web y cinco ciclos web/GPU. Durante las reaplicaciones, el mismo proceso WebKit sigue vivo y no se acumulan procesos de renderizado adicionales. PSS total de la unidad: aproximadamente 539–549 MiB en web; últimos ciclos web: 547–549 MiB.
- Al volver a GPU desaparece el proceso `WebKitWebProcess`; permanece `WebKitNetworkProcess`. El número de procesos de la unidad pasa de ocho a dos.
- Se restauraron `JellyfishPointsWallpaper`, colores y perfiles originales; reproducción activa y Ajustes abierto en el workspace original (3).

El primer tramo de reposo tuvo cambios de fondo externos a la secuencia del script; no se interpreta como un minuto continuo de GPU. Se repitió con siete muestras verificando `--status` en cada una: de 09:16:47 a 09:17:48, siempre Medusa y PID 59091. RSS del daemon: 401,50 MiB constante; PSS: 302,74 MiB; memoria anónima: 86,75 MiB constante. Esa medición se hizo antes de volver a abrir Ajustes.

El consumo posterior a WebKit no vuelve al arranque frío: PSS total inicial GPU ~233 MiB frente a ~352 MiB después del uso web. Memoria GPU del daemon observada al final: 228 MiB; inicialmente 88 MiB. Hay recursos y procesos compartidos retenidos tras usar web; esta prueba corta demuestra una meseta observada, no que toda memoria o VRAM se devuelva inmediatamente ni ausencia de fugas a largo plazo.

### Defecto encontrado durante la validación

Lluvia de Ruido no supera la validación de renderizado. El journal muestra que `flow particles / primes` está enlazado simultáneamente como `STORAGE_READ_ONLY` (group 0) y `STORAGE_READ_WRITE` (group 1) durante `flowUpdate`. wgpu invalida el compute pass y el command encoder. Esto es independiente del tamaño de buffer corregido. La prueba de regresión de capacidad verifica las subidas de datos, pero no cubre ese pase de renderizado del presenter.

Ubicación: `linux/crates/particlewall-render/src/gpu.rs`, pase de flujo en `render`, bindings 0 y 1. Debe evitarse enlazar el mismo almacenamiento con accesos incompatibles en un mismo dispatch, y añadirse una prueba del pase real. No se modificó código de producto durante esta validación.

La validación funcional queda parcial: Medusa, ASCII, pausa y transición a Espiral pasan; Lluvia requiere otro parche. También siguen pendientes el límite FPS GPU y la prueba de fallos de configure en compositor aislado. `--status` devuelve `outputs: 0` al activar web aunque la superficie esté visible; su hook actual solo cuenta salidas GPU, por lo que ese dato no valida el número de monitores web.

Evidencias temporales:

- `/tmp/particlewall-live-memory-validation.json`: muestras de toda la unidad, incluido WebKit.
- `/tmp/particlewall-live-final-samples.json`: siete muestras finales con fondo verificado.
- `/tmp/particlewall-live-visual-results.json`: comparación de capturas de animación/pausa.
- `/tmp/particlewall-open-validation.png`, `/tmp/particlewall-gpu-active-a.png`, `/tmp/particlewall-web-active-a.png`: capturas inspeccionadas.
- Journal del servicio desde las 09:10:38; error de Lluvia a las 09:13:08. Después de volver a otros fondos no aparecieron nuevos errores de validación durante los ciclos probados.


### Ajustes unificados (2026-10-07)

El bucle GPU ahora programa cada tick usando el límite FPS configurado; dejó de
leer y descartar ese valor. El modo automático conserva la cadencia anterior de
aproximadamente 60 Hz. El límite y las preferencias ASCII se guardan en la
configuración. El contador de salidas web también usa los monitores disponibles.
Estas correcciones sustituyen los pendientes de FPS y `outputs: 0` descritos en
la validación anterior. Lluvia de Ruido y la inyección de fallos Wayland siguen
pendientes.
