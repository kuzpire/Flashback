use super::*;

// Muxer en directo de la grabación manual: recibe paquetes ya codificados (H.264 + AAC) del
// pipeline compartido y los escribe a un MP4 en passthrough al vuelo (a diferencia del replay,
// que los guarda en RAM y muxea al final). Un stream passthrough necesita sus cabeceras al
// declararse (SPS/PPS del vídeo, AudioSpecificConfig de cada pista), que solo se conocen tras
// el primer paquete de cada fuente: por eso hay un handshake que bufferea hasta tenerlas.
struct LiveMuxState {
    writer: Option<IMFSinkWriter>,
    video_stream: u32,
    sys_stream: Option<u32>,
    mic_stream: Option<u32>,
    base: i64,
    seq_header: Vec<u8>,
    sys_hdr: Option<(Vec<u8>, u32)>,
    mic_hdr: Option<(Vec<u8>, u32)>,
    pending: Vec<(Option<AudioRole>, Vec<u8>, i64, i64, bool)>,
    writing: bool,
    finalized: bool,
    first_pkt_at: Option<Instant>,
    failed: bool,
}

pub(super) struct LiveMux {
    path: String,
    width: u32,
    height: u32,
    fps: u32,
    bitrate: u32,
    // Pistas de audio esperadas (sample_rate, canales) del AAC ya downmezclado; None = ausente.
    sys: Option<(u32, u16)>,
    mic: Option<(u32, u16)>,
    header_timeout: Mutex<Duration>,
    st: Mutex<LiveMuxState>,
}

// El IMFSinkWriter (COM, !Send/!Sync) vive dentro de `st`; todo acceso pasa por ese Mutex, que
// serializa las llamadas del worker de vídeo y de los hilos de audio. Mismo patrón que
// ReplayPipeline y el antiguo Encoder de la grabación manual.
unsafe impl Send for LiveMux {}
unsafe impl Sync for LiveMux {}

impl LiveMux {
    pub(super) fn new(
        path: String,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
        sys: Option<(u32, u16)>,
        mic: Option<(u32, u16)>,
    ) -> Arc<LiveMux> {
        Arc::new(LiveMux {
            path,
            width,
            height,
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
    pub(super) fn is_writing(&self) -> bool {
        self.st.lock_ok().writing
    }
    #[cfg(test)]
    pub(super) fn set_header_timeout(&self, d: Duration) {
        *self.header_timeout.lock_ok() = d;
    }
    #[cfg(test)]
    pub(super) fn mic_stream_is_none(&self) -> bool {
        self.st.lock_ok().mic_stream.is_none()
    }

    pub(super) fn set_audio_header(&self, role: AudioRole, user_data: Vec<u8>, payload_type: u32) {
        let mut st = self.st.lock_ok();
        match role {
            AudioRole::Sys => st.sys_hdr = Some((user_data, payload_type)),
            AudioRole::Mic => st.mic_hdr = Some((user_data, payload_type)),
        }
    }

    pub(super) fn push_audio(&self, role: AudioRole, data: Vec<u8>, time: i64, dur: i64) {
        let mut st = self.st.lock_ok();
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

    // ¿Están todas las cabeceras esperadas, o venció el timeout (fallback que descarta las
    // pistas de audio que aún no reportaron cabecera, p. ej. un micrófono muerto)?
    fn headers_ready(&self, st: &LiveMuxState) -> bool {
        if st.seq_header.is_empty() {
            return false;
        }
        let sys_ok = self.sys.is_none() || st.sys_hdr.is_some();
        let mic_ok = self.mic.is_none() || st.mic_hdr.is_some();
        if sys_ok && mic_ok {
            return true;
        }
        let timeout = *self.header_timeout.lock_ok();
        st.first_pkt_at.map(|t| t.elapsed() >= timeout).unwrap_or(false)
    }

    // Intenta abrir el SinkWriter y volcar lo pendiente. Requiere base (primer keyframe) y
    // headers_ready().
    fn try_begin(&self, st: &mut LiveMuxState) {
        if st.writing || st.failed || st.base == i64::MIN || !self.headers_ready(st) {
            return;
        }
        match self.open_writer(st) {
            Ok(()) => {
                st.writing = true;
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
            MFCreateFile(
                MF_ACCESSMODE_READWRITE,
                MF_OPENMODE_DELETE_IF_EXIST,
                MF_FILEFLAGS_NONE,
                &url,
            )?
        };
        if let Ok(bs_attr) = byte_stream.cast::<IMFAttributes>() {
            unsafe {
                let _ = bs_attr.SetUINT32(&MF_MPEG4SINK_MOOV_BEFORE_MDAT, 1);
            }
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
            &writer,
            &st.seq_header,
            self.width,
            self.height,
            self.fps,
            self.bitrate,
        )?;

        // Sistema primero (pista de audio por defecto). Solo se declaran las que tienen cabecera.
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

    // Escribe un sample rebasado a `base`. role=None => vídeo. Descarta audio con time < base
    // (el contenedor no admite timestamps negativos), igual que el muxer del replay.
    fn write_one(
        &self,
        st: &mut LiveMuxState,
        role: Option<AudioRole>,
        data: Vec<u8>,
        time: i64,
        dur: i64,
        key: bool,
    ) {
        let Some(writer) = st.writer.clone() else {
            return;
        };
        let ts = time - st.base;
        if ts < 0 {
            return;
        }
        let stream = match role {
            None => st.video_stream,
            Some(AudioRole::Sys) => match st.sys_stream {
                Some(s) => s,
                None => return,
            },
            Some(AudioRole::Mic) => match st.mic_stream {
                Some(s) => s,
                None => return,
            },
        };
        let Ok(mf_buf) = (unsafe { MFCreateMemoryBuffer(data.len() as u32) }) else {
            return;
        };
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
        let Ok(sample) = (unsafe { MFCreateSample() }) else {
            return;
        };
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

    pub(super) fn finalize(&self) -> Option<String> {
        let mut st = self.st.lock_ok();
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
        let mut st = self.st.lock_ok();
        if st.seq_header.is_empty() {
            st.seq_header = bytes;
            self.try_begin(&mut st);
        }
    }
    fn push_video(&self, data: Vec<u8>, time: i64, dur: i64, key: bool) {
        let mut st = self.st.lock_ok();
        if st.finalized || st.failed {
            return;
        }
        st.first_pkt_at.get_or_insert_with(Instant::now);
        // El primer keyframe fija la base temporal; antes de él se descarta (un MP4 no puede
        // empezar fuera de un IDR, igual que save_replay).
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

#[allow(clippy::too_many_arguments)]
pub(super) fn mux_replay(
    path: &str,
    packets: &[(Vec<u8>, i64, i64, bool)],
    seq_header: &[u8],
    width: u32,
    height: u32,
    fps: u32,
    bitrate: u32,
    sys_audio: Option<AudioMuxTrack>,
    mic_audio: Option<AudioMuxTrack>,
) -> Result<()> {
    ensure_mf();
    let url = HSTRING::from(path);

    // Byte stream propio (seekable) para pedir faststart: el sink de MPEG-4 escribe
    // el `moov` (índice) ANTES del `mdat`, así el reproductor empieza al instante en
    // vez de escanear el archivo entero al abrir (lo que daba ~10 s en negro). El
    // atributo MF_MPEG4SINK_MOOV_BEFORE_MDAT se lee del byte stream.
    let byte_stream = unsafe {
        MFCreateFile(
            MF_ACCESSMODE_READWRITE,
            MF_OPENMODE_DELETE_IF_EXIST,
            MF_FILEFLAGS_NONE,
            &url,
        )?
    };
    if let Ok(bs_attr) = byte_stream.cast::<IMFAttributes>() {
        unsafe {
            let _ = bs_attr.SetUINT32(&MF_MPEG4SINK_MOOV_BEFORE_MDAT, 1);
        }
    }

    let attrs = unsafe {
        let mut a: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut a, 2)?;
        let a = a.ok_or_else(null_out)?;
        a.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
        // Sin URL no se infiere el contenedor: hay que decirlo explícitamente.
        a.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
        a
    };
    let writer =
        unsafe { MFCreateSinkWriterFromURL(PCWSTR::null(), &byte_stream, &attrs)? };

    let stream = add_h264_passthrough_stream(&writer, seq_header, width, height, fps, bitrate)?;

    // Sys se declara primero para que sea la pista de audio por defecto.
    let sys_stream = sys_audio
        .as_ref()
        .map(|t| add_aac_passthrough_stream(&writer, t))
        .transpose()?;
    let mic_stream = mic_audio
        .as_ref()
        .map(|t| add_aac_passthrough_stream(&writer, t))
        .transpose()?;

    unsafe { writer.BeginWriting()? };

    let base = packets[0].1;
    let n = packets.len();
    for i in 0..n {
        let data = &packets[i].0;
        let time = packets[i].1;
        let key = packets[i].3;
        // Duración = salto al siguiente frame (captura VFR: WGC no manda duplicados,
        // así que el ritmo real lo marcan los timestamps). El último usa un valor
        // nominal de 1/60 s. De aquí sale la duración correcta del MP4.
        let dur = if i + 1 < n {
            (packets[i + 1].1 - time).max(1)
        } else {
            166_667
        };
        let mf_buf = unsafe { MFCreateMemoryBuffer(data.len() as u32)? };
        unsafe {
            let mut ptr: *mut u8 = std::ptr::null_mut();
            mf_buf.Lock(&mut ptr, None, None)?;
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
            mf_buf.Unlock()?;
            mf_buf.SetCurrentLength(data.len() as u32)?;
        }
        let sample = unsafe { MFCreateSample()? };
        unsafe {
            sample.AddBuffer(&mf_buf)?;
            sample.SetSampleTime(time - base)?;
            sample.SetSampleDuration(dur)?;
            if key {
                sample.SetUINT32(&MFSampleExtension_CleanPoint, 1)?;
            }
            writer.WriteSample(stream, &sample)?;
        }
    }

    if let (Some(stream), Some(track)) = (sys_stream, &sys_audio) {
        write_audio_track(&writer, stream, track, base)?;
    }
    if let (Some(stream), Some(track)) = (mic_stream, &mic_audio) {
        write_audio_track(&writer, stream, track, base)?;
    }

    unsafe { writer.Finalize()? };
    Ok(())
}

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

// Declara un stream de audio en passthrough (entrada == salida, AAC ya codificado):
// mismo idioma que el H.264 de vídeo arriba. El AudioSpecificConfig (MF_MT_USER_DATA)
// viaja en el tipo para que el demuxer/reproductor sepa decodificar el AAC crudo.
fn add_aac_passthrough_stream(writer: &IMFSinkWriter, track: &AudioMuxTrack) -> Result<u32> {
    let media_type = unsafe { MFCreateMediaType()? };
    unsafe {
        media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        media_type.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
        media_type.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, track.sample_rate)?;
        media_type.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, track.channels as u32)?;
        media_type.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        // El tipo AAC debe llevar el byte-rate de AUDIO (no MF_MT_AVG_BITRATE, que es de
        // vídeo): sin él, el sink MP4 no forma un tipo AAC completo y Finalize falla con
        // MF_E_SINK_HEADERS_NOT_FOUND. Es el mismo atributo que usa add_aac_stream (la
        // ruta de grabación manual, que sí funciona).
        media_type.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, track.bitrate / 8)?;
        // Debe coincidir con el framing real emitido por el encoder (ver build_aac_encoder):
        // si no, el sink MP4 genera un `esds` que el decodificador rechaza al reproducir.
        media_type.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, track.payload_type)?;
        if !track.user_data.is_empty() {
            media_type.SetBlob(&MF_MT_USER_DATA, &track.user_data)?;
        }
    }
    let stream = unsafe { writer.AddStream(&media_type)? };
    unsafe { writer.SetInputMediaType(stream, &media_type, None)? };
    Ok(stream)
}

// Paquetes AAC ya codificados: se rebasan al mismo origen que el vídeo (`base`,
// el primer paquete de vídeo tras alinear al keyframe) y se descartan los que
// quedan antes de ese punto, ya que el contenedor no admite timestamps negativos.
fn write_audio_track(
    writer: &IMFSinkWriter,
    stream: u32,
    track: &AudioMuxTrack,
    base: i64,
) -> Result<()> {
    for (data, time, dur) in &track.packets {
        if *time < base {
            continue;
        }
        let mf_buf = unsafe { MFCreateMemoryBuffer(data.len() as u32)? };
        unsafe {
            let mut ptr: *mut u8 = std::ptr::null_mut();
            mf_buf.Lock(&mut ptr, None, None)?;
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
            mf_buf.Unlock()?;
            mf_buf.SetCurrentLength(data.len() as u32)?;
        }
        let sample = unsafe { MFCreateSample()? };
        unsafe {
            sample.AddBuffer(&mf_buf)?;
            sample.SetSampleTime(time - base)?;
            sample.SetSampleDuration(*dur)?;
            writer.WriteSample(stream, &sample)?;
        }
    }
    Ok(())
}
