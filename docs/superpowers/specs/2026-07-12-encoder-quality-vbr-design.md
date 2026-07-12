# Diseño: calidad de imagen del encoder (VBR + High/CABAC + GOP)

Fecha: 2026-07-12
Estado: aprobado el enfoque; pendiente de plan de implementación.

## Objetivo

Eliminar la inconsistencia de calidad percibida en los clips ("a veces se ve peor,
sobre todo en escenas con movimiento") sin degradar el rendimiento del camino de
captura. El diagnóstico NO es falta de bitrate (los defaults ya son generosos: ~50
Mbps a 1080p60 en "Alto", ~130 Mbps en "Ultra"), sino tres decisiones de
configuración del encoder H.264:

1. **CBR** (bitrate plano): la calidad "respira": baja en escenas complejas y
   desperdicia bits en las estáticas.
2. **Perfil Baseline** (sin CABAC): menos eficiente, peor en movimiento.
3. **GOP de 0.5 s** en el replay: keyframes IDR muy frecuentes y caros que, en CBR,
   provocan bajones de calidad justo tras cada uno.

Fuera de alcance (spec aparte): la estabilidad de FPS bajo carga de GPU (el resync de
emergencia que descarta ~1 s de slots). Aquí no se toca el pacing.

## Decisiones

### Control de tasa: VBR con techo (Peak-Constrained VBR)

- Modo `eAVEncCommonRateControlMode_PeakConstrainedVBR` vía `ICodecAPI`
  (`CODECAPI_AVEncCommonRateControlMode`).
- `CODECAPI_AVEncCommonMeanBitRate` = el bitrate objetivo actual (`target_bitrate` /
  override del usuario). Es decir, la media apunta a lo mismo que hoy.
- `CODECAPI_AVEncCommonMaxBitRate` = pico = `mean * 7/4` (1.75×). Da margen para
  ráfagas de bits en escenas complejas y ahorra en estáticas, con tamaño acotado y
  predecible.
- Se elige VBR (no calidad constante/CQP) por su soporte universal en NVENC/QSV/AMF,
  evitando lógica de fallback por hardware.

### Perfil y entropía: High + CABAC, sin B-frames

- `MF_MT_MPEG2_PROFILE` = 100 (`eAVEncH264VProfile_High`) en lugar de 66 (Baseline),
  en ambos caminos.
- CABAC activado (`CODECAPI_AVEncH264CABACEnable` = true; el perfil High ya lo
  habilita, se fija explícito por robustez).
- **B-frames = 0** (`CODECAPI_AVEncMPVDefaultBPictureCount` = 0, ya presente). El
  perfil High sin B-frames conserva el orden salida=entrada, así que NO rompe el
  emparejado FIFO de timestamps del Instant Replay. Los B-frames quedan explícitamente
  fuera de alcance (reordenarían la salida y exigirían reescribir la lógica de PTS/DTS
  del replay).
- Se mantiene **H.264 / MP4**. WebView2 (Chromium) reproduce High sin problema; HEVC
  exigiría extensiones no garantizadas y contradice el default de CLAUDE.md.

### GOP / keyframes

- Manual (`Encoder` de `IMFSinkWriter`): keyframe cada ~2 s
  (`CODECAPI_AVEncMPVGOPSize` = `fps * 2`). Hoy no se controla explícitamente.
- Replay (`configure_encoder_types`): subir de `fps/2` (~0.5 s) a `fps` (~1 s). Menos
  IDRs caros (mejor calidad) manteniendo granularidad fina para alinear el guardado del
  replay al keyframe anterior. No se sube a 2 s para no perder precisión de recorte.

### Aplicar a los dos caminos por igual

Manual (SinkWriter) y replay (MFT propio) comparten la misma política (VBR + High/CABAC
+ GOP), coherente con "manual y replay comparten pipeline y encoder" de CLAUDE.md. Se
configuran por vías distintas:

- **Replay**: ya posee el `IMFTransform`; se fija todo vía su `ICodecAPI` en
  `configure_encoder_types`, más `MF_MT_MPEG2_PROFILE` = 100 en el tipo de salida.
- **Manual**: `IMFSinkWriter` hospeda el MFT. Se obtiene el transform con
  `IMFSinkWriterEx::GetTransformForStream`, se castea a `ICodecAPI` y se fijan las
  mismas propiedades **antes de `BeginWriting`**; además `MF_MT_MPEG2_PROFILE` = 100 en
  el tipo de salida.

### Tiers de calidad

Se mantienen los niveles actuales (Bajo…Ultra, default "Alto") y su mapeo a
`bitrate_factor`. Solo cambia su significado interno: dejan de ser "bitrate plano" y
pasan a ser "media objetivo (mean) + pico (max = 1.75×)". Sin nuevas opciones en la UI.

## Robustez (CLAUDE.md §4.4)

Cada `ICodecAPI::SetValue` es best-effort (patrón ya existente): si un MFT no expone una
propiedad, se ignora y se sigue con el default del encoder. Nunca se aborta la captura
por un ajuste de calidad rechazado. Puntos a vigilar en implementación:

- Orden de configuración: varias propiedades de `ICodecAPI` deben fijarse antes de
  `SetOutputType`/`BeginWriting`/primer `ProcessInput`. Verificar el orden correcto.
- `MF_MT_MPEG2_LEVEL`: dejar que el encoder lo derive; vigilar solo si resoluciones/fps
  altos (p. ej. 4K120) dan error de tipo de medio.

## Verificación

- Un clip manual y un replay reales: arrancan y finalizan sin error, y el editor
  (WebView2) los reproduce.
- Comparación visual escena estática vs. acción: la calidad ya no "respira" tras cada
  keyframe y el movimiento se ve claramente mejor que en Baseline/CBR.
- Confirmar que el guardado del replay sigue alineando al keyframe (GOP ~1 s) y que la
  duración/timestamps del MP4 son correctos (el FIFO de PTS sigue intacto sin B-frames).
- Medir que no hay regresión de rendimiento en el camino de captura (uso de CPU/GPU del
  hilo de captura estable respecto a la versión previa).

## Archivos afectados (previsión)

- `src-tauri/src/capture.rs`: `Encoder::new` (manual), `configure_encoder_types`
  (replay), y helpers de bitrate (`resolve_bitrate`/nuevo `peak_bitrate`). Posibles
  imports nuevos de `CODECAPI_*` y `IMFSinkWriterEx`.
