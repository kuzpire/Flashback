use serde::{Deserialize, Serialize};

#[derive(Serialize, Clone, Default)]
pub struct ClipAudio {
    pub system: Option<String>,
    pub mic: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MixerState {
    pub sys_vol: f32,
    pub sys_muted: bool,
    pub mic_vol: f32,
    pub mic_muted: bool,
}

impl Default for MixerState {
    fn default() -> Self {
        Self {
            sys_vol: 1.0,
            sys_muted: false,
            mic_vol: 1.0,
            mic_muted: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Segment {
    pub start_ms: f64,
    pub end_ms: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ClipEdit {
    pub segments: Vec<Segment>,
    pub mixer: MixerState,
}

#[cfg(target_os = "windows")]
pub use win::{clip_fps, export_clip, keyframe_times, load_edit, prepare_clip_audio, save_edit};

#[cfg(not(target_os = "windows"))]
pub fn prepare_clip_audio(_path: String) -> Result<ClipAudio, String> {
    Err("El editor solo está disponible en Windows".into())
}

#[cfg(not(target_os = "windows"))]
pub fn load_edit(_path: String) -> Result<ClipEdit, String> {
    Err("El editor solo está disponible en Windows".into())
}

#[cfg(not(target_os = "windows"))]
pub fn save_edit(_path: String, _edit: ClipEdit) -> Result<(), String> {
    Err("El editor solo está disponible en Windows".into())
}

#[cfg(not(target_os = "windows"))]
pub fn keyframe_times(_path: String) -> Result<Vec<f64>, String> {
    Err("El editor solo está disponible en Windows".into())
}

#[cfg(not(target_os = "windows"))]
pub fn clip_fps(_path: String) -> Result<u32, String> {
    Err("El editor solo está disponible en Windows".into())
}

#[cfg(not(target_os = "windows"))]
pub fn export_clip(_src: String, _dst: String, _edit: ClipEdit) -> Result<(), String> {
    Err("El editor solo está disponible en Windows".into())
}

#[cfg(target_os = "windows")]
mod win {
    use std::sync::Once;

    use windows::core::{Result, GUID, HSTRING};
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};

    use super::{ClipAudio, ClipEdit};

    const ALL_STREAMS: u32 = 0xFFFF_FFFE;
    const ENDOFSTREAM: u32 = 0x0000_0002;

    static MF_INIT: Once = Once::new();
    fn ensure_mf() {
        MF_INIT.call_once(|| unsafe {
            let _ = MFStartup(MF_VERSION, MFSTARTUP_FULL);
        });
    }

    // Ejecuta una operación de Media Foundation en su propio hilo con COM (MTA) y MF
    // inicializados. Los SourceReader/SinkWriter exigen ese contexto; sin él, fallan en
    // silencio en el hilo de comandos de Tauri.
    fn with_mf<T, F>(f: F) -> std::result::Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce() -> std::result::Result<T, String> + Send + 'static,
    {
        std::thread::spawn(move || {
            unsafe { let _ = CoInitializeEx(None, COINIT_MULTITHREADED); }
            ensure_mf();
            let r = f();
            unsafe { CoUninitialize(); }
            r
        })
        .join()
        .map_err(|_| "El hilo de Media Foundation terminó inesperadamente".to_string())?
    }

    pub fn prepare_clip_audio(path: String) -> std::result::Result<ClipAudio, String> {
        std::thread::spawn(move || {
            unsafe { let _ = CoInitializeEx(None, COINIT_MULTITHREADED); }
            ensure_mf();
            let r = extract(&path);
            unsafe { CoUninitialize(); }
            r
        })
        .join()
        .map_err(|_| "El hilo de extracción de audio terminó inesperadamente".to_string())?
    }

    fn extract(path: &str) -> std::result::Result<ClipAudio, String> {
        let mf = |e| format!("{e:?}");
        let io = |e: std::io::Error| e.to_string();

        let audio_streams = count_audio_streams(path).map_err(mf)?;
        if audio_streams < 3 {
            return Ok(ClipAudio::default());
        }

        let key = temp_key(path);
        let dir = std::env::temp_dir();
        let sys = dir
            .join(format!("flashback_edit_{key}_sys.wav"))
            .to_string_lossy()
            .into_owned();
        let mic = dir
            .join(format!("flashback_edit_{key}_mic.wav"))
            .to_string_lossy()
            .into_owned();

        // Los clips son inmutables (edición no destructiva): si ya se separaron las pistas en
        // una apertura anterior, se reutilizan en vez de volver a volcar cientos de MB de WAV.
        let ready = |p: &str| std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false);
        if !(ready(&sys) && ready(&mic)) {
            let (sys_pcm, sr, sc) = read_pcm(path, 1).map_err(mf)?;
            write_wav(&sys, &sys_pcm, sr, sc).map_err(io)?;
            let (mic_pcm, mr, mc) = read_pcm(path, 2).map_err(mf)?;
            write_wav(&mic, &mic_pcm, mr, mc).map_err(io)?;
        }

        Ok(ClipAudio {
            system: Some(sys),
            mic: Some(mic),
        })
    }

    // Clave estable por ruta completa: evita colisiones entre clips con el mismo nombre de
    // archivo en carpetas distintas (p. ej. el original y su `_edit.mp4`).
    fn temp_key(path: &str) -> String {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        path.hash(&mut h);
        format!("{:016x}", h.finish())
    }

    fn open_reader(path: &str) -> Result<IMFSourceReader> {
        let url = HSTRING::from(path);
        unsafe { MFCreateSourceReaderFromURL(&url, None) }
    }

    fn count_audio_streams(path: &str) -> Result<usize> {
        let reader = open_reader(path)?;
        let mut count = 0usize;
        let mut i = 0u32;
        while let Ok(mt) = unsafe { reader.GetNativeMediaType(i, 0) } {
            if unsafe { mt.GetGUID(&MF_MT_MAJOR_TYPE) }
                .map(|g| g == MFMediaType_Audio)
                .unwrap_or(false)
            {
                count += 1;
            }
            i += 1;
        }
        Ok(count)
    }

    fn read_pcm(path: &str, ordinal: usize) -> Result<(Vec<u8>, u32, u16)> {
        let reader = open_reader(path)?;
        unsafe { reader.SetStreamSelection(ALL_STREAMS, false)? };

        let mut target: Option<u32> = None;
        let mut seen = 0usize;
        let mut i = 0u32;
        while let Ok(mt) = unsafe { reader.GetNativeMediaType(i, 0) } {
            let major = unsafe { mt.GetGUID(&MF_MT_MAJOR_TYPE) }
                .unwrap_or(GUID::zeroed());
            if major == MFMediaType_Audio {
                if seen == ordinal {
                    target = Some(i);
                    break;
                }
                seen += 1;
            }
            i += 1;
        }

        let idx = target.ok_or_else(|| {
            windows::core::Error::from(windows::core::HRESULT(0x80070002u32 as i32))
        })?;

        unsafe { reader.SetStreamSelection(idx, true)? };

        let pcm_type = unsafe { MFCreateMediaType()? };
        unsafe { pcm_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)? };
        unsafe { pcm_type.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)? };
        unsafe { reader.SetCurrentMediaType(idx, None, &pcm_type)? };

        let actual = unsafe { reader.GetCurrentMediaType(idx)? };
        let sr = unsafe { actual.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND) }.unwrap_or(48000);
        let ch = unsafe { actual.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS) }.unwrap_or(2) as u16;

        let mut pcm = Vec::new();
        loop {
            let mut flags = 0u32;
            let mut sample: Option<IMFSample> = None;
            unsafe { reader.ReadSample(idx, 0, None, Some(&mut flags), None, Some(&mut sample))? };
            if flags & ENDOFSTREAM != 0 { break; }
            let Some(sample) = sample else { continue };
            let buf = unsafe { sample.ConvertToContiguousBuffer()? };
            let mut ptr: *mut u8 = std::ptr::null_mut();
            let mut cur = 0u32;
            unsafe { buf.Lock(&mut ptr, None, Some(&mut cur))? };
            if cur > 0 {
                let slice = unsafe { std::slice::from_raw_parts(ptr, cur as usize) };
                pcm.extend_from_slice(slice);
            }
            unsafe { buf.Unlock()? };
        }

        Ok((pcm, sr, ch))
    }

    fn write_wav(path: &str, pcm: &[u8], sample_rate: u32, channels: u16) -> std::io::Result<()> {
        use std::io::Write;
        let bits = 16u16;
        let byte_rate = sample_rate * channels as u32 * (bits as u32 / 8);
        let block_align = channels * (bits / 8);
        let data_len = pcm.len() as u32;
        let file_len = 36 + data_len;

        let mut f = std::fs::File::create(path)?;
        f.write_all(b"RIFF")?;
        f.write_all(&file_len.to_le_bytes())?;
        f.write_all(b"WAVE")?;
        f.write_all(b"fmt ")?;
        f.write_all(&16u32.to_le_bytes())?;
        f.write_all(&1u16.to_le_bytes())?;
        f.write_all(&channels.to_le_bytes())?;
        f.write_all(&sample_rate.to_le_bytes())?;
        f.write_all(&byte_rate.to_le_bytes())?;
        f.write_all(&block_align.to_le_bytes())?;
        f.write_all(&bits.to_le_bytes())?;
        f.write_all(b"data")?;
        f.write_all(&data_len.to_le_bytes())?;
        f.write_all(pcm)?;
        Ok(())
    }

    // Edición no destructiva: los cortes y la mezcla se guardan en un sidecar JSON junto al
    // clip (mismo criterio que el `.clip.json` de la biblioteca). El MP4 original no se toca.
    fn edit_sidecar(path: &str) -> std::path::PathBuf {
        std::path::Path::new(path).with_extension("edit.json")
    }

    pub fn save_edit(path: String, edit: ClipEdit) -> std::result::Result<(), String> {
        let json = serde_json::to_string(&edit).map_err(|e| e.to_string())?;
        std::fs::write(edit_sidecar(&path), json).map_err(|e| e.to_string())
    }

    pub fn load_edit(path: String) -> std::result::Result<ClipEdit, String> {
        match std::fs::read_to_string(edit_sidecar(&path)) {
            Ok(s) => serde_json::from_str::<ClipEdit>(&s).map_err(|e| e.to_string()),
            Err(_) => Ok(ClipEdit {
                segments: Vec::new(),
                mixer: super::MixerState::default(),
            }),
        }
    }

    pub fn keyframe_times(path: String) -> std::result::Result<Vec<f64>, String> {
        with_mf(move || keyframe_times_inner(&path))
    }

    pub fn clip_fps(path: String) -> std::result::Result<u32, String> {
        with_mf(move || read_video_meta(&path).map(|m| m.fps).map_err(|e| format!("{e:?}")))
    }

    fn keyframe_times_inner(path: &str) -> std::result::Result<Vec<f64>, String> {
        let mf = |e: windows::core::Error| format!("{e:?}");
        let reader = open_reader(path).map_err(mf)?;
        let idx = find_stream(&reader, MFMediaType_Video).map_err(mf)?;
        unsafe { reader.SetStreamSelection(ALL_STREAMS, false) }.map_err(mf)?;
        unsafe { reader.SetStreamSelection(idx, true) }.map_err(mf)?;

        let mut times = Vec::new();
        loop {
            let mut flags = 0u32;
            let mut sample: Option<IMFSample> = None;
            unsafe { reader.ReadSample(idx, 0, None, Some(&mut flags), None, Some(&mut sample)) }
                .map_err(mf)?;
            if flags & ENDOFSTREAM != 0 { break; }
            let Some(sample) = sample else { continue };
            let is_sync = unsafe { sample.GetUINT32(&MFSampleExtension_CleanPoint) }.unwrap_or(0) != 0;
            if is_sync {
                let t = unsafe { sample.GetSampleTime() }.unwrap_or(0);
                times.push(t as f64 / 10_000.0);
            }
        }
        Ok(times)
    }

    pub fn export_clip(src: String, dst: String, edit: ClipEdit) -> std::result::Result<(), String> {
        std::thread::spawn(move || {
            unsafe { let _ = CoInitializeEx(None, COINIT_MULTITHREADED); }
            ensure_mf();
            let r = do_export(&src, &dst, &edit);
            unsafe { CoUninitialize(); }
            r
        })
        .join()
        .map_err(|_| "El hilo de exportación terminó inesperadamente".to_string())?
    }

    #[derive(Clone)]
    struct VideoMeta {
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
    }

    fn extract_video_range(src: &str, start_ms: f64, end_ms: f64, meta: &VideoMeta, keyframes: &[f64]) -> std::result::Result<Vec<H264Packet>, String> {
        let mf = |e: windows::core::Error| format!("{e:?}");

        let in_key_ms = keyframes.iter()
            .rev()
            .find(|k| **k <= start_ms)
            .copied()
            .unwrap_or(0.0);

        let frame_accurate = (start_ms - in_key_ms).abs() >= 1.0;
        let packets = if frame_accurate {
            let next_kf = keyframes.iter()
                .find(|k| **k > start_ms)
                .copied()
                .unwrap_or(end_ms);
            let boundary = reencode_boundary_gop(src, start_ms, in_key_ms, next_kf, &meta)
                .map_err(mf)?;
            if boundary.is_empty() {
                return Err("El recorte del borde no produjo fotogramas".into());
            }
            let bd = boundary.last().map(|(_, t, d, _)| t + d).unwrap_or(0);

            let rest = if next_kf < end_ms {
                let mut p = extract_h264_packets(src, next_kf, end_ms).map_err(mf)?;
                for (_, t, _, _) in &mut p { *t += bd; }
                p
            } else {
                Vec::new()
            };

            let mut all = boundary;
            all.extend(rest);
            all
        } else {
            extract_h264_packets(src, in_key_ms, end_ms).map_err(mf)?
        };
        Ok(packets)
    }

    fn read_video_meta(path: &str) -> Result<VideoMeta> {
        let reader = open_reader(path)?;
        let v_idx = find_stream(&reader, MFMediaType_Video)?;
        let mt = unsafe { reader.GetCurrentMediaType(v_idx)? };
        let size = unsafe { mt.GetUINT64(&MF_MT_FRAME_SIZE) }.unwrap_or(pack2(0, 0));
        let w = (size >> 32) as u32;
        let h = (size & 0xFFFFFFFF) as u32;
        let fps_packed = unsafe { mt.GetUINT64(&MF_MT_FRAME_RATE) }.unwrap_or(pack2(30, 1));
        let fps_n = (fps_packed >> 32) as u32;
        let fps_d = (fps_packed & 0xFFFFFFFF) as u32;
        let fps = if fps_d == 0 { 30 } else { fps_n / fps_d };
        let bitrate = unsafe { mt.GetUINT32(&MF_MT_AVG_BITRATE) }.unwrap_or(0);

        Ok(VideoMeta {
            width: w,
            height: h,
            fps: fps.max(1),
            bitrate: bitrate.max(1_000_000),
        })
    }

    fn find_stream(reader: &IMFSourceReader, kind: GUID) -> Result<u32> {
        let mut i = 0u32;
        while let Ok(mt) = unsafe { reader.GetNativeMediaType(i, 0) } {
            let major = unsafe { mt.GetGUID(&MF_MT_MAJOR_TYPE) }
                .unwrap_or(GUID::zeroed());
            if major == kind {
                return Ok(i);
            }
            i += 1;
        }
        Err(windows::core::Error::from(windows::core::HRESULT(0x80070002u32 as i32)))
    }

    type H264Packet = (Vec<u8>, i64, i64, bool);

    fn extract_h264_packets(path: &str, in_ms: f64, end_ms: f64) -> Result<Vec<H264Packet>> {
        let reader = open_reader(path)?;
        let idx = find_stream(&reader, MFMediaType_Video)?;
        unsafe { reader.SetStreamSelection(ALL_STREAMS, false)? };
        unsafe { reader.SetStreamSelection(idx, true)? };

        let in_hns = (in_ms * 10_000.0) as i64;
        let end_hns = (end_ms * 10_000.0) as i64;
        let mut out = Vec::new();
        let mut base: Option<i64> = None;

        loop {
            let mut flags = 0u32;
            let mut sample: Option<IMFSample> = None;
            unsafe { reader.ReadSample(idx, 0, None, Some(&mut flags), None, Some(&mut sample))? };
            if flags & ENDOFSTREAM != 0 { break; }
            let Some(sample) = sample else { continue };
            let t = unsafe { sample.GetSampleTime()? };
            if t < in_hns { continue; }
            if t >= end_hns && end_hns > 0 { break; }

            let is_sync = unsafe { sample.GetUINT32(&MFSampleExtension_CleanPoint) }.unwrap_or(0) != 0;
            let dur = unsafe { sample.GetSampleDuration() }.unwrap_or(0);
            let buf = unsafe { sample.ConvertToContiguousBuffer()? };
            let mut ptr: *mut u8 = std::ptr::null_mut();
            let mut cur = 0u32;
            unsafe { buf.Lock(&mut ptr, None, Some(&mut cur))? };
            if cur == 0 { unsafe { buf.Unlock()? }; continue; }
            let data = unsafe { std::slice::from_raw_parts(ptr, cur as usize) }.to_vec();
            unsafe { buf.Unlock()? };

            let bt = base.unwrap_or(t);
            if base.is_none() { base = Some(t); }
            out.push((data, t - bt, dur, is_sync));
        }
        Ok(out)
    }

    fn create_h264_encoder(width: u32, height: u32, fps: u32, bitrate: u32) -> Result<(IMFTransform, IMFMediaType, IMFMediaType)> {
        let encoder: IMFTransform = unsafe {
            CoCreateInstance(&CLSID_MSH264EncoderMFT, None, CLSCTX_INPROC_SERVER)?
        };

        let in_type = unsafe { MFCreateMediaType()? };
        unsafe { in_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)? };
        unsafe { in_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)? };
        unsafe { in_type.SetUINT64(&MF_MT_FRAME_SIZE, pack2(width, height))? };
        unsafe { in_type.SetUINT64(&MF_MT_FRAME_RATE, pack2(fps, 1))? };
        unsafe { in_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)? };
        unsafe { in_type.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)? };
        unsafe { encoder.SetInputType(0, &in_type, 0)? };

        let out_type = unsafe { MFCreateMediaType()? };
        unsafe { out_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)? };
        unsafe { out_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)? };
        unsafe { out_type.SetUINT64(&MF_MT_FRAME_SIZE, pack2(width, height))? };
        unsafe { out_type.SetUINT64(&MF_MT_FRAME_RATE, pack2(fps, 1))? };
        unsafe { out_type.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)? };
        unsafe { encoder.SetOutputType(0, &out_type, 0)? };

        let attrs = unsafe { encoder.GetAttributes()? };
        unsafe { attrs.SetUINT32(&CODECAPI_AVEncCommonRateControlMode, 0)? };
        unsafe { attrs.SetUINT32(&CODECAPI_AVEncCommonQuality, 100)? };
        unsafe { encoder.GetOutputStreamInfo(0)? };

        unsafe { encoder.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)? };
        unsafe { encoder.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)? };

        Ok((encoder, in_type, out_type))
    }

    fn reencode_boundary_gop(
        src: &str,
        edit_in_ms: f64,
        in_key_ms: f64,
        next_kf_ms: f64,
        meta: &VideoMeta,
    ) -> Result<Vec<H264Packet>> {
        let reader = open_reader(src)?;
        let v_idx = find_stream(&reader, MFMediaType_Video)?;
        unsafe { reader.SetStreamSelection(ALL_STREAMS, false)? };
        unsafe { reader.SetStreamSelection(v_idx, true)? };

        let nv12 = unsafe { MFCreateMediaType()? };
        unsafe { nv12.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)? };
        unsafe { nv12.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)? };
        unsafe { reader.SetCurrentMediaType(v_idx, None, &nv12)? };

        let (encoder, _enc_in_type, _enc_out_type) =
            create_h264_encoder(meta.width, meta.height, meta.fps, meta.bitrate)?;

        let edit_hns = (edit_in_ms * 10_000.0) as i64;
        let first_hns = (in_key_ms * 10_000.0) as i64;
        let end_hns = (next_kf_ms * 10_000.0) as i64;

        let mut output = Vec::new();
        let mut samples = Vec::new();
        let mut first_ts: Option<i64> = None;

        loop {
            let mut flags = 0u32;
            let mut sample: Option<IMFSample> = None;
            unsafe { reader.ReadSample(v_idx, 0, None, Some(&mut flags), None, Some(&mut sample))? };
            if flags & ENDOFSTREAM != 0 { break; }
            let Some(sample) = sample else { continue };
            let t = unsafe { sample.GetSampleTime()? };
            if t < first_hns { continue; }
            if t >= end_hns && end_hns > first_hns { break; }

            if t >= edit_hns {
                if first_ts.is_none() {
                    unsafe { sample.SetSampleTime(0)? };
                    first_ts = Some(0);
                } else {
                    unsafe { sample.SetSampleTime(t - edit_hns)? };
                }
                samples.push(sample);
            }
        }

        for sample in &samples {
            unsafe { encoder.ProcessInput(0, sample, 0)? };

            loop {
                let mut out = MFT_OUTPUT_DATA_BUFFER::default();
                let mut status = 0u32;
                match unsafe { encoder.ProcessOutput(0, std::slice::from_mut(&mut out), &mut status) } {
                    Ok(()) => {
                        if let Some(out_s) = &*out.pSample {
                            let t = unsafe { out_s.GetSampleTime() }.unwrap_or(0);
                            let dur = unsafe { out_s.GetSampleDuration() }.unwrap_or(0);
                            let buf = unsafe { out_s.ConvertToContiguousBuffer() }?;
                            let mut ptr: *mut u8 = std::ptr::null_mut();
                            let mut cur = 0u32;
                            unsafe { buf.Lock(&mut ptr, None, Some(&mut cur))? };
                            if cur > 0 {
                                let data = unsafe { std::slice::from_raw_parts(ptr, cur as usize) }.to_vec();
                                unsafe { buf.Unlock()? };
                                let is_sync = unsafe { out_s.GetUINT32(&MFSampleExtension_CleanPoint) }.unwrap_or(0) != 0;
                                output.push((data, t, dur, is_sync));
                            } else {
                                unsafe { buf.Unlock()? };
                            }
                        }
                    }
                    Err(e) if e.code() == windows::core::HRESULT(MF_E_TRANSFORM_NEED_MORE_INPUT.0) => break,
                    Err(e) => return Err(e),
                }
            }
        }

        unsafe { encoder.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0)? };
        unsafe { encoder.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0)? };

        loop {
            let mut out = MFT_OUTPUT_DATA_BUFFER::default();
            let mut status = 0u32;
            match unsafe { encoder.ProcessOutput(0, std::slice::from_mut(&mut out), &mut status) } {
                Ok(()) => {
                    if let Some(out_s) = &*out.pSample {
                        let t = unsafe { out_s.GetSampleTime() }.unwrap_or(0);
                        let dur = unsafe { out_s.GetSampleDuration() }.unwrap_or(0);
                        let buf = unsafe { out_s.ConvertToContiguousBuffer() }?;
                        let mut ptr: *mut u8 = std::ptr::null_mut();
                        let mut cur = 0u32;
                        unsafe { buf.Lock(&mut ptr, None, Some(&mut cur))? };
                        if cur > 0 {
                            let data = unsafe { std::slice::from_raw_parts(ptr, cur as usize) }.to_vec();
                            unsafe { buf.Unlock()? };
                            let is_sync = unsafe { out_s.GetUINT32(&MFSampleExtension_CleanPoint) }.unwrap_or(0) != 0;
                            output.push((data, t, dur, is_sync));
                        } else {
                            unsafe { buf.Unlock()? };
                        }
                    }
                }
                Err(e) if e.code() == windows::core::HRESULT(MF_E_TRANSFORM_NEED_MORE_INPUT.0) => break,
                Err(e) => return Err(e),
            }
        }

        Ok(output)
    }

    fn do_export(src: &str, dst: &str, edit: &ClipEdit) -> std::result::Result<(), String> {
        let mf = |e: windows::core::Error| format!("{e:?}");

        let meta = read_video_meta(src).map_err(mf)?;
        let keyframes = keyframe_times_inner(src)?;

        if edit.segments.is_empty() {
            return Err("No hay segmentos para exportar".into());
        }

        let mut all_video = Vec::new();
        let mut offset = 0i64;

        for seg in &edit.segments {
            let mut packets = extract_video_range(src, seg.start_ms, seg.end_ms, &meta, &keyframes)?;
            if packets.is_empty() {
                continue;
            }
            for (_, t, _, _) in &mut packets {
                *t += offset;
            }
            if let Some((_, last_t, last_d, _)) = packets.last() {
                offset = last_t + last_d;
            }
            all_video.extend(packets);
        }

        if all_video.is_empty() {
            return Err("No hay paquetes de vídeo en los segmentos seleccionados".into());
        }

        mux_mp4(dst, src, &all_video, edit, &meta)
            .map_err(|e| format!("Error al multiplexar: {e:?}"))
    }

    fn extract_param_sets(data: &[u8]) -> Vec<u8> {
        let mut starts: Vec<usize> = Vec::new();
        let mut i = 0usize;
        while i + 3 <= data.len() {
            if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
                starts.push(i);
                i += 3;
            } else {
                i += 1;
            }
        }
        let mut out = Vec::new();
        for (idx, &s) in starts.iter().enumerate() {
            let header_pos = s + 3;
            if header_pos >= data.len() { break; }
            let sc_start = if s > 0 && data[s - 1] == 0 { s - 1 } else { s };
            let end = match starts.get(idx + 1) {
                Some(&ns) if ns > 0 && data[ns - 1] == 0 => ns - 1,
                Some(&ns) => ns,
                None => data.len(),
            };
            let nal_type = data[header_pos] & 0x1F;
            if nal_type == 7 || nal_type == 8 {
                out.extend_from_slice(&data[sc_start..end]);
            }
        }
        out
    }

    fn blob(mt: &IMFAttributes, key: &GUID) -> Option<Vec<u8>> {
        unsafe {
            let size = mt.GetBlobSize(key).ok()?;
            if size == 0 { return None; }
            let mut v = vec![0u8; size as usize];
            mt.GetBlob(key, &mut v, None).ok()?;
            Some(v)
        }
    }

    fn mux_mp4(
        dst: &str,
        src: &str,
        video: &[H264Packet],
        edit: &ClipEdit,
        meta: &VideoMeta,
    ) -> Result<()> {
        let src_reader = open_reader(src)?;

        let v_idx = find_stream(&src_reader, MFMediaType_Video)?;
        let v_cur = unsafe { src_reader.GetCurrentMediaType(v_idx)? };
        let seq_header = blob(&v_cur, &MF_MT_MPEG_SEQUENCE_HEADER)
            .or_else(|| video.first().map(|(data, _, _, _)| extract_param_sets(data)))
            .unwrap_or_default();

        // Audio: si el mezclador está en sus valores por defecto, se copia tal cual la
        // pista 0 (mezcla) sin recodificar. Si el usuario tocó volúmenes o silenció una
        // fuente, se rehornea desde sistema (pista 1) y micro (pista 2) a una sola pista AAC.
        let remix = !mixer_is_default(&edit.mixer)
            && count_audio_streams(src).map(|n| n >= 3).unwrap_or(false);
        let remixed = if remix { Some(build_remixed_pcm(src, edit)?) } else { None };
        let a_idx = if remix { None } else { find_stream(&src_reader, MFMediaType_Audio).ok() };

        let dst_url = HSTRING::from(dst);
        let sink: IMFSinkWriter = unsafe { MFCreateSinkWriterFromURL(&dst_url, None, None)? };

        let v_type = unsafe { MFCreateMediaType()? };
        unsafe { v_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)? };
        unsafe { v_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)? };
        unsafe { v_type.SetUINT64(&MF_MT_FRAME_SIZE, pack2(meta.width, meta.height))? };
        unsafe { v_type.SetUINT64(&MF_MT_FRAME_RATE, pack2(meta.fps, 1))? };
        unsafe { v_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)? };
        unsafe { v_type.SetUINT32(&MF_MT_AVG_BITRATE, meta.bitrate)? };
        if !seq_header.is_empty() {
            unsafe { v_type.SetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &seq_header)? };
        }
        let v_stream = unsafe { sink.AddStream(&v_type)? };
        unsafe { sink.SetInputMediaType(v_stream, &v_type, None)? };

        let mut pass_stream = None;
        let mut remix_stream = None;
        if let Some((_, rate)) = &remixed {
            remix_stream = Some(add_remix_audio_stream(&sink, *rate)?);
        } else if let Some(a_idx) = a_idx {
            let aac_out = unsafe { MFCreateMediaType()? };
            unsafe {
                aac_out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
                aac_out.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
                src_reader.SetCurrentMediaType(a_idx, None, &aac_out)?;
            }
            let a_native = unsafe { src_reader.GetCurrentMediaType(a_idx)? };
            let s = unsafe { sink.AddStream(&a_native)? };
            unsafe { sink.SetInputMediaType(s, &a_native, None)? };
            pass_stream = Some((a_idx, s));
        }

        unsafe { sink.BeginWriting()? };

        for (data, ts_hns, dur_hns, is_sync) in video {
            let sample = unsafe { MFCreateSample()? };
            let buf = unsafe { MFCreateMemoryBuffer(data.len() as u32)? };
            let mut ptr: *mut u8 = std::ptr::null_mut();
            unsafe { buf.Lock(&mut ptr, None, None)? };
            unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len()) };
            unsafe { buf.Unlock()? };
            unsafe { buf.SetCurrentLength(data.len() as u32)? };
            unsafe { sample.AddBuffer(&buf)? };
            unsafe { sample.SetSampleTime(*ts_hns)? };
            unsafe { sample.SetSampleDuration(*dur_hns)? };
            if *is_sync {
                unsafe { sample.SetUINT32(&MFSampleExtension_CleanPoint, 1)? };
            }
            unsafe { sink.WriteSample(v_stream, &sample)? };
        }

        if let Some(a_stream) = remix_stream {
            let (mixed, rate) = remixed.as_ref().unwrap();
            write_remixed_audio(&sink, a_stream, mixed, *rate, edit)?;
        } else if let Some((a_idx, a_stream)) = pass_stream {
            unsafe { src_reader.SetStreamSelection(ALL_STREAMS, false)? };
            unsafe { src_reader.SetStreamSelection(a_idx, true)? };

            let seg_ranges: Vec<(i64, i64, i64)> = {
                let mut kept_before = 0i64;
                edit.segments.iter().map(|seg| {
                    let start_hns = (seg.start_ms * 10_000.0) as i64;
                    let end_hns = (seg.end_ms * 10_000.0) as i64;
                    let offset = start_hns - kept_before;
                    kept_before += end_hns - start_hns;
                    (start_hns, end_hns, offset)
                }).collect()
            };

            let mut seg_idx = 0usize;

            loop {
                let mut flags = 0u32;
                let mut sample: Option<IMFSample> = None;
                unsafe { src_reader.ReadSample(a_idx, 0, None, Some(&mut flags), None, Some(&mut sample))? };
                if flags & ENDOFSTREAM != 0 { break; }
                let Some(sample) = sample else { continue };
                let t = unsafe { sample.GetSampleTime()? };

                while seg_idx < seg_ranges.len() && t >= seg_ranges[seg_idx].1 {
                    seg_idx += 1;
                }
                if seg_idx >= seg_ranges.len() { break; }

                let (start_hns, _end_hns, offset) = seg_ranges[seg_idx];
                if t >= start_hns {
                    unsafe { sample.SetSampleTime(t - offset)? };
                    unsafe { sink.WriteSample(a_stream, &sample)? };
                }
            }
        }

        unsafe { sink.Finalize()? };
        Ok(())
    }

    fn pack2(hi: u32, lo: u32) -> u64 {
        (hi as u64) << 32 | lo as u64
    }

    fn mixer_is_default(m: &super::MixerState) -> bool {
        !m.sys_muted
            && !m.mic_muted
            && (m.sys_vol - 1.0).abs() < 1e-3
            && (m.mic_vol - 1.0).abs() < 1e-3
    }

    // Genera la pista de mezcla final aplicando los volúmenes/silencios del editor sobre las
    // pistas de sistema (1) y micro (2). Devuelve PCM16 estéreo entrelazado al rate elegido.
    // El AAC de Media Foundation solo admite 44100/48000 Hz; si el origen es otro, se remuestrea.
    fn build_remixed_pcm(src: &str, edit: &ClipEdit) -> Result<(Vec<i16>, u32)> {
        let (sys_raw, sr, sc) = read_pcm(src, 1)?;
        let (mic_raw, mr, mc) = read_pcm(src, 2)?;

        let out_rate = if sr == 44100 || sr == 48000 {
            sr
        } else if mr == 44100 || mr == 48000 {
            mr
        } else {
            48000
        };

        let sys = to_stereo_f32(&sys_raw, sr, sc, out_rate);
        let mic = to_stereo_f32(&mic_raw, mr, mc, out_rate);

        let sys_gain = if edit.mixer.sys_muted { 0.0 } else { edit.mixer.sys_vol.max(0.0) };
        let mic_gain = if edit.mixer.mic_muted { 0.0 } else { edit.mixer.mic_vol.max(0.0) };

        let n = sys.len().max(mic.len());
        let mut mixed = vec![0i16; n];
        for i in 0..n {
            let s = sys.get(i).copied().unwrap_or(0.0) * sys_gain;
            let m = mic.get(i).copied().unwrap_or(0.0) * mic_gain;
            mixed[i] = soft_clip_sample(s + m);
        }
        Ok((mixed, out_rate))
    }

    // PCM16 entrelazado de `src_ch` canales a estéreo f32 al `out_rate`. Downmix multicanal
    // (frontales íntegros, central/surround a 0.707) y remuestreo lineal cuando los rates
    // difieren. Mismo criterio que el mezclador en vivo (audio.rs), pero offline.
    fn to_stereo_f32(pcm: &[u8], src_rate: u32, src_ch: u16, out_rate: u32) -> Vec<f32> {
        let src_ch = src_ch.max(1) as usize;
        let in_frames = pcm.len() / (src_ch * 2);
        if in_frames == 0 {
            return Vec::new();
        }
        let rd = |frame: usize, ch: usize| -> f32 {
            let idx = (frame * src_ch + ch) * 2;
            i16::from_le_bytes([pcm[idx], pcm[idx + 1]]) as f32
        };
        let lr = |frame: usize| -> (f32, f32) {
            if src_ch == 1 {
                let m = rd(frame, 0);
                (m, m)
            } else {
                let mut l = rd(frame, 0);
                let mut r = rd(frame, 1);
                if src_ch >= 3 {
                    let c = 0.707 * rd(frame, 2);
                    l += c;
                    r += c;
                }
                let mut i = 4;
                while i < src_ch {
                    let s = 0.707 * rd(frame, i);
                    if (i - 4) % 2 == 0 {
                        l += s;
                    } else {
                        r += s;
                    }
                    i += 1;
                }
                (l, r)
            }
        };

        let src_rate_i = src_rate.max(1) as i64;
        let out_rate_i = out_rate.max(1) as i64;
        let same = src_rate_i == out_rate_i;
        let out_frames = if same {
            in_frames
        } else {
            ((in_frames as i64 * out_rate_i) / src_rate_i).max(0) as usize
        };

        let mut out = Vec::with_capacity(out_frames * 2);
        for k in 0..out_frames {
            let (l, r) = if same {
                lr(k.min(in_frames - 1))
            } else {
                let pos = k as f64 * src_rate_i as f64 / out_rate_i as f64;
                let i0 = (pos.floor() as usize).min(in_frames - 1);
                let i1 = (i0 + 1).min(in_frames - 1);
                let frac = (pos - pos.floor()) as f32;
                let (l0, r0) = lr(i0);
                let (l1, r1) = lr(i1);
                (l0 + (l1 - l0) * frac, r0 + (r1 - r0) * frac)
            };
            out.push(l);
            out.push(r);
        }
        out
    }

    // Soft clip: lineal por debajo del umbral, compresión suave por encima. Evita la
    // distorsión áspera del recorte duro al sumar dos fuentes a tope.
    fn soft_clip_sample(x: f32) -> i16 {
        const T: f32 = 0.75;
        let n = x / 32768.0;
        let a = n.abs();
        let y = if a <= T {
            n
        } else {
            n.signum() * (T + (1.0 - T) * (1.0 - (-(a - T) / (1.0 - T)).exp()))
        };
        (y * 32767.0).clamp(-32768.0, 32767.0) as i16
    }

    fn add_remix_audio_stream(sink: &IMFSinkWriter, rate: u32) -> Result<u32> {
        let ch = 2u32;
        let out_type = unsafe { MFCreateMediaType()? };
        unsafe {
            out_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            out_type.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
            out_type.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
            out_type.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, ch)?;
            out_type.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            out_type.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 128_000 / 8)?;
            out_type.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)?;
        }
        let stream = unsafe { sink.AddStream(&out_type)? };

        let in_type = unsafe { MFCreateMediaType()? };
        unsafe {
            in_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            in_type.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
            in_type.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
            in_type.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, ch)?;
            in_type.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            in_type.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, ch * 2)?;
            in_type.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, rate * ch * 2)?;
            sink.SetInputMediaType(stream, &in_type, None)?;
        }
        Ok(stream)
    }

    // Escribe la mezcla recortada por segmentos: solo las porciones conservadas, concatenadas,
    // con timestamps secuenciales (el SinkWriter recodifica PCM→AAC). `mixed` es estéreo
    // entrelazado al `rate` dado.
    fn write_remixed_audio(
        sink: &IMFSinkWriter,
        a_stream: u32,
        mixed: &[i16],
        rate: u32,
        edit: &ClipEdit,
    ) -> Result<()> {
        let frames_total = mixed.len() / 2;
        let rate_i = rate.max(1) as i64;
        let block_frames = (rate / 50).max(1) as usize;
        let mut out_t: i64 = 0;

        for seg in &edit.segments {
            let start_f = (((seg.start_ms / 1000.0) * rate as f64).round() as i64).max(0) as usize;
            let end_f = (((seg.end_ms / 1000.0) * rate as f64).round() as i64).max(0) as usize;
            let start_f = start_f.min(frames_total);
            let end_f = end_f.min(frames_total);

            let mut f = start_f;
            while f < end_f {
                let chunk = (end_f - f).min(block_frames);
                let byte_len = chunk * 4;

                let sample = unsafe { MFCreateSample()? };
                let buf = unsafe { MFCreateMemoryBuffer(byte_len as u32)? };
                let mut ptr: *mut u8 = std::ptr::null_mut();
                unsafe { buf.Lock(&mut ptr, None, None)? };
                unsafe {
                    let dst = std::slice::from_raw_parts_mut(ptr as *mut i16, chunk * 2);
                    dst.copy_from_slice(&mixed[f * 2..f * 2 + chunk * 2]);
                }
                unsafe { buf.Unlock()? };
                unsafe { buf.SetCurrentLength(byte_len as u32)? };
                unsafe { sample.AddBuffer(&buf)? };
                unsafe { sample.SetSampleTime(out_t)? };
                let dur = chunk as i64 * 10_000_000 / rate_i;
                unsafe { sample.SetSampleDuration(dur)? };
                unsafe { sink.WriteSample(a_stream, &sample)? };

                out_t += dur;
                f += chunk;
            }
        }
        Ok(())
    }
}
