# Toast overlay nativo (Direct2D/DirectWrite)

Fecha: 2026-07-07

## Contexto y problema

El toast in-game actual ("Ready to clip", "Clip guardado", errores…) es una **ventana WebView2 aparte** (`overlay`) que carga [`static/overlay.html`](../../../static/overlay.html): transparente, always-on-top, click-through, no-activable. Se mantiene oculta entre avisos y se muestra ~2.8 s.

El problema no es funcional sino de coste: para un cartel de una línea se mantiene residente una instancia entera de WebView2 (decenas de MB de RAM + su proceso) en una app cuyo principio nº1 es **rendimiento y bajo consumo**. Es la parte menos coherente del stack.

Objetivo: sustituir el toast por una implementación **nativa de Windows** (sin WebView2), más ligera y coherente con el pipeline D3D11/D2D que ya existe, y de paso rediseñarlo **más alto y a dos líneas**, inspirado en el toast de SteelSeries Moments.

## Alcance

- Reemplazar el toast WebView2 por una ventana Win32 en capas dibujada con Direct2D/DirectWrite.
- Rediseño visual: lengüeta anclada al borde derecho, sin borde, redondeo sutil, dos líneas (marca + contenido), icono a la izquierda, teclas del atajo como *keycaps*.
- Mantener el mismo contrato de invocación desde el frontend (`invoke('toast', …)`), ampliando el payload.

Fuera de alcance:
- Cola de toasts (se mantiene el comportamiento de **reemplazo**).
- Dibujar sobre juegos en **fullscreen exclusivo real** (limitación preexistente; sin regresión: funciona en ventana/borderless como hasta ahora).
- Hooking del swapchain del juego (contrario a la filosofía del proyecto).

## Aspecto visual

- **Forma**: lengüeta pegada al borde derecho de la pantalla, con **solo las esquinas izquierdas redondeadas** (radio sutil, ~8 px, menor que los 12 px actuales). **Sin borde.** Fondo oscuro sólido (~`#141416`, como el actual `rgba(20,20,22,0.96)`).
- **Posición**: arriba-derecha del monitor primario, con un pequeño margen superior. El lado derecho queda a ras del borde del monitor.
- **Layout**: icono mono a la izquierda **centrado verticalmente**, y a su derecha dos líneas:
  - Línea 1: **Flashback** (Segoe UI Semibold, color claro `~#f0f2f7`).
  - Línea 2: contenido variable (ver "Modelo de datos").
- **Keycaps**: cuando la línea inferior lleva teclas, cada tecla se dibuja en una **cajita redondeada** en D2D (`[ Alt ] + [ F8 ]`), con fondo un pelo más claro que la tarjeta y texto tenue. El `+` entre cajitas es texto normal. Tras las cajitas va el texto del `body`.
- **Animación**: entra deslizando desde la derecha + fundido; sale igual. ~2.8 s visible + ~0.3 s de salida (equivalente a la transición CSS actual).
- **Errores**: como se elimina el borde, el tipo `error` **no** usa recuadro. Se distingue con un **acento rojo sutil** (tinte rojo en el icono y/o la línea de marca). El resto de tipos (`info`, `ready`, `saved`) comparten el mismo estilo limpio, sin diferencias de color.

## Arquitectura

Ventana Win32 en capas propia, sin WebView2, dibujada con Direct2D/DirectWrite (misma tecnología que [`src-tauri/src/overlay.rs`](../../../src-tauri/src/overlay.rs)).

- **Ventana**: `WS_POPUP` con estilos extendidos `WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP`. Click-through real, nunca roba foco al juego, oculta de Alt-Tab.
- **Composición**: **DirectComposition** (device + target + visual) para el alfa por píxel y la animación (offset + opacidad) compuesta en GPU, sin readback CPU por frame.
- **Dispositivo D3D11 propio y minúsculo**, **separado** del device de captura/encoder. El camino de captura es sagrado: el toast no comparte device con él para evitar cualquier contención (coherente con la regla "replay single device"). No toca texturas de captura.
- **Hilo dedicado** con su propio message pump que posee el HWND y los recursos D2D/DComp. Recibe comandos por canal (`Show { title, body, keys, kind }`, `Hide`) y gestiona el temporizador de auto-ocultado. Nada de esto toca el hilo de captura/codificación.
- **DPI-aware**: el tamaño de la ventana se calcula midiendo el texto con DWrite y se reescala según el monitor primario (`GetDpiForMonitor`). Reposicionado en cada `Show` por si cambió resolución/escala.

### Ciclo de vida

- Se crea una sola vez al arrancar la app (en el `setup` de Tauri, o perezosamente en el primer `toast`). El hilo del overlay queda vivo, con la ventana oculta.
- `Show`: recalcula tamaño/posición según contenido y monitor, pinta, muestra la ventana (sin activarla) y arranca la animación de entrada; programa el auto-ocultado.
- `Hide`: animación de salida y oculta la ventana. Un `Show` nuevo mientras hay uno visible **reemplaza** el contenido y reinicia la animación de entrada.

## Modelo de datos / IPC

El frontend ya conoce el atajo (`hotkeys.saveReplay`, con `labelFor` en [`src/lib/hotkeys.svelte.ts`](../../../src/lib/hotkeys.svelte.ts)), así que **calcula las teclas y las pasa** en el payload.

El comando `toast` pasa a aceptar:

```rust
struct ToastPayload {
    title: String,        // "Flashback"
    body: String,         // texto de la línea inferior (tras los keycaps si los hay)
    keys: Vec<String>,    // p.ej. ["Alt","F8"]; vacío = sin keycaps
    kind: String,         // "info" | "ready" | "saved" | "error"
}
```

Mapeo de los toasts existentes:

- **Replay listo** (`toast.replayReady`): `title: "Flashback"`, `keys: ["Alt","F8"]` (las teclas reales configuradas), `body:` nueva clave i18n tipo `"para hacer un clip"`. Pierde el texto "Listo para clipear".
- **Resto** (Clip guardado, Grabando, Grabación detenida, avisos, errores): `title: "Flashback"`, `keys: []`, `body:` su mensaje actual, `kind` correspondiente.

### Cambios en frontend

- El helper `toast()` de [`src/routes/+layout.svelte`](../../../src/routes/+layout.svelte) añade `title` fijo `"Flashback"` y un parámetro opcional `keys`.
- Solo la llamada del toast de "listo" (`if (wasOff) toast(...)`) pasa `keys` derivadas de `hotkeys.saveReplay` (troceando el acelerador en tokens; reutilizar la lógica de `labelFor`).
- i18n: añadir la clave del hint (`toast.replayReadyHint`: ES "para hacer un clip" / EN "to clip") y ajustar el uso de `toast.replayReady`.

## Comportamiento

- **Reemplazo** (no cola): un toast nuevo sustituye al visible.
- **Duración**: ~2.8 s visible, ~0.3 s de salida.
- **Monitor**: primario (paridad con el comportamiento actual).
- **Sin foco**: `Show` nunca activa la ventana ni roba foco al juego.

## Qué se elimina

- [`static/overlay.html`](../../../static/overlay.html).
- La ventana `overlay` en [`src-tauri/tauri.conf.json`](../../../src-tauri/tauri.conf.json).
- [`src-tauri/capabilities/overlay.json`](../../../src-tauri/capabilities/overlay.json) (si no aporta otros permisos).
- La lógica de `setup` que configuraba la webview `overlay` (`set_ignore_cursor_events`, `set_focusable`) en [`src-tauri/src/lib.rs`](../../../src-tauri/src/lib.rs).
- Los comandos `toast` / `dismiss_toast` se reescriben para hablar con el overlay nativo (el `dismiss_toast` invocado desde JS deja de existir; el auto-ocultado lo gestiona el hilo nativo).

## Riesgos y notas

- **Fullscreen exclusivo**: ni el WebView2 actual ni la ventana en capas se pintan sobre juegos en fullscreen exclusivo real. No es una regresión; en borderless/ventana funciona igual.
- **DirectComposition**: requiere cuidado con el orden de creación (device D3D11 → DComp device → target del HWND → visual → surface D2D). Aislado en su módulo.
- **Medición de texto multilínea + keycaps**: el ancho de la lengüeta depende del `body` y del número de teclas; se mide con DWrite antes de dimensionar la ventana.
- **Recuperación**: pérdida del device (p. ej. cambio de GPU/driver) debe recrear recursos del overlay sin tumbar la app; al ser un device propio y desechable, basta recrear en el siguiente `Show`.
