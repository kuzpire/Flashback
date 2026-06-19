// Captura de audio WASAPI (loopback de sistema + micrófono), desacoplada de capture.rs
// (ver CLAUDE.md §4: módulos con límites claros). No se pasa ningún objeto COM entre
// hilos: cada hilo que lo necesita inicializa su propio apartamento y resuelve el
// dispositivo por su cuenta.

use std::collections::VecDeque;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use windows::core::{Result, GUID, HSTRING, PCWSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, MMDeviceEnumerator, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

const WAVE_FORMAT_IEEE_FLOAT_TAG: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE_TAG: u16 = 0xFFFE;
// KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: no añadimos Win32_Media_Multimedia solo por este GUID.
const SUBTYPE_IEEE_FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);

#[derive(Clone)]
pub enum TrackKind {
    SystemLoopback,
    Microphone(String),
}

pub enum Encoding {
    Pcm,
    Aac(u32),
}

// Toma del PCM ya downmezclado (post-downmix, antes de codificar) de una pista, para
// alimentar el mezclador que genera la pista 0 = mezcla (sistema + micro). Se entrega el
// mismo `time` (QPC) que llevan los paquetes codificados, así el mezclador alinea por
// reloj absoluto compartido entre ambas fuentes.
pub trait PcmTap: Send + Sync + 'static {
    fn on_pcm(&self, pcm: &[u8], time: i64, dur: i64);
}

pub trait AudioSink: Send + Sync + 'static {
    fn push(&self, data: Vec<u8>, time: i64, dur: i64);
    // Mejor esfuerzo: AudioSpecificConfig del AAC, para reconstruir el tipo de salida
    // al muxear desde el ring buffer. Solo lo usa el camino Aac.
    fn set_user_data(&self, _data: Vec<u8>) {}
    // MF_MT_AAC_PAYLOAD_TYPE real del encoder (0 = AAC crudo, sin framing ADTS/LOAS):
    // el muxer de Instant Replay debe declarar el mismo valor o el contenedor queda
    // con una configuración que el decodificador rechaza al reproducir.
    fn set_payload_type(&self, _v: u32) {}
}

pub struct TrackHandle {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl TrackHandle {
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for TrackHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

fn resolve_device(kind: &TrackKind) -> Result<IMMDevice> {
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    match kind {
        TrackKind::SystemLoopback => unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) },
        TrackKind::Microphone(id) => {
            let endpoint = mmdevice_id_from_winrt(id);
            let id = HSTRING::from(endpoint.as_str());
            unsafe { enumerator.GetDevice(&id) }
        }
    }
}

// La lista de micrófonos se obtiene por WinRT (DeviceInformation), cuyos IDs tienen forma
// "\\?\SWD#MMDEVAPI#{0.0.1.00000000}.{guid}#{iface}". IMMDeviceEnumerator::GetDevice espera
// el ID de endpoint de MMDevice ("{0.0.1.00000000}.{guid}"), así que extraemos esa parte; si
// el patrón no aparece, se usa el ID tal cual (ya sería un ID de MMDevice).
fn mmdevice_id_from_winrt(id: &str) -> String {
    const MARK: &str = "MMDEVAPI#";
    if let Some(pos) = id.find(MARK) {
        let rest = &id[pos + MARK.len()..];
        return rest.split('#').next().unwrap_or(rest).to_string();
    }
    id.to_string()
}

// Sample rate/canales nativos del dispositivo, sin abrir un stream real. Llamarlo antes
// de construir el SinkWriter/buffer para declarar el stream de audio con el tipo
// correcto. Se asume que el hilo llamante ya tiene COM inicializado (MTA).
pub fn probe_format(kind: &TrackKind) -> Option<(u32, u16)> {
    let device = resolve_device(kind).ok()?;
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None).ok()? };
    let pwfx = unsafe { client.GetMixFormat().ok()? };
    let (rate, channels) = unsafe { ((*pwfx).nSamplesPerSec, (*pwfx).nChannels) };
    unsafe { CoTaskMemFree(Some(pwfx as *const _)) };
    Some((rate, channels))
}

pub fn spawn_track(
    kind: TrackKind,
    encoding: Encoding,
    sample_rate: u32,
    channels: u16,
    sink: Arc<dyn AudioSink>,
    pcm_tap: Option<Arc<dyn PcmTap>>,
) -> TrackHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_t = stop.clone();
    let handle = std::thread::Builder::new()
        .name("flashback-audio".into())
        .spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            // Si el dispositivo no abre (sin micrófono, endpoint desconectado, etc.) el
            // hilo termina sin más: el sink no recibe nada, pero no compromete el resto
            // de la captura (CLAUDE.md §4.4).
            if let Err(e) =
                run_track(&kind, encoding, sample_rate, channels, &sink, pcm_tap.as_ref(), &stop_t)
            {
                eprintln!("audio: la pista de captura terminó con error: {e:?}");
            }
            unsafe { CoUninitialize() };
        })
        .expect("no se pudo crear el hilo de audio");

    TrackHandle {
        stop,
        handle: Some(handle),
    }
}

fn run_track(
    kind: &TrackKind,
    encoding: Encoding,
    sample_rate: u32,
    channels: u16,
    sink: &Arc<dyn AudioSink>,
    pcm_tap: Option<&Arc<dyn PcmTap>>,
    stop: &Arc<AtomicBool>,
) -> Result<()> {
    let device = resolve_device(kind)?;
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
    let pwfx = unsafe { client.GetMixFormat()? };
    let is_float = unsafe { is_float_format(pwfx) };
    let block_align = unsafe { (*pwfx).nBlockAlign };
    let bits = unsafe { (*pwfx).wBitsPerSample };
    if !is_float && bits != 16 {
        eprintln!("audio: PCM de {bits} bits no convertible (se capturará silencio); hace falta normalizar el formato");
    }

    let mut flags: u32 = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
    if matches!(kind, TrackKind::SystemLoopback) {
        flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
    }

    // Buffer de 2s: generoso para no perder paquetes si el hilo se retrasa un momento;
    // el ritmo real lo marca el evento, no este tamaño.
    let init = unsafe { client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 20_000_000, 0, pwfx, None) };
    unsafe { CoTaskMemFree(Some(pwfx as *const _)) };
    init?;

    // El encoder AAC solo admite 1-2 canales: capturamos a `channels` nativos y hacemos
    // downmix a `dst_ch` (mono/estéreo) antes de codificar/empujar.
    let dst_ch = target_channels(channels);

    let event = unsafe { CreateEventW(None, false, false, PCWSTR::null())? };
    let result = run_track_loop(
        &client, &event, channels, dst_ch, block_align, bits, is_float, sample_rate, encoding,
        sink, pcm_tap, stop,
    );
    unsafe { let _ = CloseHandle(event); }
    result
}

#[allow(clippy::too_many_arguments)]
fn run_track_loop(
    client: &IAudioClient,
    event: &windows::Win32::Foundation::HANDLE,
    channels: u16,
    dst_ch: u16,
    block_align: u16,
    bits: u16,
    is_float: bool,
    sample_rate: u32,
    encoding: Encoding,
    sink: &Arc<dyn AudioSink>,
    pcm_tap: Option<&Arc<dyn PcmTap>>,
    stop: &Arc<AtomicBool>,
) -> Result<()> {
    unsafe { client.SetEventHandle(*event)? };
    let capture: IAudioCaptureClient = unsafe { client.GetService()? };

    let mut aac = match encoding {
        Encoding::Aac(bitrate) => match build_aac_encoder(sample_rate, dst_ch, bitrate) {
            Ok(enc) => Some(enc),
            Err(e) => {
                eprintln!(
                    "audio: el encoder AAC rechazó el formato (rate={sample_rate} ch={dst_ch} bitrate={bitrate}): {e:?}"
                );
                return Err(e);
            }
        },
        Encoding::Pcm => None,
    };

    unsafe { client.Start()? };

    while !stop.load(Ordering::SeqCst) {
        // El evento despierta de inmediato en captura normal (micrófono). En el loopback de
        // sistema el evento puede no señalizarse, así que NO condicionamos el drenaje a
        // WAIT_OBJECT_0: el timeout actúa como sondeo y, en ambos casos, vaciamos los
        // paquetes disponibles. (Antes, en loopback, el timeout hacía `continue` y el audio
        // del sistema no se capturaba jamás.)
        unsafe { WaitForSingleObject(*event, 100); }
        loop {
            let packet = unsafe { capture.GetNextPacketSize() }.unwrap_or(0);
            if packet == 0 {
                break;
            }
            let mut data_ptr: *mut u8 = std::ptr::null_mut();
            let mut frames = 0u32;
            let mut buf_flags = 0u32;
            let mut qpc = 0u64;
            unsafe {
                capture.GetBuffer(&mut data_ptr, &mut frames, &mut buf_flags, None, Some(&mut qpc))?;
            }
            let silent = buf_flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
            let byte_len = frames as usize * block_align as usize;
            let pcm16 = if silent {
                vec![0u8; frames as usize * channels as usize * 2]
            } else {
                let raw = unsafe { std::slice::from_raw_parts(data_ptr, byte_len) };
                if is_float {
                    float_to_pcm16(raw)
                } else if bits == 16 {
                    raw.to_vec()
                } else {
                    Vec::new()
                }
            };
            unsafe { capture.ReleaseBuffer(frames)? };

            if !pcm16.is_empty() && frames > 0 {
                let out = downmix(&pcm16, channels as usize, dst_ch as usize);
                let dur = (frames as i64 * 10_000_000) / sample_rate.max(1) as i64;
                let time = qpc as i64;
                if let Some(tap) = pcm_tap {
                    tap.on_pcm(&out, time, dur);
                }
                emit_encoded(&mut aac, out, time, dur, sink);
            }
        }
    }

    unsafe { let _ = client.Stop(); }
    Ok(())
}

// Codifica (AAC) o reenvía (PCM) un bloque ya downmezclado al sink. Compartido por las
// pistas de captura y por el mezclador, que produce PCM y termina por el mismo camino.
fn emit_encoded(
    aac: &mut Option<AacEncoder>,
    pcm: Vec<u8>,
    time: i64,
    dur: i64,
    sink: &Arc<dyn AudioSink>,
) {
    match aac {
        Some(enc) => encode_aac(enc, &pcm, time, dur, sink),
        None => sink.push(pcm, time, dur),
    }
}

unsafe fn is_float_format(pwfx: *mut WAVEFORMATEX) -> bool {
    let tag = (*pwfx).wFormatTag;
    if tag == WAVE_FORMAT_IEEE_FLOAT_TAG {
        return true;
    }
    if tag == WAVE_FORMAT_EXTENSIBLE_TAG {
        let ext = pwfx as *const WAVEFORMATEXTENSIBLE;
        let sub = (*ext).SubFormat;
        return sub == SUBTYPE_IEEE_FLOAT;
    }
    false
}

fn float_to_pcm16(raw: &[u8]) -> Vec<u8> {
    let samples = raw.len() / 4;
    let mut out = Vec::with_capacity(samples * 2);
    for i in 0..samples {
        let f = f32::from_le_bytes([raw[i * 4], raw[i * 4 + 1], raw[i * 4 + 2], raw[i * 4 + 3]]);
        let v = (f.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

fn target_channels(channels: u16) -> u16 {
    if channels <= 1 {
        1
    } else {
        2
    }
}

// El encoder AAC de Media Foundation solo admite 1-2 canales y 44100/48000 Hz. Dado el
// formato nativo del dispositivo, devuelve el formato AAC admisible más cercano (downmix a
// mono/estéreo) o None si el sample rate exigiría remuestreo (en ese caso el audio se omite
// en vez de romper la captura; los dispositivos virtuales 7.1 que vemos aquí ya son 48 kHz).
pub fn aac_target_format(rate: u32, channels: u16) -> Option<(u32, u16)> {
    if rate != 44100 && rate != 48000 {
        return None;
    }
    Some((rate, target_channels(channels)))
}

// Downmix de PCM16 entrelazado de `src` canales a `dst` (1 o 2). Para >2 canales aplica la
// matriz estándar (frontales íntegros, central y surround a 0.707) y satura a i16; LFE
// (índice 3) se descarta. Si src==dst no transforma. Mantiene el audio inteligible cuando
// la salida del sistema es 5.1/7.1, que el encoder AAC no aceptaría.
fn downmix(pcm: &[u8], src: usize, dst: usize) -> Vec<u8> {
    if src == dst || src == 0 {
        return pcm.to_vec();
    }
    let frames = pcm.len() / (src * 2);
    let rd = |i: usize| i16::from_le_bytes([pcm[i * 2], pcm[i * 2 + 1]]) as f32;

    if dst == 1 {
        let mut out = Vec::with_capacity(frames * 2);
        for f in 0..frames {
            let base = f * src;
            let mut acc = 0.0f32;
            for c in 0..src {
                acc += rd(base + c);
            }
            let v = (acc / src as f32).clamp(-32768.0, 32767.0) as i16;
            out.extend_from_slice(&v.to_le_bytes());
        }
        return out;
    }

    let mut out = Vec::with_capacity(frames * 4);
    for f in 0..frames {
        let base = f * src;
        let mut l = rd(base);
        let mut r = rd(base + 1);
        if src >= 3 {
            let c = 0.707 * rd(base + 2);
            l += c;
            r += c;
        }
        let mut i = 4;
        while i < src {
            let s = 0.707 * rd(base + i);
            if (i - 4) % 2 == 0 {
                l += s;
            } else {
                r += s;
            }
            i += 1;
        }
        let li = l.clamp(-32768.0, 32767.0) as i16;
        let ri = r.clamp(-32768.0, 32767.0) as i16;
        out.extend_from_slice(&li.to_le_bytes());
        out.extend_from_slice(&ri.to_le_bytes());
    }
    out
}

// ===================== Encoder AAC crudo (camino Instant Replay) =====================
//
// A diferencia de la grabación manual (donde el SinkWriter resuelve su propio MFT AAC a
// partir del tipo declarado, igual que ya hace con el H.264), aquí necesitamos los
// paquetes ya codificados para el ring buffer: por eso se maneja el IMFTransform a mano,
// con el mismo idioma ProcessInput/ProcessOutput que ya usa el encoder de vídeo software.

struct AacEncoder {
    mft: IMFTransform,
    provides_output: bool,
    out_size: u32,
    user_data_sent: bool,
}

fn build_aac_encoder(sample_rate: u32, channels: u16, bitrate: u32) -> Result<AacEncoder> {
    let activate = enum_aac_encoder()?
        .ok_or_else(|| windows::core::Error::from_hresult(MF_E_TOPO_CODEC_NOT_FOUND))?;
    let mft: IMFTransform = unsafe { activate.ActivateObject()? };

    // El encoder AAC exige fijar la SALIDA antes que la entrada y solo acepta un tipo de
    // salida idéntico a uno de los que él mismo enumera; construir uno a mano o modificar el
    // enumerado (p. ej. añadiendo MF_MT_AVG_BITRATE) da MF_E_INVALIDMEDIATYPE. Por eso se
    // elige uno de sus tipos admisibles tal cual, priorizando el bytes/seg deseado y AAC
    // crudo (payload 0, lo que espera el `esds` del sink MP4).
    let want_bytes = bitrate / 8;
    let mut chosen: Option<IMFMediaType> = None;
    let mut idx = 0u32;
    loop {
        let t = match unsafe { mft.GetOutputAvailableType(0, idx) } {
            Ok(t) => t,
            Err(_) => break,
        };
        idx += 1;
        let ch = unsafe { t.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap_or(0) };
        let sr = unsafe { t.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).unwrap_or(0) };
        let pt = unsafe { t.GetUINT32(&MF_MT_AAC_PAYLOAD_TYPE).unwrap_or(0) };
        if ch != channels as u32 || sr != sample_rate || pt != 0 {
            continue;
        }
        let b = unsafe { t.GetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND).unwrap_or(0) };
        if b == want_bytes {
            chosen = Some(t);
            break;
        }
        chosen.get_or_insert(t);
    }
    let out_type =
        chosen.ok_or_else(|| windows::core::Error::from_hresult(MF_E_INVALIDMEDIATYPE))?;
    unsafe { mft.SetOutputType(0, &out_type, 0)? };

    let in_type = unsafe { MFCreateMediaType()? };
    unsafe {
        in_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        in_type.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        in_type.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, sample_rate)?;
        in_type.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels as u32)?;
        in_type.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        in_type.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, channels as u32 * 2)?;
        in_type.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, sample_rate * channels as u32 * 2)?;
        mft.SetInputType(0, &in_type, 0)?;
    }

    let info = unsafe { mft.GetOutputStreamInfo(0)? };
    let provides_output = info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32) != 0;

    unsafe { let _ = mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0); }

    Ok(AacEncoder {
        mft,
        provides_output,
        out_size: info.cbSize.max(4096),
        user_data_sent: false,
    })
}

fn enum_aac_encoder() -> Result<Option<IMFActivate>> {
    let info = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Audio,
        guidSubtype: MFAudioFormat_AAC,
    };
    let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count = 0u32;
    unsafe {
        MFTEnumEx(
            MFT_CATEGORY_AUDIO_ENCODER,
            MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER,
            None,
            Some(&info),
            &mut activates,
            &mut count,
        )?;
    }
    if count == 0 || activates.is_null() {
        return Ok(None);
    }
    let first = unsafe { (*activates).clone() };
    for i in 0..count as usize {
        unsafe {
            let _ = std::ptr::read(activates.add(i));
        }
    }
    unsafe { CoTaskMemFree(Some(activates as *const _)) };
    Ok(first)
}

fn encode_aac(enc: &mut AacEncoder, pcm: &[u8], time: i64, dur: i64, sink: &Arc<dyn AudioSink>) {
    let Ok(mf_buf) = (unsafe { MFCreateMemoryBuffer(pcm.len() as u32) }) else {
        return;
    };
    let ok = unsafe {
        let mut ptr: *mut u8 = std::ptr::null_mut();
        if mf_buf.Lock(&mut ptr, None, None).is_err() {
            false
        } else {
            std::ptr::copy_nonoverlapping(pcm.as_ptr(), ptr, pcm.len());
            let _ = mf_buf.Unlock();
            mf_buf.SetCurrentLength(pcm.len() as u32).is_ok()
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
        let _ = sample.SetSampleTime(time);
        let _ = sample.SetSampleDuration(dur);
    }

    for _ in 0..64 {
        match unsafe { enc.mft.ProcessInput(0, &sample, 0) } {
            Ok(()) => break,
            Err(e) if e.code() == MF_E_NOTACCEPTING => drain_aac(enc, sink),
            Err(_) => return,
        }
    }
    drain_aac(enc, sink);
}

fn drain_aac(enc: &mut AacEncoder, sink: &Arc<dyn AudioSink>) {
    loop {
        let mut out = MFT_OUTPUT_DATA_BUFFER::default();
        if !enc.provides_output {
            let (Ok(buf), Ok(sample)) =
                (unsafe { MFCreateMemoryBuffer(enc.out_size) }, unsafe { MFCreateSample() })
            else {
                break;
            };
            unsafe { let _ = sample.AddBuffer(&buf); }
            out.pSample = ManuallyDrop::new(Some(sample));
        }
        let mut status = 0u32;
        let hr = unsafe { enc.mft.ProcessOutput(0, std::slice::from_mut(&mut out), &mut status) };
        let taken = unsafe { ManuallyDrop::take(&mut out.pSample) };
        match hr {
            Ok(()) => {}
            Err(_) => break,
        }

        if !enc.user_data_sent {
            if let Ok(mt) = unsafe { enc.mft.GetOutputCurrentType(0) } {
                if let Some(ud) = blob(&mt, &MF_MT_USER_DATA) {
                    sink.set_user_data(ud);
                    let payload_type =
                        unsafe { mt.GetUINT32(&MF_MT_AAC_PAYLOAD_TYPE) }.unwrap_or(0);
                    sink.set_payload_type(payload_type);
                    enc.user_data_sent = true;
                }
            }
        }

        if let Some(sample) = taken {
            if let Some((data, time, dur)) = read_sample(&sample) {
                sink.push(data, time, dur);
            }
        }
    }
}

fn read_sample(sample: &IMFSample) -> Option<(Vec<u8>, i64, i64)> {
    unsafe {
        let buf = sample.ConvertToContiguousBuffer().ok()?;
        let mut ptr: *mut u8 = std::ptr::null_mut();
        let mut cur = 0u32;
        buf.Lock(&mut ptr, None, Some(&mut cur)).ok()?;
        let data = std::slice::from_raw_parts(ptr, cur as usize).to_vec();
        let _ = buf.Unlock();
        let time = sample.GetSampleTime().unwrap_or(0);
        let dur = sample.GetSampleDuration().unwrap_or(0);
        Some((data, time, dur))
    }
}

fn blob(mt: &IMFMediaType, key: &GUID) -> Option<Vec<u8>> {
    unsafe {
        let size = mt.GetBlobSize(key).ok()?;
        if size == 0 {
            return None;
        }
        let mut v = vec![0u8; size as usize];
        mt.GetBlob(key, &mut v, None).ok()?;
        Some(v)
    }
}

// ===================== Mezclador (pista 0 = sistema + micro) =====================
//
// Combina las dos fuentes PCM ya downmezcladas en una sola pista estéreo que reproduce
// "todo" por defecto en cualquier player (modelo SteelSeries Moments). Las pistas de
// sistema y micro se guardan aparte para el mute/solo no destructivo del editor; esta es
// solo la mezcla de conveniencia. Solo se crea cuando hay ambas fuentes: con una sola, esa
// pista ya es la que suena por defecto y la mezcla sería redundante.
//
// La colocación es por timestamp absoluto (QPC, el mismo reloj en ambas fuentes), así no
// hay deriva acumulada: cada bloque se suma en su posición temporal exacta. Un pequeño
// colchón de latencia da margen a que la otra fuente aporte su parte de cada ventana antes
// de emitirla; lo que falte queda en silencio.

const MIX_LATENCY_HNS: i64 = 1_500_000; // 150 ms

struct SrcState {
    rate: u32,
    channels: u16,
    queue: VecDeque<(i64, Vec<u8>)>,
}

struct MixerShared {
    sys: Mutex<SrcState>,
    mic: Mutex<SrcState>,
}

struct MixTap {
    shared: Arc<MixerShared>,
    mic: bool,
}

impl PcmTap for MixTap {
    fn on_pcm(&self, pcm: &[u8], time: i64, _dur: i64) {
        let m = if self.mic { &self.shared.mic } else { &self.shared.sys };
        m.lock().unwrap().queue.push_back((time, pcm.to_vec()));
    }
}

pub struct MixerHandle {
    shared: Arc<MixerShared>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MixerHandle {
    pub fn system_tap(&self) -> Arc<dyn PcmTap> {
        Arc::new(MixTap { shared: self.shared.clone(), mic: false })
    }

    pub fn mic_tap(&self) -> Arc<dyn PcmTap> {
        Arc::new(MixTap { shared: self.shared.clone(), mic: true })
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for MixerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

// Arranca el hilo del mezclador. Las pistas reales deben pararse ANTES que el mezclador:
// así, al pararlo, sus colas ya están completas y el flush final no pierde la cola del
// audio. La mezcla siempre sale estéreo al `out_rate` (el del sistema, fuente continua).
pub fn spawn_mixer(
    sys_rate: u32,
    sys_ch: u16,
    mic_rate: u32,
    mic_ch: u16,
    out_rate: u32,
    encoding: Encoding,
    sink: Arc<dyn AudioSink>,
) -> MixerHandle {
    let shared = Arc::new(MixerShared {
        sys: Mutex::new(SrcState { rate: sys_rate, channels: sys_ch, queue: VecDeque::new() }),
        mic: Mutex::new(SrcState { rate: mic_rate, channels: mic_ch, queue: VecDeque::new() }),
    });
    let stop = Arc::new(AtomicBool::new(false));
    let shared_t = shared.clone();
    let stop_t = stop.clone();
    let handle = std::thread::Builder::new()
        .name("flashback-mixer".into())
        .spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            run_mixer(&shared_t, out_rate, encoding, &sink, &stop_t);
            unsafe { CoUninitialize() };
        })
        .expect("no se pudo crear el hilo del mezclador");
    MixerHandle { shared, stop, handle: Some(handle) }
}

fn run_mixer(
    shared: &Arc<MixerShared>,
    out_rate: u32,
    encoding: Encoding,
    sink: &Arc<dyn AudioSink>,
    stop: &Arc<AtomicBool>,
) {
    let mut aac = match encoding {
        Encoding::Aac(bitrate) => match build_aac_encoder(out_rate, 2, bitrate) {
            Ok(e) => Some(e),
            Err(e) => {
                eprintln!("audio (mezcla): el encoder AAC rechazó el formato: {e:?}");
                return;
            }
        },
        Encoding::Pcm => None,
    };

    let rate = out_rate.max(1) as i64;
    // Acumulador estéreo intercalado (l, r, l, r, ...) desde `base`. `acc_start` es el
    // índice de frame absoluto de acc[0]; lo ya emitido se descarta por el frente.
    let mut acc: Vec<f32> = Vec::new();
    let mut acc_start: i64 = 0;
    let mut base: Option<i64> = None;
    let mut max_time: i64 = i64::MIN;
    // Cursor de muestra absoluta por fuente: dentro de una racha continua las muestras se
    // colocan SEGUIDAS (no recalculando el índice desde cada timestamp), lo que evita los
    // micro-solapes/huecos por jitter del QPC que sonaban como "escarcha". Solo se
    // resincroniza al timestamp si el salto supera la tolerancia (un hueco real, p. ej.
    // tras un silencio en el loopback).
    let mut sys_next: i64 = i64::MIN;
    let mut mic_next: i64 = i64::MIN;
    let resync_tol = (rate / 100).max(1); // ~10 ms

    loop {
        let stopping = stop.load(Ordering::SeqCst);

        let mut drained: Vec<(i64, Vec<u8>, u32, u16, bool)> = Vec::new();
        {
            let mut s = shared.sys.lock().unwrap();
            let (r, c) = (s.rate, s.channels);
            while let Some((t, pcm)) = s.queue.pop_front() {
                drained.push((t, pcm, r, c, false));
            }
        }
        {
            let mut s = shared.mic.lock().unwrap();
            let (r, c) = (s.rate, s.channels);
            while let Some((t, pcm)) = s.queue.pop_front() {
                drained.push((t, pcm, r, c, true));
            }
        }
        drained.sort_by_key(|d| d.0);

        for (t, pcm, r, c, is_mic) in &drained {
            let b = *base.get_or_insert(*t);
            let expected = ((*t - b) * rate / 10_000_000).max(0);
            let cur = if *is_mic { &mut mic_next } else { &mut sys_next };
            // Racha continua → seguir desde el cursor; salto grande → resincronizar.
            let start = if *cur == i64::MIN || (expected - *cur).abs() > resync_tol {
                expected
            } else {
                *cur
            };
            let placed = place_chunk(&mut acc, acc_start, start, rate, pcm, *r, *c);
            *cur = start + placed;
            let end_t = b + *cur * 10_000_000 / rate;
            max_time = max_time.max(end_t);
        }

        if let Some(b) = base {
            // play_head: hasta dónde es seguro emitir. Al parar, se vacía todo (sin colchón).
            let head_time = if stopping { max_time } else { max_time - MIX_LATENCY_HNS };
            let head_index = ((head_time - b) * rate / 10_000_000).max(0);
            let want = (head_index - acc_start).max(0) as usize;
            let n = want.min(acc.len() / 2);
            if n > 0 {
                let mut pcm16 = Vec::with_capacity(n * 4);
                for k in 0..n {
                    let l = soft_clip_sample(acc[k * 2]);
                    let r = soft_clip_sample(acc[k * 2 + 1]);
                    pcm16.extend_from_slice(&l.to_le_bytes());
                    pcm16.extend_from_slice(&r.to_le_bytes());
                }
                acc.drain(0..n * 2);
                let time = b + acc_start * 10_000_000 / rate;
                let dur = n as i64 * 10_000_000 / rate;
                acc_start += n as i64;
                emit_encoded(&mut aac, pcm16, time, dur, sink);
            }
        }

        if stopping {
            break;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

// Suma un bloque PCM16 (de `src_ch` canales a `src_rate`) en el acumulador estéreo, a partir
// de `start_index` (índice de frame absoluto, ya resuelto por el cursor de la fuente). Si los
// rates difieren, remuestrea por interpolación lineal dentro del propio bloque (calidad
// suficiente para la pista de conveniencia; las pistas separadas conservan su rate nativo
// intacto). Mono se sube a estéreo (L=R). Devuelve el nº de frames de salida colocados.
fn place_chunk(
    acc: &mut Vec<f32>,
    acc_start: i64,
    start_index: i64,
    out_rate: i64,
    pcm: &[u8],
    src_rate: u32,
    src_ch: u16,
) -> i64 {
    let src_ch = src_ch.max(1) as usize;
    let in_frames = pcm.len() / (src_ch * 2);
    if in_frames == 0 {
        return 0;
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
            (rd(frame, 0), rd(frame, 1))
        }
    };

    let src_rate_i = src_rate.max(1) as i64;
    let same_rate = src_rate_i == out_rate;
    let out_frames = if same_rate {
        in_frames
    } else {
        ((in_frames as i64 * out_rate) / src_rate_i).max(0) as usize
    };

    for k in 0..out_frames {
        let (l, r) = if same_rate {
            lr(k.min(in_frames - 1))
        } else {
            let pos = k as f64 * src_rate_i as f64 / out_rate as f64;
            let i0 = (pos.floor() as usize).min(in_frames - 1);
            let i1 = (i0 + 1).min(in_frames - 1);
            let frac = (pos - pos.floor()) as f32;
            let (l0, r0) = lr(i0);
            let (l1, r1) = lr(i1);
            (l0 + (l1 - l0) * frac, r0 + (r1 - r0) * frac)
        };
        let abs_index = start_index + k as i64;
        if abs_index < acc_start {
            continue;
        }
        let rel = (abs_index - acc_start) as usize;
        let needed = (rel + 1) * 2;
        if acc.len() < needed {
            acc.resize(needed, 0.0);
        }
        acc[rel * 2] += l;
        acc[rel * 2 + 1] += r;
    }
    out_frames as i64
}

// Suma de dos fuentes a tope satura: en vez de recortar en duro (que mete distorsión
// áspera), aplicamos un soft clip. Lineal por debajo del umbral (transparente para el caso
// normal de una sola fuente sonando) y compresión suave por encima hasta el máximo. Solo
// afecta a la pista mezcla; sistema y micro se guardan sin tocar.
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
