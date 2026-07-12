# Encoder Quality (VBR + High/CABAC + GOP) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Mejorar la consistencia de calidad de los clips cambiando el control de tasa del encoder H.264 de CBR a VBR con techo, subiendo a perfil High con CABAC y ajustando el GOP, en los caminos manual y de Instant Replay.

**Architecture:** Un helper común (`apply_quality_codec_settings`) fija la política de calidad vía `ICodecAPI` en ambos caminos. El replay ya posee su `IMFTransform`; el manual obtiene el encoder del `IMFSinkWriter` vía `IMFSinkWriterEx::GetTransformForStream`. El perfil pasa a High (100) en el tipo de salida de ambos.

**Tech Stack:** Rust, windows-rs, Media Foundation (`ICodecAPI`, `IMFSinkWriterEx`).

## Global Constraints

- El camino de captura es sagrado: sin copias GPU→CPU nuevas, sin bloqueos.
- Cada `ICodecAPI::SetValue` es best-effort: si el MFT rechaza una propiedad, se ignora y se sigue; nunca abortar la captura (CLAUDE.md §4.4).
- B-frames = 0 siempre (el FIFO de timestamps del replay depende de que salida = entrada).
- Se mantiene H.264 / MP4. Sin comentarios decorativos; solo el *porqué* no obvio.
- Todo el código Rust vive en `src-tauri/src/capture.rs`, módulo `win`.

---

### Task 1: Helpers de calidad (peak_bitrate, variant_bool, apply_quality_codec_settings)

**Files:**
- Modify: `src-tauri/src/capture.rs` (junto a `variant_u32`, ~línea 3349; test en el módulo `tests`)

**Interfaces:**
- Produces:
  - `fn peak_bitrate(mean: u32) -> u32` — pico = mean * 7/4.
  - `fn variant_bool(val: bool) -> VARIANT`.
  - `fn apply_quality_codec_settings(codec: &ICodecAPI, mean_bitrate: u32, gop: u32)`.

- [ ] **Step 1: Añadir el test de peak_bitrate** (módulo `tests`)

```rust
        #[test]
        fn peak_bitrate_is_1_75x() {
            assert_eq!(peak_bitrate(40_000_000), 70_000_000);
            assert_eq!(peak_bitrate(0), 0);
        }
```

- [ ] **Step 2: Ejecutar y ver que falla**

Run: `cargo test -p flashback peak_bitrate_is_1_75x`
Expected: FAIL (función `peak_bitrate` no existe).

- [ ] **Step 3: Implementar los helpers** (tras `variant_u32`)

```rust
    // Pico de VBR = 1.75x la media: da margen a las escenas complejas sin disparar el
    // tamaño. saturating_mul evita overflow con bitrates muy altos (4K).
    fn peak_bitrate(mean: u32) -> u32 {
        mean.saturating_mul(7) / 4
    }

    fn variant_bool(val: bool) -> VARIANT {
        let mut v = VARIANT::default();
        unsafe {
            let inner = &mut *v.Anonymous.Anonymous;
            inner.vt = VT_BOOL;
            inner.Anonymous.boolVal =
                windows::Win32::Foundation::VARIANT_BOOL(if val { -1 } else { 0 });
        }
        v
    }

    // Política de calidad común a los dos caminos (manual y replay). Peak-Constrained VBR
    // (modo 1) con media/pico, CABAC y B-frames=0. Todo best-effort: si un MFT no expone
    // una propiedad se ignora y queda su default (CLAUDE.md §4.4).
    fn apply_quality_codec_settings(codec: &ICodecAPI, mean_bitrate: u32, gop: u32) {
        unsafe {
            let _ = codec.SetValue(&CODECAPI_AVEncCommonRateControlMode, &variant_u32(1));
            let _ = codec.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &variant_u32(mean_bitrate));
            let _ = codec.SetValue(&CODECAPI_AVEncCommonMaxBitRate, &variant_u32(peak_bitrate(mean_bitrate)));
            let _ = codec.SetValue(&CODECAPI_AVEncH264CABACEnable, &variant_bool(true));
            let _ = codec.SetValue(&CODECAPI_AVEncMPVGOPSize, &variant_u32(gop));
            let _ = codec.SetValue(&CODECAPI_AVEncMPVDefaultBPictureCount, &variant_u32(0));
        }
    }
```

- [ ] **Step 4: Añadir `VT_BOOL` al import de Variant** (línea ~150)

```rust
    use windows::Win32::System::Variant::{VARIANT, VT_BOOL, VT_UI4};
```

- [ ] **Step 5: Test pasa**

Run: `cargo test -p flashback peak_bitrate_is_1_75x`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/capture.rs
git commit -m "Add VBR quality codec helpers (peak_bitrate, apply_quality_codec_settings)"
```

---

### Task 2: Camino Instant Replay — High profile + VBR + GOP ~1s

**Files:**
- Modify: `src-tauri/src/capture.rs` → `configure_encoder_types` (~1948-1996)

**Interfaces:**
- Consumes: `apply_quality_codec_settings`, `peak_bitrate` (Task 1).

- [ ] **Step 1: Subir el perfil a High (100)**

Cambiar el bloque del comentario Baseline y la línea `SetUINT32(&MF_MT_MPEG2_PROFILE, 66)`:

```rust
            // Perfil High (100) + CABAC (lo activa apply_quality_codec_settings). Sin
            // B-frames (DefaultBPictureCount=0): el perfil High sin reordenado conserva
            // el orden salida=entrada, del que depende el emparejado FIFO de timestamps
            // (el que escupe el MFT es poco fiable: a veces sale a 0 y el MP4 queda con
            // duración nula).
            out_type.SetUINT32(&MF_MT_MPEG2_PROFILE, 100)?;
```

- [ ] **Step 2: Sustituir el bloque de GOP inline por el helper**

Reemplazar:

```rust
        if let Ok(codec) = encoder.cast::<ICodecAPI>() {
            let gop = (fps / 2).max(8).min(60);
            unsafe {
                let _ = codec.SetValue(&CODECAPI_AVEncMPVGOPSize, &variant_u32(gop));
                let _ = codec.SetValue(&CODECAPI_AVEncMPVDefaultBPictureCount, &variant_u32(0));
            }
        }
```

por:

```rust
        // GOP ~1 s (antes 0.5 s): menos IDRs caros = mejor calidad, sin perder mucha
        // granularidad para alinear el guardado del replay al keyframe anterior.
        if let Ok(codec) = encoder.cast::<ICodecAPI>() {
            apply_quality_codec_settings(&codec, bitrate, fps.max(8));
        }
```

- [ ] **Step 3: Compila**

Run: `cargo check -p flashback`
Expected: OK.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/capture.rs
git commit -m "Replay encoder: High profile, VBR rate control, ~1s GOP"
```

---

### Task 3: Camino grabación manual — High profile + VBR vía IMFSinkWriterEx

**Files:**
- Modify: `src-tauri/src/capture.rs` → `Encoder::new` (~687-721)

**Interfaces:**
- Consumes: `apply_quality_codec_settings` (Task 1).

- [ ] **Step 1: Añadir el perfil High al tipo de salida**

Tras `out_type.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;` añadir:

```rust
                out_type.SetUINT32(&MF_MT_MPEG2_PROFILE, 100)?;
```

- [ ] **Step 2: Configurar el encoder del SinkWriter antes de BeginWriting**

Justo antes de `unsafe { writer.BeginWriting()? };` insertar:

```rust
            // El encoder H.264 vive dentro del SinkWriter; se obtiene su ICodecAPI para
            // aplicar la misma política de calidad que el replay (VBR + CABAC + GOP ~2 s).
            // Best-effort: si no se puede obtener, el clip sale con perfil High pero el
            // rate control por defecto del encoder.
            if let Ok(ex) = writer.cast::<IMFSinkWriterEx>() {
                let mut transform: Option<IMFTransform> = None;
                if unsafe { ex.GetTransformForStream(stream, 0, None, &mut transform) }.is_ok() {
                    if let Some(t) = transform {
                        if let Ok(codec) = t.cast::<ICodecAPI>() {
                            apply_quality_codec_settings(&codec, bitrate, (fps * 2).max(16));
                        }
                    }
                }
            }
```

- [ ] **Step 3: Compila**

Run: `cargo check -p flashback`
Expected: OK. (Si `GetTransformForStream` tiene otra firma en esta versión de windows-rs, ajustar los parámetros `pguidcategory`/`pptransform` según el error del compilador.)

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/capture.rs
git commit -m "Manual recording encoder: High profile + VBR via IMFSinkWriterEx"
```

---

### Task 4: Verificación de build y ejecución

- [ ] **Step 1: Tests + build de la lib**

Run: `cargo test -p flashback` y `cargo build -p flashback --release`
Expected: compila y pasan los tests.

- [ ] **Step 2: Verificación funcional (manual, anotar resultado)**

- Grabar un clip manual y guardar un replay reales.
- Confirmar que el editor (WebView2) los reproduce y que la duración/timestamps son correctos.
- Comparar visualmente estático vs. acción: la calidad ya no "respira" tras cada keyframe.
