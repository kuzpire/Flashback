# Manual Recording on the Shared Encoding Pipeline — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make manual recording reuse the Instant Replay encoding pipeline (MFT video encoder with clean VBR + `build_aac_encoder` audio), writing encoded packets to a live passthrough muxer on disk instead of the fragile `IMFSinkWriter` path, so manual clips gain working audio and replay-grade quality.

**Architecture:** Introduce a `VideoPacketSink` trait so the encoder pump can target either the RAM ring buffer (replay, unchanged) or a new live muxer (manual). Build `LiveMux`, a passthrough MP4 muxer fed incrementally that waits for stream headers (video SPS/PPS + per-track AAC `AudioSpecificConfig`) before `BeginWriting`, then streams packets and `Finalize`s on stop. Rewire the manual capture path to build the replay-style pipeline pointed at `LiveMux`, and delete the old `SinkWriter`-based `Encoder`.

**Tech Stack:** Rust, Windows Media Foundation (`windows` crate), Direct3D 11, WGC. All capture code lives in `src-tauri/src/capture.rs` (module `win`), audio in `src-tauri/src/audio.rs`.

## Global Constraints

- Language of comments/UX copy: Spanish. Commit messages: English. (CLAUDE.md §9)
- No decorative comments; comment only non-obvious *why* (performance / counter-intuitive decisions). (CLAUDE.md §9)
- Commit authorship: owner only. NEVER add `Co-Authored-By: Claude` or any Claude/Generated-with attribution to commits or PRs. (CLAUDE.md §9)
- The capture/encode path is sacred: no avoidable allocations/copies, no GPU→CPU copies, keep zero-copy GPU path. New per-packet cost limited to one trait-object dispatch per already-encoded packet (not in the GPU hot path). (CLAUDE.md §4)
- Manual recording and Instant Replay must share the same encoder/pipeline. (CLAUDE.md §4)
- Output stays MP4 + H.264 + AAC. Export is the only path that re-encodes; capture/replay-save never do. (CLAUDE.md §2)
- Instant Replay behavior must not change. Its existing tests must keep passing unmodified.
- Windows-only code lives under `#[cfg(target_os = "windows")] mod win`. The non-Windows stubs for `start`/`stop` already exist and need no change.
- Build check command (cwd matters — run from `src-tauri`): `cargo check`. Test command: `cargo test` (runs the `#[cfg(test)] mod tests` inside `capture.rs`).

---

## File structure

- `src-tauri/src/capture.rs` — all changes below. It is already a large single file organized as `mod win`; follow that structure (do NOT split the file — matches the established pattern). New items (`VideoPacketSink`, `LiveMux`, `MuxAudioSink`, `add_h264_passthrough_stream`, `build_pipeline_core`, new manual `capture_thread`) go in `mod win` near the code they relate to.
- `src-tauri/src/audio.rs` — no changes expected (the `AudioSink` trait and `Encoding::Aac` already exist and are reused as-is).

---

## Task 1: `VideoPacketSink` trait + make the encoder pump sink-agnostic

Foundational refactor. After this, the pump writes through a trait; replay behaves identically because `ReplayBuffer` implements the trait with its current logic.

**Files:**
- Modify: `src-tauri/src/capture.rs`
  - `drain_encoder_output` (currently `capture.rs:2882`)
  - `feed_encoder_async` (`capture.rs:2203`)
  - `run_pump` / `run_pump_async` / `run_pump_sync` (`capture.rs:2154`, `2240`, `2501`)
  - `run_encoder_thread` (`capture.rs:2395`)
  - `ReplayBuffer` impl (`capture.rs:999`)
- Test: existing `#[cfg(test)] mod tests` in `capture.rs` (`capture.rs:~3940`)

**Interfaces:**
- Produces:
  ```rust
  trait VideoPacketSink: Send + Sync + 'static {
      fn set_seq_header(&self, bytes: Vec<u8>);
      fn push_video(&self, data: Vec<u8>, time: i64, dur: i64, key: bool);
  }
  ```
  and an impl for the ring buffer so the pump can hold `&Arc<dyn VideoPacketSink>`.

- [ ] **Step 1: Add the trait and implement it for the ring buffer.**

Add near the `ReplayBuffer` definition (after its `impl`, around `capture.rs:1085`):

```rust
// Destino de los paquetes de vídeo ya codificados que emite el pump. Dos implementaciones:
// el ring buffer del Instant Replay (RAM, acotado por segundos) y el muxer en directo de la
// grabación manual (disco). El pump habla con este trait para compartir un solo pipeline de
// codificación entre ambos modos (CLAUDE.md §4).
trait VideoPacketSink: Send + Sync + 'static {
    fn set_seq_header(&self, bytes: Vec<u8>);
    fn push_video(&self, data: Vec<u8>, time: i64, dur: i64, key: bool);
}

impl VideoPacketSink for Mutex<ReplayBuffer> {
    fn set_seq_header(&self, bytes: Vec<u8>) {
        self.lock().unwrap().seq_header = bytes;
    }
    fn push_video(&self, data: Vec<u8>, time: i64, dur: i64, key: bool) {
        self.lock().unwrap().push(data, time, dur, key);
    }
}
```

- [ ] **Step 2: Change the pump signatures from `&Arc<Mutex<ReplayBuffer>>` to `&Arc<dyn VideoPacketSink>`.**

In each of `run_pump`, `run_pump_async`, `run_pump_sync`, `feed_encoder_async`, `run_encoder_thread`, `drain_encoder_output`, replace the parameter
`buffer: &Arc<Mutex<ReplayBuffer>>` with `sink: &Arc<dyn VideoPacketSink>` and thread it through unchanged. Inside `drain_encoder_output`, replace the two concrete writes:

```rust
// before:
buffer.lock().unwrap().seq_header = h;      // ~capture.rs:2902
buffer.lock().unwrap().seq_header = ps;     // ~capture.rs:2921
buffer.lock().unwrap().push(data, time, dur, key);  // ~capture.rs:2928
// after:
sink.set_seq_header(h);
sink.set_seq_header(ps);
sink.push_video(data, time, dur, key);
```

`run_pump` forwards `sink` to `run_pump_async`/`run_pump_sync`; those forward to `feed_encoder_async`/`run_encoder_thread`/`drain_encoder_output`. No other body changes.

- [ ] **Step 3: Update `replay_thread` to pass a trait object.**

In `replay_thread` (`capture.rs:1368`) the call is `run_pump(&pipe, &stop, &buffer, window_mode);` where `buffer: Arc<Mutex<ReplayBuffer>>`. Create the trait object once before the loop:

```rust
// El pump escribe a través de VideoPacketSink; el replay usa el ring buffer como sink.
let video_sink: Arc<dyn VideoPacketSink> = buffer.clone();
```

and change the call to `run_pump(&pipe, &stop, &video_sink, window_mode);`. `save_replay`/`mux_replay` keep using `buffer` directly (they read the ring, not the trait).

- [ ] **Step 4: Build.**

Run (from `src-tauri`): `cargo check`
Expected: compiles with no errors. Fix any missed call site (the compiler lists them).

- [ ] **Step 5: Run existing tests to prove replay is unaffected.**

Run (from `src-tauri`): `cargo test`
Expected: PASS, including `audio_reaches_back_to_video_anchor_keyframe` and `selector_jumps_after_freeze`.

- [ ] **Step 6: Commit.**

```bash
git add src-tauri/src/capture.rs
git commit -m "Capture: route the encoder pump through a VideoPacketSink trait"
```

---

## Task 2: `add_h264_passthrough_stream` helper (shared by mux_replay and LiveMux)

Extract the video passthrough stream declaration so `LiveMux` reuses the exact same setup `mux_replay` already uses.

**Files:**
- Modify: `src-tauri/src/capture.rs` (`mux_replay` at `capture.rs:3060-3075`; add helper next to `add_aac_passthrough_stream` at `capture.rs:3137`)

**Interfaces:**
- Produces: `fn add_h264_passthrough_stream(writer: &IMFSinkWriter, seq_header: &[u8], width: u32, height: u32, fps: u32, bitrate: u32) -> Result<u32>`

- [ ] **Step 1: Add the helper** (next to `add_aac_passthrough_stream`):

```rust
// Declara el stream de vídeo en passthrough (entrada == salida, H.264 ya codificado). El
// SPS/PPS viaja en MF_MT_MPEG_SEQUENCE_HEADER para que el sink MP4 escriba el `avcC`.
fn add_h264_passthrough_stream(
    writer: &IMFSinkWriter,
    seq_header: &[u8],
    width: u32,
    height: u32,
    fps: u32,
    bitrate: u32,
) -> Result<u32> {
    let h264 = unsafe { MFCreateMediaType()? };
    unsafe {
        h264.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        h264.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
        h264.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
        h264.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        h264.SetUINT64(&MF_MT_FRAME_SIZE, pack2(width, height))?;
        h264.SetUINT64(&MF_MT_FRAME_RATE, pack2(fps, 1))?;
        h264.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack2(1, 1))?;
        if !seq_header.is_empty() {
            h264.SetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, seq_header)?;
        }
    }
    let stream = unsafe { writer.AddStream(&h264)? };
    unsafe { writer.SetInputMediaType(stream, &h264, None)? };
    Ok(stream)
}
```

- [ ] **Step 2: Use it in `mux_replay`.** Replace the inline block at `capture.rs:3060-3075` (the `let h264 = ...` through `SetInputMediaType(stream, &h264, None)?`) with:

```rust
let stream = add_h264_passthrough_stream(&writer, seq_header, width, height, fps, bitrate)?;
```

- [ ] **Step 3: Build & test.**

Run (from `src-tauri`): `cargo check && cargo test`
Expected: PASS (mux_replay behavior unchanged).

- [ ] **Step 4: Commit.**

```bash
git add src-tauri/src/capture.rs
git commit -m "Capture: extract add_h264_passthrough_stream shared by the muxers"
```

---

## Task 3: `LiveMux` — live passthrough muxer with header handshake

The new sink for manual recording. Buffers packets until headers are known, then declares streams, `BeginWriting`, flushes, streams live, and `Finalize`s on stop. Rebases everything to the first video keyframe (mirrors `mux_replay`).

**Files:**
- Modify: `src-tauri/src/capture.rs` (add `LiveMux` and its `#[cfg(test)]` tests)

**Interfaces:**
- Consumes: `VideoPacketSink` (Task 1), `add_h264_passthrough_stream` (Task 2), existing `add_aac_passthrough_stream` (`capture.rs:3137`), `AudioMuxTrack` (`capture.rs:965`).
- Produces:
  - `struct LiveMux` implementing `VideoPacketSink`.
  - `LiveMux::new(path: String, fps: u32, bitrate: u32, sys: Option<(u32,u16)>, mic: Option<(u32,u16)>) -> Arc<LiveMux>` — `sys`/`mic` are `(sample_rate, channels)` of the *encoded* (downmixed) AAC track; `None` = track absent.
  - `LiveMux::push_audio(&self, role: AudioRole, data: Vec<u8>, time: i64, dur: i64)`
  - `LiveMux::set_audio_header(&self, role: AudioRole, user_data: Vec<u8>, payload_type: u32)`
  - `LiveMux::finalize(&self) -> Option<String>` — returns the written path, or `None` on failure/no video.

- [ ] **Step 1: Write the failing tests.**

Add inside the `#[cfg(test)] mod tests` block (near `capture.rs:3990`). These use synthetic packets, like the existing `audio_reaches_back_to_video_anchor_keyframe`. Note LiveMux writes a real MP4 via MF, so tests call `ensure_mf()` and write to a temp path, then assert the file exists and is non-trivial in size (a full MP4 structural parse is out of scope; the smoke assertion is "MF accepted the packets and Finalize produced a file").

```rust
#[test]
fn livemux_waits_for_headers_before_writing() {
    ensure_mf();
    let dir = std::env::temp_dir();
    let path = dir.join("flashback_livemux_test1.mp4").to_string_lossy().into_owned();
    let _ = std::fs::remove_file(&path);
    let mux = LiveMux::new(path.clone(), 30, 4_000_000, Some((48_000, 2)), None);

    // Vídeo llega antes que la cabecera de audio: no debe escribir todavía.
    mux.set_seq_header(vec![0u8; 32]);
    mux.push_video(vec![0u8; 64], 0, 333_333, true);
    assert!(!mux.is_writing(), "no debe arrancar sin la cabecera de la pista de audio esperada");

    // Llega la cabecera de audio esperada -> transición a Writing.
    mux.set_audio_header(AudioRole::Sys, vec![0u8; 2], 0);
    mux.push_audio(AudioRole::Sys, vec![0u8; 16], 0, 213_333);
    assert!(mux.is_writing(), "con vídeo + cabecera de audio debe estar escribiendo");

    let out = mux.finalize();
    assert_eq!(out.as_deref(), Some(path.as_str()));
    assert!(std::fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn livemux_timeout_drops_missing_audio_track() {
    ensure_mf();
    let dir = std::env::temp_dir();
    let path = dir.join("flashback_livemux_test2.mp4").to_string_lossy().into_owned();
    let _ = std::fs::remove_file(&path);
    // Se espera micrófono además de sistema, pero el micrófono nunca reporta cabecera.
    let mux = LiveMux::new(path.clone(), 30, 4_000_000, Some((48_000, 2)), Some((48_000, 1)));
    mux.set_header_timeout(std::time::Duration::from_millis(0)); // fuerza el fallback ya

    mux.set_seq_header(vec![0u8; 32]);
    mux.set_audio_header(AudioRole::Sys, vec![0u8; 2], 0);
    mux.push_video(vec![0u8; 64], 0, 333_333, true); // dispara la evaluación del handshake
    assert!(mux.is_writing(), "con timeout vencido debe arrancar descartando el micrófono");

    let out = mux.finalize();
    assert_eq!(out.as_deref(), Some(path.as_str()));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn livemux_no_video_returns_none() {
    ensure_mf();
    let mux = LiveMux::new("nonexistent.mp4".into(), 30, 4_000_000, Some((48_000, 2)), None);
    // Nunca llega vídeo: no hay base, no se abre archivo.
    assert_eq!(mux.finalize(), None);
}
```

- [ ] **Step 2: Run tests to verify they fail.**

Run (from `src-tauri`): `cargo test livemux`
Expected: FAIL to compile (`LiveMux` not defined).

- [ ] **Step 3: Implement `LiveMux`.**

Add near `mux_replay` (`capture.rs:~3010`). The muxer object and its `IMFSinkWriter` live behind a `Mutex` so the trait methods (`&self`) can mutate. All packets carry absolute times; `base` is the first keyframe's time; everything is written as `time - base`; audio before `base` is dropped.

```rust
// Estado interno del muxer en directo, tras el lock.
struct LiveMuxState {
    writer: Option<IMFSinkWriter>,
    video_stream: u32,
    sys_stream: Option<u32>,
    mic_stream: Option<u32>,
    base: i64,               // i64::MIN hasta el primer keyframe de vídeo
    seq_header: Vec<u8>,
    // Cabeceras de audio recogidas durante el handshake.
    sys_hdr: Option<(Vec<u8>, u32)>, // (user_data, payload_type)
    mic_hdr: Option<(Vec<u8>, u32)>,
    // Cola pendiente mientras se esperan cabeceras. (role=None => vídeo)
    pending: Vec<(Option<AudioRole>, Vec<u8>, i64, i64, bool)>,
    writing: bool,
    finalized: bool,
    first_pkt_at: Option<Instant>,
    failed: bool,
}

struct LiveMux {
    path: String,
    fps: u32,
    bitrate: u32,
    // Pistas de audio esperadas (sample_rate, channels) del AAC ya downmezclado; None = ausente.
    sys: Option<(u32, u16)>,
    mic: Option<(u32, u16)>,
    header_timeout: Mutex<Duration>,
    st: Mutex<LiveMuxState>,
}

impl LiveMux {
    fn new(
        path: String,
        fps: u32,
        bitrate: u32,
        sys: Option<(u32, u16)>,
        mic: Option<(u32, u16)>,
    ) -> Arc<LiveMux> {
        Arc::new(LiveMux {
            path,
            fps,
            bitrate,
            sys,
            mic,
            header_timeout: Mutex::new(Duration::from_millis(1000)),
            st: Mutex::new(LiveMuxState {
                writer: None,
                video_stream: 0,
                sys_stream: None,
                mic_stream: None,
                base: i64::MIN,
                seq_header: Vec::new(),
                sys_hdr: None,
                mic_hdr: None,
                pending: Vec::new(),
                writing: false,
                finalized: false,
                first_pkt_at: None,
                failed: false,
            }),
        })
    }

    #[cfg(test)]
    fn is_writing(&self) -> bool {
        self.st.lock().unwrap().writing
    }
    #[cfg(test)]
    fn set_header_timeout(&self, d: Duration) {
        *self.header_timeout.lock().unwrap() = d;
    }

    fn set_audio_header(&self, role: AudioRole, user_data: Vec<u8>, payload_type: u32) {
        let mut st = self.st.lock().unwrap();
        match role {
            AudioRole::Sys => st.sys_hdr = Some((user_data, payload_type)),
            AudioRole::Mic => st.mic_hdr = Some((user_data, payload_type)),
        }
    }

    fn push_audio(&self, role: AudioRole, data: Vec<u8>, time: i64, dur: i64) {
        let mut st = self.st.lock().unwrap();
        if st.finalized || st.failed {
            return;
        }
        st.first_pkt_at.get_or_insert_with(Instant::now);
        if st.writing {
            self.write_one(&mut st, Some(role), data, time, dur, false);
        } else {
            st.pending.push((Some(role), data, time, dur, false));
            self.try_begin(&mut st);
        }
    }

    // ¿Están todas las cabeceras esperadas, o venció el timeout?
    fn headers_ready(&self, st: &LiveMuxState) -> bool {
        if st.seq_header.is_empty() {
            return false;
        }
        let sys_ok = self.sys.is_none() || st.sys_hdr.is_some();
        let mic_ok = self.mic.is_none() || st.mic_hdr.is_some();
        if sys_ok && mic_ok {
            return true;
        }
        // Fallback: si venció el timeout, arrancar con lo que haya (se descartan pistas sin cabecera).
        let timeout = *self.header_timeout.lock().unwrap();
        st.first_pkt_at.map(|t| t.elapsed() >= timeout).unwrap_or(false)
    }

    // Intenta abrir el SinkWriter y volcar lo pendiente. Requiere que ya exista `base`
    // (primer keyframe) y que headers_ready() sea true.
    fn try_begin(&self, st: &mut LiveMuxState) {
        if st.writing || st.failed || st.base == i64::MIN || !self.headers_ready(st) {
            return;
        }
        match self.open_writer(st) {
            Ok(()) => {
                st.writing = true;
                // Volcar la cola pendiente (rebasada; audio previo a base se descarta en write_one).
                let pending = std::mem::take(&mut st.pending);
                for (role, data, time, dur, key) in pending {
                    self.write_one(st, role, data, time, dur, key);
                }
            }
            Err(e) => {
                eprintln!("grabación manual: no se pudo abrir el muxer en directo: {e:?}");
                st.failed = true;
                st.pending.clear();
            }
        }
    }

    // Crea el SinkWriter passthrough (faststart) y declara los streams disponibles.
    fn open_writer(&self, st: &mut LiveMuxState) -> Result<()> {
        let url = HSTRING::from(self.path.as_str());
        let byte_stream = unsafe {
            MFCreateFile(MF_ACCESSMODE_READWRITE, MF_OPENMODE_DELETE_IF_EXIST, MF_FILEFLAGS_NONE, &url)?
        };
        if let Ok(bs_attr) = byte_stream.cast::<IMFAttributes>() {
            unsafe { let _ = bs_attr.SetUINT32(&MF_MPEG4SINK_MOOV_BEFORE_MDAT, 1); }
        }
        let attrs = unsafe {
            let mut a: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut a, 2)?;
            let a = a.ok_or_else(null_out)?;
            a.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
            a.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
            a
        };
        let writer = unsafe { MFCreateSinkWriterFromURL(PCWSTR::null(), &byte_stream, &attrs)? };

        let video_stream = add_h264_passthrough_stream(
            &writer, &st.seq_header, 0, 0, self.fps, self.bitrate,
        )?;
        // width/height van en 0 arriba a propósito: se corrigen aquí a partir de que el SPS los
        // lleva embebidos. En la práctica el muxer del replay los pasa explícitos; para el vivo
        // los tomamos de los campos del LiveMux si se añaden. (Ver nota de implementación.)

        // Pistas de audio: solo las que tienen cabecera. Sistema primero (pista por defecto).
        let mut sys_stream = None;
        if let (Some((rate, ch)), Some((ud, pt))) = (self.sys, st.sys_hdr.clone()) {
            let track = AudioMuxTrack {
                packets: Vec::new(),
                sample_rate: rate,
                channels: ch,
                bitrate: aac_bitrate(ch),
                user_data: ud,
                payload_type: pt,
            };
            sys_stream = Some(add_aac_passthrough_stream(&writer, &track)?);
        }
        let mut mic_stream = None;
        if let (Some((rate, ch)), Some((ud, pt))) = (self.mic, st.mic_hdr.clone()) {
            let track = AudioMuxTrack {
                packets: Vec::new(),
                sample_rate: rate,
                channels: ch,
                bitrate: aac_bitrate(ch),
                user_data: ud,
                payload_type: pt,
            };
            mic_stream = Some(add_aac_passthrough_stream(&writer, &track)?);
        }

        unsafe { writer.BeginWriting()? };
        st.writer = Some(writer);
        st.video_stream = video_stream;
        st.sys_stream = sys_stream;
        st.mic_stream = mic_stream;
        Ok(())
    }

    // Escribe un sample rebasado a `base`. `role=None` => vídeo. Descarta audio con time < base.
    fn write_one(
        &self,
        st: &mut LiveMuxState,
        role: Option<AudioRole>,
        data: Vec<u8>,
        time: i64,
        dur: i64,
        key: bool,
    ) {
        let Some(writer) = st.writer.clone() else { return };
        let ts = time - st.base;
        if ts < 0 {
            return; // audio anterior al primer keyframe: el contenedor no admite negativos
        }
        let stream = match role {
            None => st.video_stream,
            Some(AudioRole::Sys) => match st.sys_stream { Some(s) => s, None => return },
            Some(AudioRole::Mic) => match st.mic_stream { Some(s) => s, None => return },
        };
        let Ok(mf_buf) = (unsafe { MFCreateMemoryBuffer(data.len() as u32) }) else { return };
        let ok = unsafe {
            let mut ptr: *mut u8 = std::ptr::null_mut();
            if mf_buf.Lock(&mut ptr, None, None).is_err() {
                false
            } else {
                std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
                let _ = mf_buf.Unlock();
                mf_buf.SetCurrentLength(data.len() as u32).is_ok()
            }
        };
        if !ok {
            return;
        }
        let Ok(sample) = (unsafe { MFCreateSample() }) else { return };
        unsafe {
            let _ = sample.AddBuffer(&mf_buf);
            let _ = sample.SetSampleTime(ts);
            let _ = sample.SetSampleDuration(dur);
            if role.is_none() && key {
                let _ = sample.SetUINT32(&MFSampleExtension_CleanPoint, 1);
            }
            if writer.WriteSample(stream, &sample).is_err() && !st.failed {
                st.failed = true;
                eprintln!("grabación manual: WriteSample falló; se detiene el muxer en directo");
            }
        }
    }

    fn finalize(&self) -> Option<String> {
        let mut st = self.st.lock().unwrap();
        if st.finalized {
            return None;
        }
        st.finalized = true;
        let writer = st.writer.take()?;
        if st.failed {
            return None;
        }
        match unsafe { writer.Finalize() } {
            Ok(()) => Some(self.path.clone()),
            Err(e) => {
                eprintln!("grabación manual: Finalize del muxer falló: {e:?}");
                None
            }
        }
    }
}

impl VideoPacketSink for LiveMux {
    fn set_seq_header(&self, bytes: Vec<u8>) {
        let mut st = self.st.lock().unwrap();
        if st.seq_header.is_empty() {
            st.seq_header = bytes;
            self.try_begin(&mut st);
        }
    }
    fn push_video(&self, data: Vec<u8>, time: i64, dur: i64, key: bool) {
        let mut st = self.st.lock().unwrap();
        if st.finalized || st.failed {
            return;
        }
        st.first_pkt_at.get_or_insert_with(Instant::now);
        // El primer keyframe fija la base temporal. Antes de eso se descarta (no se puede
        // arrancar un MP4 fuera de un IDR, igual que save_replay).
        if st.base == i64::MIN {
            if !key {
                return;
            }
            st.base = time;
        }
        if st.writing {
            self.write_one(&mut st, None, data, time, dur, key);
        } else {
            st.pending.push((None, data, time, dur, key));
            self.try_begin(&mut st);
        }
    }
}
```

**Implementation note for the executor (width/height):** `add_h264_passthrough_stream` needs the coded width/height. Add `width: u32, height: u32` fields to `LiveMux` (set in `new`, plumbed from the pipeline's `out_w`/`out_h` in Task 4) and pass them in `open_writer` instead of `0, 0`. Update `LiveMux::new`'s signature to `new(path, width, height, fps, bitrate, sys, mic)` and the Task-3 tests accordingly (use `1920, 1080`). This keeps the muxer self-contained.

- [ ] **Step 4: Run tests to verify they pass.**

Run (from `src-tauri`): `cargo test livemux`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit.**

```bash
git add src-tauri/src/capture.rs
git commit -m "Capture: add LiveMux live passthrough muxer with header handshake"
```

---

## Task 4: `MuxAudioSink` — feed AAC packets/headers into LiveMux

**Files:**
- Modify: `src-tauri/src/capture.rs` (near the other `AudioSink` impls, `capture.rs:~2106`)

**Interfaces:**
- Consumes: `audio::AudioSink` trait (`audio.rs:54`), `LiveMux` (Task 3), `AudioRole` (`capture.rs:~2128`).
- Produces: `struct MuxAudioSink { mux: Arc<LiveMux>, role: AudioRole }` implementing `audio::AudioSink`.

The replay's audio track is spawned with `Encoding::Aac`, whose sink receives already-encoded AAC via `push`, the `AudioSpecificConfig` via `set_user_data`, and the framing via `set_payload_type` (see `audio.rs:56-62` and `drain_aac` at `audio.rs:598`). `MuxAudioSink` collects the header (both callbacks) and forwards packets.

- [ ] **Step 1: Implement the sink.**

```rust
// Sink de audio de la grabación manual: entrega los paquetes AAC (ya codificados por
// build_aac_encoder, igual que el replay) al muxer en directo. La cabecera del AAC llega por
// set_user_data + set_payload_type antes del primer paquete drenado.
struct MuxAudioSink {
    mux: Arc<LiveMux>,
    role: AudioRole,
    user_data: Mutex<Vec<u8>>,
    payload_type: AtomicU32,
    header_sent: AtomicBool,
}

impl MuxAudioSink {
    fn new(mux: Arc<LiveMux>, role: AudioRole) -> MuxAudioSink {
        MuxAudioSink {
            mux,
            role,
            user_data: Mutex::new(Vec::new()),
            payload_type: AtomicU32::new(0),
            header_sent: AtomicBool::new(false),
        }
    }
    fn maybe_send_header(&self) {
        // Se envía una vez, cuando ya hay user_data (el AudioSpecificConfig). payload_type llega
        // en la misma pasada de drenado; si por orden llegara después, el valor por defecto 0
        // (AAC crudo) coincide con lo que build_aac_encoder produce.
        if self.header_sent.load(Ordering::SeqCst) {
            return;
        }
        let ud = self.user_data.lock().unwrap().clone();
        if ud.is_empty() {
            return;
        }
        self.mux.set_audio_header(self.role, ud, self.payload_type.load(Ordering::SeqCst));
        self.header_sent.store(true, Ordering::SeqCst);
    }
}

impl audio::AudioSink for MuxAudioSink {
    fn push(&self, data: Vec<u8>, time: i64, dur: i64) {
        self.maybe_send_header();
        self.mux.push_audio(self.role, data, time, dur);
    }
    fn set_user_data(&self, data: Vec<u8>) {
        *self.user_data.lock().unwrap() = data;
        self.maybe_send_header();
    }
    fn set_payload_type(&self, v: u32) {
        self.payload_type.store(v, Ordering::SeqCst);
    }
}
```

- [ ] **Step 2: Build.**

Run (from `src-tauri`): `cargo check`
Expected: compiles (may warn `MuxAudioSink` unused until Task 5 — acceptable this step).

- [ ] **Step 3: Commit.**

```bash
git add src-tauri/src/capture.rs
git commit -m "Capture: add MuxAudioSink forwarding AAC to LiveMux"
```

---

## Task 5: Extract `build_pipeline_core` shared by replay and manual

Pull the device→encoder→converter→BGRA-ring→FrameArrived-handler→MFT-start→`ReplayPipeline` assembly out of `build_replay` so the manual builder reuses it verbatim. The audio-track spawning, the ring-buffer setup, and the window-only card/retarget stay in `build_replay` (replay-specific).

**Files:**
- Modify: `src-tauri/src/capture.rs` (`build_replay` at `capture.rs:1502-1825`)

**Interfaces:**
- Produces: `fn build_pipeline_core(item: GraphicsCaptureItem, fps: u32, factor: f64, resolution: u32, bitrate_override: u32, encoder_pref: &str, window_mode: bool, video_sink: Arc<dyn VideoPacketSink>, video_base: Arc<AtomicI64>) -> Result<(ReplayPipeline, u32, u32, ID3D11Device)>` — returns the pipeline plus coded `(out_w, out_h)` and a clone of the `ID3D11Device` (so the caller can build audio sinks / overlay). The pump already writes through `video_sink`.

Because `run_encoder_thread`/`run_pump_*` now take `&Arc<dyn VideoPacketSink>` (Task 1) but currently receive `buffer` from `replay_thread`, the pipeline itself does not store the sink; the sink is passed to `run_pump` by the thread function. Therefore `build_pipeline_core` does NOT need to own the sink for the pump — keep passing the sink at `run_pump` call sites. The `video_sink`/`video_base` params exist only for wiring audio sinks in the caller. **Simplify:** drop `video_sink` from `build_pipeline_core`'s signature; keep `video_base`.

Final signature:
```rust
fn build_pipeline_core(
    item: GraphicsCaptureItem,
    fps: u32,
    factor: f64,
    resolution: u32,
    bitrate_override: u32,
    encoder_pref: &str,
    window_mode: bool,
    video_base: Arc<AtomicI64>,
) -> Result<PipelineCore>

struct PipelineCore {
    pipe: ReplayPipeline,   // card=None, game_hwnd resuelto si window_mode
    device: ID3D11Device,
    out_w: u32,
    out_h: u32,
    bitrate: u32,
}
```

- [ ] **Step 1: Introduce `PipelineCore` and `build_pipeline_core`** by moving lines `capture.rs:1517-1716` (create_device through the `FrameArrived` token + MFT start) plus the `card`/`game_hwnd` resolution (`capture.rs:1756-1779`) into it, returning `PipelineCore`. The `ReplayPipeline` built here has `card`/`game_hwnd` for window mode and empty `audio_tracks` (the caller fills them). Keep `video_base` as a param so both callers share one base.

- [ ] **Step 2: Rewrite `build_replay` to call `build_pipeline_core`** then do the replay-only parts: `buffer.lock().init_audio(...)` + width/height/fps/bitrate stamping (`capture.rs:1550-1564`), spawn audio tracks with `ReplayAudioSink` (`capture.rs:1724-1754`), attach them to the pipeline, and return it. `build_replay`'s public signature and return type stay the same.

- [ ] **Step 3: Build & test (replay regression).**

Run (from `src-tauri`): `cargo check && cargo test`
Expected: PASS. Replay behavior identical.

- [ ] **Step 4: Commit.**

```bash
git add src-tauri/src/capture.rs
git commit -m "Capture: extract build_pipeline_core shared by replay and manual"
```

---

## Task 6: Rewire the manual capture path onto the shared pipeline; delete the old encoder

Replace `build_engine`/`Engine`/`capture_thread`'s cadence loop with the shared pipeline pumping into a `LiveMux`, and delete the `SinkWriter`-based `Encoder` and its helpers.

**Files:**
- Modify: `src-tauri/src/capture.rs`
  - `capture_thread` (`capture.rs:321-408`)
  - `Engine` struct + `build_engine` (`capture.rs:410-803` region — this whole block houses `Engine`, `build_engine`, and `Encoder`)
  - `pub fn stop` (`capture.rs:290`) — unchanged in signature; still returns `running.result`
- Delete: `Encoder` struct + `impl` (incl. `note_frame`, `emit_paced`, `latest_slot`, `push_audio`, `finalize`, the VBR `GetTransformForStream`/`SetOutputType` loop), `add_aac_stream` (`capture.rs:2084`), `EncoderAudioSink` (`capture.rs:2113`), and the cadence loop in `capture_thread`.

**Interfaces:**
- Consumes: `build_pipeline_core`/`PipelineCore` (Task 5), `LiveMux` (Task 3), `MuxAudioSink` (Task 4), `run_pump` (Task 1), `audio::spawn_track` + `Encoding::Aac` (`audio.rs:160`), `aac_target_format`/`probe_format` (`audio.rs`), `teardown_replay` (`capture.rs:1425`).

- [ ] **Step 1: Rewrite `capture_thread`** to mirror the (monitor-mode) replay flow but with a `LiveMux` sink and a manual lifecycle (no ring buffer, no save, no reconnection loop). Replacement body:

```rust
#[allow(clippy::too_many_arguments)]
fn capture_thread(
    target: String,
    out_dir: String,
    fps: u32,
    factor: f64,
    resolution: u32,
    bitrate_override: u32,
    mic: bool,
    mic_device: String,
    encoder_pref: String,
    stop: Arc<(Mutex<bool>, Condvar)>,
    stats: Arc<Stats>,
    result: Arc<Mutex<Option<String>>>,
    ready: mpsc::Sender<std::result::Result<(), String>>,
) {
    unsafe { let _ = CoInitializeEx(None, COINIT_MULTITHREADED); }
    let _timer = TimerRes::new();
    let _mmcss = MmcssTask::new("Capture");

    let window_mode = target == "window";
    // Un stop atómico para el pump (que espera un Arc<AtomicBool>); el par Mutex/Condvar de la
    // API pública se traduce a ese atómico en un hilo puente para reaccionar al instante.
    let pump_stop = Arc::new(AtomicBool::new(false));

    let built = resolve_target_item(&target).and_then(|item| {
        build_manual(&stats, item, &out_dir, fps, factor, resolution, bitrate_override, mic,
                     mic_device, &encoder_pref, window_mode)
            .map_err(|e| format!("{e:?}"))
    });
    let (pipe, mux, video_sink) = match built {
        Ok(v) => { let _ = ready.send(Ok(())); v }
        Err(e) => { let _ = ready.send(Err(e)); unsafe { CoUninitialize() }; return; }
    };

    // Puente stop: espera la señal pública y la traduce al atómico del pump.
    {
        let (lock, cv) = &*stop;
        let mut stopped = lock.lock().unwrap();
        while !*stopped {
            let (s, _) = cv.wait_timeout(stopped, Duration::from_millis(100)).unwrap();
            stopped = s;
        }
        pump_stop.store(true, Ordering::SeqCst);
    }
    // (run_pump corre en ESTE hilo tras arrancar un hilo puente; ver Step 2 para el orden real.)

    let _ = (&pipe, &mux, &video_sink);
    unsafe { CoUninitialize() };
}
```

**Note:** Step 1 sketches the pieces; Step 2 fixes the concurrency so `run_pump` runs while the stop bridge waits. Implement Step 2's version.

- [ ] **Step 2: Correct concurrency — run the pump, bridge stop on a helper thread.** Replace the tail of `capture_thread` (from the stop bridge onward) with:

```rust
    // Hilo puente: traduce el stop público (Mutex/Condvar) al atómico del pump.
    let bridge_stop = pump_stop.clone();
    let bridge = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let (lock, cv) = &*stop;
            let mut stopped = lock.lock().unwrap();
            while !*stopped {
                let (s, _) = cv.wait_timeout(stopped, Duration::from_millis(100)).unwrap();
                stopped = s;
            }
            bridge_stop.store(true, Ordering::SeqCst);
        })
    };

    // Bombeo en este hilo (dueño de los COM/WGC del pipeline).
    run_pump(&pipe, &pump_stop, &video_sink, window_mode);

    // Orden de parada: cortar fuentes (WGC + audio) para que el flush final vea las colas
    // completas, luego cerrar el MP4.
    teardown_replay(pipe);
    *result.lock().unwrap() = mux.finalize();

    pump_stop.store(true, Ordering::SeqCst);
    let _ = bridge.join();
    unsafe { CoUninitialize() };
```

- [ ] **Step 3: Add `build_manual`** (next to `build_replay`). Returns the pipeline, the `LiveMux` (for finalize), and the video sink trait object (for `run_pump`).

```rust
#[allow(clippy::too_many_arguments)]
fn build_manual(
    stats: &Arc<Stats>,
    item: GraphicsCaptureItem,
    out_dir: &str,
    fps: u32,
    factor: f64,
    resolution: u32,
    bitrate_override: u32,
    mic: bool,
    mic_device: String,
    encoder_pref: &str,
    window_mode: bool,
) -> Result<(ReplayPipeline, Arc<LiveMux>, Arc<dyn VideoPacketSink>)> {
    let _ = (stats, factor); // stats se conecta abajo vía el handler del core
    let sys_native = audio::probe_format(&audio::TrackKind::SystemLoopback);
    let mic_native = if mic && !mic_device.is_empty() {
        audio::probe_format(&audio::TrackKind::Microphone(mic_device.clone()))
    } else {
        None
    };
    let sys_target = sys_native.and_then(|(r, c)| audio::aac_target_format(r, c));
    let mic_target = mic_native.and_then(|(r, c)| audio::aac_target_format(r, c));

    let video_base = Arc::new(AtomicI64::new(i64::MIN));
    let core = build_pipeline_core(
        item, fps, factor, resolution, bitrate_override, encoder_pref, window_mode,
        video_base.clone(),
    )?;
    let PipelineCore { mut pipe, device, out_w, out_h, bitrate } = core;

    let out_path = format!("{out_dir}\\{}", clip_filename());
    let mux = LiveMux::new(
        out_path, out_w, out_h, fps, bitrate,
        sys_target, mic_target,
    );

    let mut audio_tracks = Vec::new();
    if let (Some((rate, ch)), Some(_)) = (sys_native, sys_target) {
        let sink = Arc::new(MuxAudioSink::new(mux.clone(), AudioRole::Sys));
        audio_tracks.push(audio::spawn_track(
            audio::TrackKind::SystemLoopback,
            audio::Encoding::Aac(aac_bitrate(sys_target.unwrap().1)),
            rate, ch, sink, None,
        ));
    }
    if let (Some((rate, ch)), Some(_)) = (mic_native, mic_target) {
        let sink = Arc::new(MuxAudioSink::new(mux.clone(), AudioRole::Mic));
        audio_tracks.push(audio::spawn_track(
            audio::TrackKind::Microphone(mic_device.clone()),
            audio::Encoding::Aac(aac_bitrate(mic_target.unwrap().1)),
            rate, ch, sink, None,
        ));
    }
    pipe.audio_tracks = audio_tracks;
    let _ = device;

    let video_sink: Arc<dyn VideoPacketSink> = mux.clone();
    Ok((pipe, mux, video_sink))
}
```

**Note (stats width/height):** the `FrameArrived` handler inside `build_pipeline_core` already updates `stats.frames`; ensure it also stores `stats.width/height` (as the old manual handler did at `capture.rs:518-519`) so `status()` keeps reporting size. If `build_pipeline_core` didn't carry `stats`, add `stats: &Arc<Stats>` to its params and to the handler.

- [ ] **Step 4: Delete the dead old path.** Remove: the `Encoder` struct and its entire `impl` (the `SinkWriter` builder `Encoder::new`, `note_frame`, `emit_paced`, `latest_slot`, `push_audio`, `finalize`, and the VBR `GetTransformForStream`/`SetOutputType` loop), `add_aac_stream` (`capture.rs:2084`), `EncoderAudioSink` (`capture.rs:2113`), and the old `build_engine`/`Engine`/`Engine::shutdown`/`finalize_encoder`/`Drop for Engine`. Keep `set_quality_codec_settings`/`log_encoder_quality` (still used by `configure_encoder_types`).

- [ ] **Step 5: Build.**

Run (from `src-tauri`): `cargo check`
Expected: compiles with no errors and no `unused` warnings for the deleted items (if a warning names a now-orphan helper only the old path used, delete that helper too).

- [ ] **Step 6: Test.**

Run (from `src-tauri`): `cargo test`
Expected: PASS (all prior tests + the 3 `livemux` tests).

- [ ] **Step 7: Commit.**

```bash
git add src-tauri/src/capture.rs
git commit -m "Capture: run manual recording on the shared pipeline via LiveMux; drop the SinkWriter encoder"
```

---

## Task 7: Revert the interim VBR workaround (superseded)

An earlier commit removed the manual `SetOutputType` VBR re-apply as a stopgap. Task 6 deletes that whole block, so no action is needed if Task 6 landed on top of it. Confirm the interim change is gone.

- [ ] **Step 1:** Confirm the manual `SetOutputType` re-apply and its comment no longer exist.

Run (from repo root): `git grep -n "no re-aplicar SetOutputType" src-tauri/src/capture.rs || echo "clean"`
Expected: `clean` (the block was deleted with the old `Encoder`).

- [ ] **Step 2:** If any orphan remains, remove it and commit:

```bash
git add src-tauri/src/capture.rs
git commit -m "Capture: drop the obsolete manual VBR workaround"
```

---

## Task 8: Manual verification (user smoke test)

Not automatable (requires GPU/WGC). The implementer hands this to the user.

- [ ] **Step 1:** `pnpm tauri dev`, record a manual clip of a monitor with system audio, then with mic enabled.
- [ ] **Step 2:** Confirm the produced MP4 has video, system audio, and (when enabled) a second mic audio track, and that quality/VBR matches an Instant Replay clip. Confirm no `MF_E_TRANSFORM_TYPE_NOT_SET` in logs.
- [ ] **Step 3:** Confirm Instant Replay still saves clips correctly (regression).

---

## Self-review notes

- **Spec coverage:** VideoPacketSink trait (Task 1) ✓; LiveMux + handshake + timeout + rebasing + faststart/CleanPoint reuse (Task 3) ✓; audio via build_aac_encoder + MuxAudioSink (Task 4) ✓; shared pipeline extraction (Task 5) ✓; manual rewire + delete old SinkWriter path/VBR hack/note_frame (Tasks 6–7) ✓; tests: LiveMux unit tests + replay regression + user smoke (Tasks 3, 6, 8) ✓; out-of-scope resize-rebuild not added ✓.
- **Type consistency:** `VideoPacketSink::{set_seq_header, push_video}` used identically in Tasks 1 and 3; `AudioRole::{Sys,Mic}` reused; `AudioMuxTrack` fields match `capture.rs:965`; `LiveMux::new` final signature includes `width,height` per the Task-3 implementation note and is called that way in Task 6.
- **Known iteration points (unsafe MF):** exact `windows`-crate import paths for symbols already used elsewhere in the file (`MFCreateFile`, `MF_MPEG4SINK_MOOV_BEFORE_MDAT`, `MFSampleExtension_CleanPoint`, `MFTranscodeContainerType_MPEG4`) are in scope via `use windows::Win32::Media::MediaFoundation::*;`. The `build_pipeline_core` extraction (Task 5) is mechanical but large; verify replay tests after it.
