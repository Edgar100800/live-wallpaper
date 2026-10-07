---
target: UI de ParticleWall para macOS
total_score: 21
max_score: 40
na_heuristics:
p0_count: 0
p1_count: 4
timestamp: 2026-10-07T14-25-53Z
slug: sources-particlewall-galleryview-swift
---
Method: dual-agent (A: /root/ui_design_review · B: /root/ui_detector_review)

# Revisión Impeccable de UI macOS

Fecha: 7 de octubre de 2026. Objetivo principal: `Sources/ParticleWall/GalleryView.swift`. También se revisaron personalización, Ajustes y acciones de barra superior. Dos evaluaciones independientes, con síntesis posterior. Revisión de código; no inspección visual en macOS.

## Dirección recomendada

Mantener una utilidad nativa, discreta y centrada en las imágenes. Los fondos aportan identidad; la interfaz debe explicar qué está activo, en qué pantalla y qué cambios se guardan. No se recomienda convertirla en un dashboard web ni añadir decoración antes de resolver interacción y confianza.

La estructura visual descrita en código es estándar para una biblioteca de medios, pero el selector por pantalla, las previews y los controles de color están ligados al producto. Esa familiaridad es adecuada en macOS. La oportunidad principal consiste en comunicar mejor el estado del escritorio.

## Evaluación provisional

Escala: 0–4 por heurística. Las diez aplican a esta interfaz de operación. Las puntuaciones describen una evaluación de código, no una certificación visual ni de accesibilidad.

| Heurística | Puntuación | Principal observación |
| --- | --- | --- |
| Visibilidad del estado | 2/4 | Galería no muestra pausa ni motivo de suspensión |
| Lenguaje del usuario | 2/4 | Aparecen renderer, thumbnail y vocabulario técnico |
| Control y libertad | 2/4 | Cerrar guarda; no existe descarte coherente |
| Consistencia nativa | 3/4 | Controles estándar, pero acciones principales por gestos |
| Prevención de errores | 2/4 | Eliminación permanente sin recuperación |
| Reconocimiento | 2/4 | Opciones dependen de hover y clic derecho |
| Eficiencia | 2/4 | Atajos parciales; selección por teclado no explícita |
| Minimalismo | 3/4 | Galería enfocada y formularios agrupados |
| Recuperación de errores | 2/4 | Errores visibles en algunos flujos, otros se silencian |
| Ayuda | 1/4 | Guía contextual insuficiente para reproducción y edición |
| **Total** | **21/40** | **Mejoras importantes de interacción** |

## Fortalezas

- Galería adaptable, imágenes protagonistas y barra compacta con búsqueda e importación.
- NSOpenPanel, NSMenu, SF Symbols, formularios y ColorPicker preservan convenciones macOS.
- Editor identifica fondo y pantalla, muestra carga, valores numéricos y perfiles de color reutilizables.

## Prioridades

### P1: acceso por teclado y VoiceOver

`GalleryView.swift:251` aplica fondo mediante `.onTapGesture`; preview y opciones se crean solo al pasar el puntero (`:225`, `:236`). Estado activo se representa con borde de color (`:233`). Sliders numéricos tienen Text adyacente, sin nombre asociado explícitamente al Slider (`WallpaperCustomizationView.swift:272`).

Usar Button o selección nativa para aplicar; mostrar opciones también al recibir foco; nombrar controles; añadir indicador Activo con texto/checkmark y valor accesible. Verificar teclado y VoiceOver en Mac. Comandos adecuados: `impeccable harden` y `impeccable audit`.

### P1: contrato de guardado coherente

Editor anuncia «Cambios sin guardar» y ofrece Guardar, pero cerrar/desaparecer persiste valores; colores y restauración también guardan inmediatamente (`WallpaperCustomizationView.swift:31`, `:44`, `:97`, `:107`, `:339`, `:391`). Abrir Personalizar aplica primero el fondo (`GalleryView.swift:118`).

Elegir autoguardado explícito con Deshacer/Revertir, o edición temporal con Cancelar/Guardar. No mezclar ambos. Aclarar si Personalizar debe aplicar el fondo o conservar asignación hasta Aplicar. Comandos: `impeccable clarify` y `impeccable harden`.

### P1: eliminación recuperable

Eliminar llama directamente al borrado de carpeta (`GalleryView.swift:284`, `LibraryManager.swift:239`). Un rol destructivo no aporta recuperación. Los fondos incluidos sí están protegidos.

Preferir Mover a la papelera. Si debe ser permanente, confirmar con nombre y pantallas afectadas. Perfiles de color pueden ofrecer Deshacer. Comando: `impeccable harden`.

### P1: reproducción y acceso a Ajustes visibles

Pausa y Power Save viven en menú de clic derecho del icono; galería no muestra reproducción (`AppDelegate.swift:185`, `:197`, `GalleryView.swift:67`). Con distintos fondos en varias pantallas, «Todas» no identifica una asignación única.

Añadir control discreto de reproducción, acceso a Ajustes y línea de estado: Reproduciendo, Pausado por usuario, Pausado en batería o Ahorro de energía. Explicar asignaciones mixtas por pantalla. Mantener menú contextual como acceso rápido. Comandos: `impeccable clarify` y `impeccable onboard`.

### P2: búsqueda sin resultados y mensajes accionables

Una búsqueda sin coincidencias reutiliza «Sin wallpapers» y pide importar (`GalleryView.swift:27`, `:139`). Parece que la biblioteca está vacía.

Distinguir biblioteca vacía de «Sin resultados para …», ofrecer Limpiar búsqueda y mencionar `.asciivideo` entre formatos admitidos. Revisar términos: Fondo, Miniatura y Ahorro de energía. Comando: `impeccable clarify`.

## Carga cognitiva y recorrido

Carga moderada: opciones secundarias bien agrupadas, pero reglas de guardado y acceso oculto requieren recordar comportamiento. Menú de fondo tiene siete ramas y FPS cinco opciones; esto no implica eliminar opciones útiles. Separar reproducción y calidad en Ajustes, mostrar FPS heredado y mantener mantenimiento como secundario.

Momento positivo: ver fondo animado y ajustar colores. Fricciones: localizar opciones ocultas, interpretar pausa sin explicación y cerrar editor sin saber qué se guardó. Terminar con confirmación discreta de fondo, pantalla y reproducción.

## Riesgos por persona

- Usuario de teclado/VoiceOver: acciones por hover, sliders sin nombre explícito y activo comunicado solo con color. Requiere validación nativa.
- Usuario nuevo: no sabe que clic aplica, Personalizar cambia escritorio y Ajustes requiere clic derecho.
- Usuario avanzado: Command-I/Command-S ayudan, pero falta flujo claro de selección por teclado y foco de búsqueda; guardado cambia según control.

## Observaciones menores

Validar nombres compuestos solo por espacios; comprobar selector invisible con una pantalla; probar nombres largos en editor de tamaño fijo; mostrar nombre completo cuando se trunca; informar fallos de rename/FPS/delete que actualmente usan `try?`. Contraste, truncamiento y navegación real siguen sin medir.

## Evidencia del detector

Se ejecutó detector una vez sobre `Sources/ParticleWall`: exit 0, cero hallazgos. Usó regex porque faltan módulos HTML; no soporta Swift. Excluyó los cuatro archivos de UI nativa y solo encontró 25 recursos web elegibles. Por tanto, cero hallazgos no significa UI validada. Ambas evaluaciones independientes coinciden en acciones por hover, sliders, guardado y eliminación; detector no aporta confirmación mecánica de estos defectos nativos.

No se generó reconstrucción web, overlay ni capturas ficticias. Esta revisión no cambió UI de producto.

## Decisiones para una siguiente iteración

1. Prioridad: accesibilidad/acciones, guardado/reversión o estado de reproducción.
2. Guardado: automático con Deshacer o explícito con Cancelar/Guardar.
