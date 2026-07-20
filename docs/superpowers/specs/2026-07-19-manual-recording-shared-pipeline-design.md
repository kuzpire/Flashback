# Grabación manual sobre el pipeline del Instant Replay

## Problema

La grabación manual delega toda la codificación en un `IMFSinkWriter` (vídeo ARGB32→H.264 + audio PCM→AAC internos, escribiendo MP4 al vuelo). Ese camino tiene dos defectos frente al Instant Replay:

1. **Audio roto.** El AAC interno del `SinkWriter` resuelve su tipo de forma diferida y frágil; al reaplicar `SetOutputType` sobre el encoder de vídeo (hack necesario para que el VBR "prenda") se corrompe esa resolución y el primer `WriteSample` de audio falla con `MF_E_TRANSFORM_TYPE_NOT_SET`. Resultado: clips manuales sin audio.
2. **Calidad inferior.** El VBR sobre el `SinkWriter` solo se logra con un hack; el Instant Replay fija el VBR de forma limpia (rate control antes de `SetOutputType`, en `configure_encoder_types`) y por eso se ve mejor.

El Instant Replay, en cambio, funciona perfecto: encoder de vídeo propio (VBR limpio) + `build_aac_encoder` propio, ambos produciendo paquetes ya codificados que un muxer en *passthrough* vuelca a MP4. CLAUDE.md §4 ya manda que **grabación manual e Instant Replay compartan el mismo pipeline y encoder**; hoy no lo hacen.

## Objetivo

La grabación manual debe reutilizar **el pipeline de codificación del Instant Replay verbatim** (device, conversor, encoder de vídeo, pump, pistas de audio AAC) para heredar exactamente su calidad, su VBR y su sincronía audio/vídeo. La única diferencia entre ambos modos pasa a ser el **destino** de los paquetes codificados:

- Instant Replay → ring buffer en RAM acotado por segundos, muxeado al guardar.
- Grabación manual → muxer en directo a disco (grabaciones potencialmente largas: no caben en RAM).

Criterio de éxito: un clip de grabación manual sale con vídeo, audio de sistema y (si está activo) micrófono, con la misma calidad y sincronía que un clip guardado por Instant Replay. El Instant Replay no cambia su comportamiento.

## No-objetivos (fuera de alcance)

- No se toca el comportamiento del Instant Replay (ring buffer, save-on-hotkey, re-target por ventana, cartel "fuera de foco").
- La grabación manual **no** adopta las funciones específicas del replay: sin ring buffer, sin guardado por atajo, sin re-target, sin cartel de fuera de foco.
- **Resize de ventana durante la grabación manual:** se mantiene el comportamiento actual (el frame pool se crea a tamaño fijo; si la ventana crece, WGC recorta). No es una regresión (la manual vieja tampoco lo manejaba). Rebuild por resize queda como posible mejora futura.
- No se cambian los formatos de salida (MP4 + H.264 + AAC) ni la selección de encoder.

## Arquitectura

### Abstracción del destino de vídeo

El audio ya tiene un trait `AudioSink` con varias implementaciones. Se introduce el equivalente para vídeo:

```rust
trait VideoPacketSink: Send + Sync + 'static {
    fn set_seq_header(&self, bytes: Vec<u8>);            // SPS/PPS (una vez)
    fn push_video(&self, data: Vec<u8>, time: i64, dur: i64, key: bool);
}
```

El pump (`run_pump_async`/`run_pump_sync` → `drain_encoder_output`) deja de recibir `&Arc<Mutex<ReplayBuffer>>` y pasa a recibir `&Arc<dyn VideoPacketSink>`. Las llamadas concretas actuales (`buffer.lock().unwrap().seq_header = h` y `buffer.lock().unwrap().push(...)`) se sustituyen por `sink.set_seq_header(h)` / `sink.push_video(...)`.

Implementaciones:

- **`ReplayBuffer`** implementa `VideoPacketSink` (envuelto en `Arc<Mutex<…>>` como hoy vía un adaptador, o directamente). El Instant Replay queda **idéntico**.
- **`LiveMux`** implementa `VideoPacketSink` (y recibe el audio vía un `MuxAudioSink`). Es la pieza nueva.

Coste en el camino sagrado: una llamada por trait object por paquete **ya codificado** (una vez por frame, fuera del hot path de copia GPU). Despreciable.

### Audio de la grabación manual

Pasa a ser idéntico al del replay: `audio::spawn_track(..., Encoding::Aac(bitrate), ...)` → `build_aac_encoder` → paquetes AAC + `user_data` (`AudioSpecificConfig`) + `payload_type`. Un nuevo sink `MuxAudioSink` entrega esos paquetes al `LiveMux` (en lugar del `EncoderAudioSink` que empujaba PCM al `SinkWriter`).

### `LiveMux` — muxer en directo con handshake de cabeceras

`LiveMux` reutiliza las **primitivas de muxer que el guardado del replay ya usa** (`add_aac_passthrough_stream`, el stream de vídeo en passthrough con `MF_MT_MPEG_SEQUENCE_HEADER`, byte stream con `MF_MPEG4SINK_MOOV_BEFORE_MDAT` para faststart, flag `MFSampleExtension_CleanPoint` en keyframes). La única lógica nueva es alimentar el muxer **incrementalmente** en vez de desde un buffer completo, lo que obliga a un handshake de cabeceras.

Un stream en passthrough necesita sus cabeceras **al declararse** (`AddStream`): el `SPS/PPS` del vídeo y el `AudioSpecificConfig` de cada pista AAC. Esas solo se conocen tras el primer paquete codificado de cada fuente. Máquina de estados:

- **`Buffering`** (estado inicial): acumula los paquetes entrantes (vídeo y audio) en una cola pendiente **acotada** y registra las cabeceras según llegan: `seq_header` del vídeo y `user_data`/`payload_type` de cada pista de audio. Las pistas esperadas se conocen en construcción (sistema siempre que haya `sys_target`; micrófono si hay `mic_target`).
- **Transición a `Writing`** cuando: hay `seq_header` de vídeo **y** cada pista de audio esperada ha reportado su `user_data`; **o** vence un timeout (~1 s desde el primer paquete), en cuyo caso se procede con lo que haya, **descartando** las pistas de audio que aún no tengan cabecera (así un micrófono muerto no bloquea el archivo para siempre).
- **`Writing`**: se crea el `SinkWriter` en passthrough (`AddStream` de vídeo + de cada audio listo, `BeginWriting`), se vuelca la cola pendiente y a partir de ahí se escribe cada paquete en cuanto llega.
- **`Finalizing`** (al parar): `Finalize()` escribe el `moov` y cierra el archivo; devuelve la ruta.

**Rebasado temporal (igual que el guardado del replay).** El primer paquete de vídeo (que es keyframe: el encoder emite IDR al arrancar) fija `base = su timestamp`. Todo se escribe como `time - base`. El audio se rebasa contra ese mismo `base` y se descartan los paquetes con `time < base` (el contenedor no admite timestamps negativos) — exactamente el patrón de `ReplayAudioSink` + `write_audio_track`. Al reutilizar las mismas fuentes de tiempo (SystemRelativeTime de vídeo, QPC de audio) que el replay, la sincronía es la misma que el replay ya logra.

La cola pendiente está acotada por el timeout: en el caso normal las cabeceras llegan en decenas de ms; en el peor caso ~1 s de paquetes codificados (unos pocos MB). No es un ring: no descarta por tamaño, solo retiene hasta `Writing`.

### Ciclo de vida de la grabación manual (`Engine`)

El `Engine` actual (handler `note_frame` + hilo de cadencia + `Encoder`/`SinkWriter`) se reemplaza por la maquinaria del replay con un ciclo de vida propio y delgado:

- **`start`**: crea device/manager, sesión WGC, conversor, encoder (`build_encoder`), pistas de audio (`Encoding::Aac` + `MuxAudioSink`) y el `LiveMux` (con la ruta de salida). Lanza el hilo del pump (como `replay_thread`) apuntando el pump al `LiveMux`.
- **`stop`**: señaliza parada, hace join del hilo del pump (que drena lo pendiente del encoder), llama a `LiveMux::finalize()` y devuelve la ruta del archivo (`stop()` sigue devolviendo `Option<String>`).

### Reutilización / refactor

Para que ambos modos compartan la construcción del pipeline sin duplicarlo, se extrae la parte común de `build_replay` (device, conversor, encoder, ring BGRA, handler de FrameArrived, arranque de MFTs, pistas de audio) a un constructor de pipeline parametrizado por el **sink** (`dyn VideoPacketSink` + fábrica de `AudioSink`) y por las piezas específicas de cada modo (ring vs mux; cartel/re-target solo replay). `build_replay` y el nuevo `build_engine` manual consumen ese constructor común. El límite exacto del refactor lo detallará el plan de implementación; la regla es no duplicar el camino de codificación.

### Código que se elimina

`Encoder::new`, `add_aac_stream`, `EncoderAudioSink`, `push_audio`, el bucle de VBR con `GetTransformForStream`/`SetOutputType` de la manual y el handler `note_frame`/cadencia del `Engine` viejo. Con ellos desaparece el hack de VBR y el AAC interno del `SinkWriter`.

## Manejo de errores

- **Pista de audio que nunca produce cabecera** (dispositivo caído): timeout → se descarta esa pista, el vídeo (y el resto del audio) se graba igual.
- **El vídeo nunca produce keyframe** (fallo de encoder): no hay `base` → no se crea archivo; se loguea, igual que el caso de ring vacío en `save_replay`. `start` ya valida el arranque del encoder por el canal `ready`.
- **Fallo de `Finalize`**: se loguea; el archivo puede quedar incompleto, pero no compromete la estabilidad (mismo criterio que el resto del pipeline, CLAUDE.md §4.4).
- **Pérdida de dispositivo/encoder**: se hereda la recuperación del pipeline del replay (no se añade nada nuevo, pero tampoco se pierde respecto a hoy).

## Pruebas

- **Unitaria del `LiveMux`** (mirando el arnés existente `audio_reaches_back_to_video_anchor_keyframe`, que alimenta un `ReplayBuffer` con paquetes sintéticos): alimentar `LiveMux` con paquetes de vídeo+audio sintéticos y verificar (a) el gating de cabeceras (no escribe hasta tenerlas), (b) el fallback por timeout descartando una pista sin cabecera, (c) el rebasado a `base` y el descarte de audio previo al keyframe, y (d) que produce un MP4 con `moov`, streams y duración válidos.
- **Regresión del Instant Replay**: los tests existentes del ring/muxer deben seguir pasando sin cambios (prueba de que `ReplayBuffer` sobre el nuevo trait es idéntico).
- **Humo manual (usuario)**: grabar un clip manual y confirmar vídeo + audio de sistema + micrófono y calidad equivalente al replay. No automatizable (requiere GPU/WGC).

## Riesgos

- **Regresión del replay** al abstraer el sink: mitigado porque `ReplayBuffer` implementa el trait con la misma lógica y las llamadas del pump se sustituyen mecánicamente; cubierto por los tests de regresión.
- **Sincronía A/V en la manual**: se hereda de reutilizar las mismas fuentes de tiempo y rebasado del replay; el riesgo es equivalente al del replay actual (que funciona).
- **Alcance del refactor** de `build_replay`: se acota extrayendo solo la construcción común del pipeline; sin refactors no relacionados.
