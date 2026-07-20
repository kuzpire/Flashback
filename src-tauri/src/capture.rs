use serde::Serialize;

// Estado de la captura para la UI. En la Fase 1 solo sirve para verificar que el
// bucle WGC corre y medir su impacto: cuántos frames llegan y a qué resolución.
#[derive(Serialize, Clone, Default)]
pub struct CaptureStatus {
    pub running: bool,
    pub frames: u64,
    pub width: u32,
    pub height: u32,
    pub seconds: f64,
}

#[derive(Serialize, Clone, Default)]
pub struct MonitorInfo {
    pub id: String,
    pub label: String,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
    pub thumb: Option<String>,
}

#[derive(Serialize, Clone, Default)]
pub struct AudioInput {
    pub id: String,
    pub name: String,
}

#[cfg(target_os = "windows")]
pub use win::{
    list_audio_inputs, list_monitors, replay_active, save_replay, start, start_replay, status,
    stop, stop_replay,
};

#[cfg(not(target_os = "windows"))]
pub fn list_monitors() -> Vec<MonitorInfo> {
    Vec::new()
}

#[cfg(not(target_os = "windows"))]
pub fn list_audio_inputs() -> Vec<AudioInput> {
    Vec::new()
}

#[cfg(not(target_os = "windows"))]
pub fn start(
    _monitor_id: String,
    _out_dir: String,
    _fps: u32,
    _quality: String,
    _resolution: u32,
    _bitrate: u32,
    _mic: bool,
    _mic_device: String,
    _encoder_pref: String,
) -> Result<(), String> {
    Err("La captura solo está disponible en Windows".into())
}

#[cfg(not(target_os = "windows"))]
pub fn stop() -> Option<String> {
    None
}

#[cfg(not(target_os = "windows"))]
pub fn status() -> CaptureStatus {
    CaptureStatus::default()
}

#[cfg(not(target_os = "windows"))]
pub fn start_replay(
    _monitor_id: String,
    _out_dir: String,
    _seconds: u32,
    _fps: u32,
    _quality: String,
    _resolution: u32,
    _bitrate: u32,
    _mic: bool,
    _mic_device: String,
    _encoder_pref: String,
) -> Result<(), String> {
    Err("El replay solo está disponible en Windows".into())
}

#[cfg(not(target_os = "windows"))]
pub fn stop_replay() {}

#[cfg(not(target_os = "windows"))]
pub fn save_replay(_source: &str) -> Option<String> {
    None
}

#[cfg(not(target_os = "windows"))]
pub fn replay_active() -> bool {
    false
}

#[cfg(target_os = "windows")]
mod win {
    use std::collections::VecDeque;
    use std::mem::ManuallyDrop;
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Condvar, Mutex, Once};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use windows::core::{IInspectable, Interface, Result, BOOL, GUID, HSTRING, PCWSTR};
    use windows::Devices::Enumeration::{DeviceClass, DeviceInformation};
    use windows::Foundation::TypedEventHandler;
    use windows::Graphics::Capture::{
        Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
    };
    use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
    use windows::Graphics::DirectX::DirectXPixelFormat;
    use windows::Win32::Foundation::{E_POINTER, HANDLE, HMODULE, HWND, LPARAM, RECT, SYSTEMTIME};
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D,
        D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_FLAG,
        D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
        D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_RESOURCE_MISC_FLAG, D3D11_SDK_VERSION,
        D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
    };
    use windows::Win32::Graphics::Dxgi::Common::{
        DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC,
    };
    use windows::Win32::Graphics::Dxgi::IDXGIDevice;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, EnumDisplayMonitors,
        GetDC, GetDIBits, GetMonitorInfoW, ReleaseDC, SelectObject, SetStretchBltMode, StretchBlt,
        BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HALFTONE, HDC, HMONITOR, MONITORINFO,
        MONITORINFOEXW, SRCCOPY,
    };
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
    use windows::Win32::System::Threading::{
        AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, AvSetMmThreadPriority,
        AVRT_PRIORITY_HIGH,
    };

    // Valor Win32 de MONITORINFOF_PRIMARY (no lo genera el crate windows).
    const MONITORINFOF_PRIMARY: u32 = 1;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    };
    use windows::Win32::System::SystemInformation::GetLocalTime;
    use windows::Win32::System::Variant::{VARIANT, VT_BOOL, VT_UI4};
    use windows::Win32::System::WinRT::Direct3D11::{
        CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
    };
    use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindow, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
        GW_OWNER,
    };

    use super::{AudioInput, CaptureStatus, MonitorInfo};
    use crate::audio;

    #[derive(Default)]
    struct Stats {
        frames: AtomicU64,
        width: AtomicU32,
        height: AtomicU32,
    }

    struct Running {
        stop: Arc<(Mutex<bool>, Condvar)>,
        handle: Option<JoinHandle<()>>,
        stats: Arc<Stats>,
        started: Instant,
        result: Arc<Mutex<Option<String>>>,
    }

    static STATE: Mutex<Option<Running>> = Mutex::new(None);

    // Media Foundation se inicializa una sola vez por proceso; no se apaga porque
    // vive lo que dure la app.
    static MF_INIT: Once = Once::new();
    fn ensure_mf() {
        MF_INIT.call_once(|| unsafe {
            let _ = MFStartup(MF_VERSION, MFSTARTUP_FULL);
        });
    }

    pub fn list_monitors() -> Vec<MonitorInfo> {
        // Un único DC del escritorio virtual sirve para fotografiar todas las
        // pantallas (cada una vive en su trozo de coordenadas del rcMonitor).
        let screen_dc = unsafe { GetDC(None) };
        let monitors = enum_monitors()
            .into_iter()
            .enumerate()
            .filter_map(|(i, hmon)| monitor_info(hmon, i, screen_dc))
            .collect();
        if !screen_dc.is_invalid() {
            unsafe { ReleaseDC(None, screen_dc) };
        }
        monitors
    }

    // Entradas de audio (micrófonos) del sistema, con su nombre amigable. Vía
    // WinRT DeviceInformation: da el nombre sin pedir permiso de micrófono. Se
    // hace en un hilo MTA propio para no depender del apartamento del llamante.
    pub fn list_audio_inputs() -> Vec<AudioInput> {
        std::thread::spawn(|| {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            let out = audio_inputs().unwrap_or_default();
            unsafe { CoUninitialize() };
            out
        })
        .join()
        .unwrap_or_default()
    }

    fn audio_inputs() -> Result<Vec<AudioInput>> {
        let collection = DeviceInformation::FindAllAsyncDeviceClass(DeviceClass::AudioCapture)?.get()?;
        let mut out = Vec::new();
        for device in &collection {
            out.push(AudioInput {
                id: device.Id()?.to_string(),
                name: device.Name()?.to_string(),
            });
        }
        Ok(out)
    }

    pub fn start(
        target: String,
        out_dir: String,
        fps: u32,
        quality: String,
        resolution: u32,
        bitrate: u32,
        mic: bool,
        mic_device: String,
        encoder_pref: String,
    ) -> std::result::Result<(), String> {
        let mut guard = STATE.lock().unwrap();
        if guard.is_some() {
            return Ok(());
        }

        let fps = clamp_fps(fps);
        let factor = bitrate_factor(&quality);
        let stats = Arc::new(Stats::default());
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let result = Arc::new(Mutex::new(None));
        let (ready_tx, ready_rx) = mpsc::channel::<std::result::Result<(), String>>();

        let stats_t = stats.clone();
        let stop_t = stop.clone();
        let result_t = result.clone();
        let handle = std::thread::Builder::new()
            .name("flashback-capture".into())
            .spawn(move || {
                capture_thread(
                    target, out_dir, fps, factor, resolution, bitrate, mic, mic_device,
                    encoder_pref, stop_t, stats_t, result_t, ready_tx,
                )
            })
            .map_err(|e| e.to_string())?;

        // El hilo construye el pipeline WGC + encoder y reporta éxito o error antes
        // de ponerse a recibir frames; así start() puede devolver un fallo real.
        match ready_rx.recv() {
            Ok(Ok(())) => {
                *guard = Some(Running {
                    stop,
                    handle: Some(handle),
                    stats,
                    started: Instant::now(),
                    result,
                });
                Ok(())
            }
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(_) => Err("El hilo de captura terminó inesperadamente".into()),
        }
    }

    // Devuelve la ruta del MP4 guardado (None si algo falló al finalizar el muxer).
    pub fn stop() -> Option<String> {
        let running = STATE.lock().unwrap().take();
        if let Some(mut running) = running {
            let (lock, cv) = &*running.stop;
            *lock.lock().unwrap() = true;
            cv.notify_all();
            if let Some(h) = running.handle.take() {
                let _ = h.join();
            }
            return running.result.lock().unwrap().take();
        }
        None
    }

    pub fn status() -> CaptureStatus {
        let guard = STATE.lock().unwrap();
        match guard.as_ref() {
            Some(r) => CaptureStatus {
                running: true,
                frames: r.stats.frames.load(Ordering::Relaxed),
                width: r.stats.width.load(Ordering::Relaxed),
                height: r.stats.height.load(Ordering::Relaxed),
                seconds: r.started.elapsed().as_secs_f64(),
            },
            None => CaptureStatus::default(),
        }
    }

    // El hilo de captura es dueño de los objetos COM/WGC: los crea con su propio
    // apartamento MTA y los suelta en el mismo hilo antes de salir.
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
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let _timer = TimerRes::new();
        let _mmcss = MmcssTask::new("Capture");

        let mut engine = match resolve_target_item(&target).and_then(|item| {
            build_engine(
                &stats, item, &out_dir, fps, factor, resolution, bitrate_override, mic, mic_device,
                &encoder_pref,
            )
            .map_err(|e| format!("{e:?}"))
        }) {
            Ok(e) => {
                let _ = ready.send(Ok(()));
                e
            }
            Err(e) => {
                let _ = ready.send(Err(e));
                unsafe { CoUninitialize() };
                return;
            }
        };

        // Cadencia CFR: el handler de WGC solo refresca el "último frame"; aquí emitimos uno
        // por slot de reloj (duplicando el último si no llegó otro), de modo que el clip salga
        // a fps constante. Se espera con wait_timeout para reaccionar al instante a stop().
        let enc = engine.encoder.clone();
        let interval = fps_interval(fps).max(1);
        let mut t0: Option<Instant> = None;
        let mut emitted: i64 = -1;
        let (lock, cv) = &*stop;
        loop {
            {
                let mut e = enc.lock().unwrap();
                if let Some(frame_slot) = e.latest_slot(interval) {
                    let start = *t0.get_or_insert_with(Instant::now);
                    let wall = elapsed_slots(start, interval);
                    // Cadencia guiada por el timestamp del frame (fase estable => sin batido); el
                    // reloj de pared solo rellena duplicados en congelados reales (>=STALL_SLOTS).
                    let cur = if wall - frame_slot >= STALL_SLOTS { wall - 1 } else { frame_slot };
                    // Sobrecarga (encoder por debajo de la tasa): no acumular retraso sin fin;
                    // resincronizar cerca del slot actual. A 60 fps no debería ocurrir.
                    if cur - emitted > fps as i64 {
                        emitted = cur - 1;
                    }
                    while emitted < cur {
                        emitted += 1;
                        let pts = cfr_pts(emitted, fps);
                        let dur = cfr_pts(emitted + 1, fps) - pts;
                        let _ = e.emit_paced(pts, dur);
                    }
                }
            }
            let stopped = lock.lock().unwrap();
            if *stopped {
                break;
            }
            let (stopped, _) = cv.wait_timeout(stopped, pace_sleep(interval)).unwrap();
            if *stopped {
                break;
            }
            drop(stopped);
        }

        // Orden de parada: cortar primero la llegada de frames (para que ningún
        // WriteSample corra contra Finalize) y luego cerrar el MP4.
        engine.shutdown();
        *result.lock().unwrap() = engine.finalize_encoder();

        drop(engine);
        unsafe { CoUninitialize() };
    }

    struct Engine {
        _device: ID3D11Device,
        frame_pool: Direct3D11CaptureFramePool,
        session: GraphicsCaptureSession,
        token: i64,
        encoder: Arc<Mutex<Encoder>>,
        audio_tracks: Vec<audio::TrackHandle>,
    }

    impl Engine {
        fn shutdown(&mut self) {
            let _ = self.frame_pool.RemoveFrameArrived(self.token);
            let _ = self.session.Close();
            for track in &mut self.audio_tracks {
                track.stop();
            }
        }

        fn finalize_encoder(&self) -> Option<String> {
            self.encoder.lock().unwrap().finalize()
        }
    }

    impl Drop for Engine {
        fn drop(&mut self) {
            let _ = self.frame_pool.RemoveFrameArrived(self.token);
            let _ = self.session.Close();
            let _ = self.frame_pool.Close();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_engine(
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
    ) -> Result<Engine> {
        let (device, d3d_device) = create_device()?;
        // NV12/H.264 exigen dimensiones PARES (ver nota en build_replay): la ventana de un
        // juego puede ser impar y disparaba MF_E_INVALIDMEDIATYPE. Redondear a par.
        let mut size = item.Size()?;
        size.Width = size.Width.max(2) & !1;
        size.Height = size.Height.max(2) & !1;
        let width = size.Width as u32;
        let height = size.Height as u32;
        // Se captura a nativo y el encoder escala al objetivo (downscale). El bitrate se
        // calcula sobre la resolución de salida, no la de captura.
        let (out_w, out_h) = output_dims(width, height, resolution);
        let bitrate = resolve_bitrate(out_w, out_h, fps, factor, bitrate_override);

        // Sistema: loopback siempre, en ambos modos de captura (pantalla y aplicación).
        // Micrófono: solo si el toggle está activo y hay dispositivo elegido. Se declara el
        // stream con el formato AAC admisible (downmix a estéreo); la captura es a nativo.
        let sys_native = audio::probe_format(&audio::TrackKind::SystemLoopback);
        let mic_native = if mic && !mic_device.is_empty() {
            let f = audio::probe_format(&audio::TrackKind::Microphone(mic_device.clone()));
            if f.is_none() {
                eprintln!("audio: no se pudo abrir el micrófono (device='{mic_device}')");
            }
            f
        } else {
            if mic {
                eprintln!("audio: micrófono activado pero sin dispositivo seleccionado");
            }
            None
        };
        let sys_target = sys_native.and_then(|(r, c)| audio::aac_target_format(r, c));
        let mic_target = mic_native.and_then(|(r, c)| audio::aac_target_format(r, c));

        let out_path = format!("{out_dir}\\{}", clip_filename());
        let encoder = Arc::new(Mutex::new(Encoder::new(
            &device, width, height, out_w, out_h, fps, bitrate, out_path, sys_target,
            mic_target, encoder_pref,
        )?));

        let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &d3d_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )?;
        let session = frame_pool.CreateCaptureSession(&item)?;
        // Sin el borde de captura de WGC (si el SO lo permite): no queremos esa línea de
        // color alrededor de la ventana/pantalla dentro del clip grabado.
        let _ = session.SetIsBorderRequired(false);
        set_capture_rate(&session, fps);

        // El handler corre en el pool de hilos del sistema (frame pool free-threaded):
        // recoge la textura del frame y la empuja al encoder por hardware. La textura
        // WGC se copia GPU→GPU dentro del encoder (no baja a CPU): el camino sagrado.
        // El límite de FPS descarta frames antes de tocar la GPU para no encodear de más.
        let stats = stats.clone();
        let enc = encoder.clone();
        let interval = fps_interval(fps);
        let selector = Arc::new(SlotSelector::new());
        let handler = TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(
            move |pool, _| {
                ensure_handler_priority();
                if let Some(pool) = pool.as_ref() {
                    if let Ok(frame) = pool.TryGetNextFrame() {
                        if let Ok(s) = frame.ContentSize() {
                            stats.width.store(s.Width.max(0) as u32, Ordering::Relaxed);
                            stats.height.store(s.Height.max(0) as u32, Ordering::Relaxed);
                        }
                        let t = frame.SystemRelativeTime().map(|x| x.Duration).unwrap_or(0);
                        if selector.keep(t, interval) {
                            if let Ok(surface) = frame.Surface() {
                                if let Ok(access) = surface.cast::<IDirect3DDxgiInterfaceAccess>() {
                                    if let Ok(tex) =
                                        unsafe { access.GetInterface::<ID3D11Texture2D>() }
                                    {
                                        // Solo refresca el "último frame" (copia GPU→GPU); la
                                        // emisión a cadencia CFR la hace el hilo de captura.
                                        enc.lock().unwrap().note_frame(&tex, t);
                                    }
                                }
                            }
                            stats.frames.fetch_add(1, Ordering::Relaxed);
                        }
                        let _ = frame.Close();
                    }
                }
                Ok(())
            },
        );
        let token = frame_pool.FrameArrived(&handler)?;

        let mut audio_tracks = Vec::new();
        if let (Some((rate, ch)), Some(_)) = (sys_native, sys_target) {
            let stream = encoder.lock().unwrap().sys_audio_stream.expect("stream de sistema declarado");
            let sink = Arc::new(EncoderAudioSink { encoder: encoder.clone(), stream });
            audio_tracks.push(audio::spawn_track(
                audio::TrackKind::SystemLoopback,
                audio::Encoding::Pcm,
                rate,
                ch,
                sink,
                None,
            ));
        }
        if let (Some((rate, ch)), Some(_)) = (mic_native, mic_target) {
            let stream = encoder.lock().unwrap().mic_audio_stream.expect("stream de mic declarado");
            let sink = Arc::new(EncoderAudioSink { encoder: encoder.clone(), stream });
            audio_tracks.push(audio::spawn_track(
                audio::TrackKind::Microphone(mic_device.clone()),
                audio::Encoding::Pcm,
                rate,
                ch,
                sink,
                None,
            ));
        }

        session.StartCapture()?;

        Ok(Engine {
            _device: device,
            frame_pool,
            session,
            token,
            encoder,
            audio_tracks,
        })
    }

    // Device D3D11 con soporte BGRA (obligatorio para el interop de WGC) y de vídeo
    // (lo exige Media Foundation para codificar por hardware compartiendo el device),
    // más su equivalente WinRT IDirect3DDevice, que es lo que consume el frame pool.
    fn create_device() -> Result<(ID3D11Device, IDirect3DDevice)> {
        let mut device: Option<ID3D11Device> = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )?;
        }
        let device = device.expect("D3D11CreateDevice no devolvió device");
        let dxgi: IDXGIDevice = device.cast()?;
        // Prioridad de scheduling GPU máxima para este device (dentro de nuestro proceso).
        let _ = unsafe { dxgi.SetGPUThreadPriority(7) };
        let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? };
        let d3d_device: IDirect3DDevice = inspectable.cast()?;
        Ok((device, d3d_device))
    }

    // Encoder H.264 por hardware vía Media Foundation. El SinkWriter hospeda el MFT
    // por hardware (NVENC/AMF/QSV) y el conversor de color BGRA→NV12, ambos en GPU
    // gracias al DXGI device manager que comparte nuestro ID3D11Device. Le damos las
    // texturas BGRA de WGC y escribe el MP4 (H.264) directamente.
    struct Encoder {
        writer: IMFSinkWriter,
        stream: u32,
        sys_audio_stream: Option<u32>,
        mic_audio_stream: Option<u32>,
        ctx: ID3D11DeviceContext,
        pool: Vec<ID3D11Texture2D>,
        next: usize,
        // Índice en `pool` del último frame real recibido de WGC; lo emite la cadencia CFR,
        // duplicándolo en los slots sin frame nuevo. None hasta que llega el primero.
        latest: Option<usize>,
        base: i64,
        has_base: bool,
        // Timestamp WGC (SystemRelativeTime) del último frame real: ancla la rejilla CFR al reloj
        // del juego para que la cadencia no bata con el reloj de pared (evita micro-tirones).
        last_time: i64,
        path: String,
        finalized: bool,
        audio_err_logged: bool,
    }

    // El handler de FrameArrived exige Send+Sync. El Encoder solo se toca bajo el
    // Mutex y desde el callback (que WGC serializa), con el device protegido para
    // multihilo, así que moverlo entre hilos de forma sincronizada es seguro.
    unsafe impl Send for Encoder {}

    impl Encoder {
        // (in_w,in_h) = resolución de captura nativa que se le entrega; (out_w,out_h) =
        // resolución del MP4. Si difieren, el SinkWriter intercala un Video Processor que
        // escala (en GPU, vía el device manager compartido) antes del encoder H.264.
        #[allow(clippy::too_many_arguments)]
        fn new(
            device: &ID3D11Device,
            in_w: u32,
            in_h: u32,
            out_w: u32,
            out_h: u32,
            fps: u32,
            bitrate: u32,
            path: String,
            sys_audio: Option<(u32, u16)>,
            mic_audio: Option<(u32, u16)>,
            encoder_pref: &str,
        ) -> Result<Encoder> {
            ensure_mf();

            // Compartir el mismo device con MF: así el encoder lee nuestras texturas
            // sin copiarlas a CPU. El device debe estar protegido para multihilo.
            let mut token = 0u32;
            let mut manager: Option<IMFDXGIDeviceManager> = None;
            unsafe { MFCreateDXGIDeviceManager(&mut token, &mut manager)? };
            let manager = manager.ok_or_else(null_out)?;
            unsafe { manager.ResetDevice(device, token)? };

            let ctx = unsafe { device.GetImmediateContext()? };
            if let Ok(mt) = ctx.cast::<ID3D11Multithread>() {
                let _ = unsafe { mt.SetMultithreadProtected(true) };
            }

            let use_hw = encoder_pref != "Software"
                && std::env::var_os("FLASHBACK_FORCE_SW_ENCODER").is_none();
            let attrs = unsafe {
                let mut a: Option<IMFAttributes> = None;
                MFCreateAttributes(&mut a, 3)?;
                let a = a.ok_or_else(null_out)?;
                if use_hw {
                    a.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
                    a.SetUnknown(&MF_SINK_WRITER_D3D_MANAGER, &manager)?;
                }
                a.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
                a
            };

            let url = HSTRING::from(path.as_str());
            let writer = unsafe { MFCreateSinkWriterFromURL(&url, None, &attrs)? };

            // fps nominal: WGC no entrega duplicados, así que el ritmo real lo marcan los
            // timestamps por muestra; aquí es metadato/objetivo del rate control. El
            // límite real de FPS lo aplica el handler descartando frames.
            let out_type = unsafe { MFCreateMediaType()? };
            unsafe {
                out_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                out_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
                out_type.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
                out_type.SetUINT32(&MF_MT_MPEG2_PROFILE, 100)?;
                out_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                out_type.SetUINT64(&MF_MT_FRAME_SIZE, pack2(out_w, out_h))?;
                out_type.SetUINT64(&MF_MT_FRAME_RATE, pack2(fps, 1))?;
                out_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack2(1, 1))?;
            }
            let stream = unsafe { writer.AddStream(&out_type)? };

            let in_type = unsafe { MFCreateMediaType()? };
            unsafe {
                in_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                in_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_ARGB32)?;
                in_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                in_type.SetUINT64(&MF_MT_FRAME_SIZE, pack2(in_w, in_h))?;
                in_type.SetUINT64(&MF_MT_FRAME_RATE, pack2(fps, 1))?;
                in_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack2(1, 1))?;
                writer.SetInputMediaType(stream, &in_type, None)?;
            }

            // PCM crudo de entrada: el SinkWriter resuelve su propio MFT AAC, igual que
            // ya resuelve el MFT H.264 a partir del tipo de vídeo declarado arriba.
            let sys_audio_stream = match sys_audio {
                Some((rate, ch)) => Some(add_aac_stream(&writer, rate, ch)?),
                None => None,
            };
            let mic_audio_stream = match mic_audio {
                Some((rate, ch)) => Some(add_aac_stream(&writer, rate, ch)?),
                None => None,
            };

            // Misma política de calidad que el replay (VBR + CABAC + GOP ~2 s) sobre el
            // encoder que hospeda el SinkWriter. El encoder NO tiene por qué ser el primer
            // transform del stream: con entrada ARGB32 el SinkWriter intercala un conversor
            // de color, así que recorremos los transforms y aplicamos a los que expongan
            // ICodecAPI. El conversor ignora las propiedades de encoder (best-effort). Se
            // hace antes de BeginWriting, cuando la cadena ya está resuelta.
            //
            // NO re-aplicar SetOutputType aquí para forzar el VBR: manipular a mano el tipo de
            // salida del encoder de vídeo antes de BeginWriting corrompe la resolución diferida
            // de los streams de audio del SinkWriter, y el primer WriteSample de audio falla con
            // MF_E_TRANSFORM_TYPE_NOT_SET. El VBR queda como best-effort vía SetValue (el audio,
            // función principal, tiene prioridad sobre esa mejora de calidad).
            if let Ok(ex) = writer.cast::<IMFSinkWriterEx>() {
                for idx in 0..8u32 {
                    let mut transform: Option<IMFTransform> = None;
                    if unsafe { ex.GetTransformForStream(stream, idx, None, &mut transform) }.is_err()
                    {
                        break;
                    }
                    let Some(t) = transform else { continue };
                    let Ok(codec) = t.cast::<ICodecAPI>() else { continue };
                    let gop = (fps * 2).max(16);
                    let failed = set_quality_codec_settings(&codec, bitrate, gop);
                    log_encoder_quality(&codec, "manual", bitrate, gop, &failed);
                }
            }

            unsafe { writer.BeginWriting()? };

            // Anillo de texturas propias: WGC reutiliza las suyas en cuanto soltamos
            // el frame, pero el encoder es asíncrono y puede leerlas más tarde; copiar
            // a una textura nuestra (rotando varias) evita esa carrera. Van a resolución
            // de captura (in_*): el escalado a out_* lo hace el SinkWriter.
            let desc = D3D11_TEXTURE2D_DESC {
                Width: in_w,
                Height: in_h,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE).0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_FLAG(0).0 as u32,
                MiscFlags: D3D11_RESOURCE_MISC_FLAG(0).0 as u32,
            };
            let mut pool = Vec::with_capacity(6);
            for _ in 0..6 {
                let mut t: Option<ID3D11Texture2D> = None;
                unsafe { device.CreateTexture2D(&desc, None, Some(&mut t))? };
                pool.push(t.ok_or_else(null_out)?);
            }

            Ok(Encoder {
                writer,
                stream,
                sys_audio_stream,
                mic_audio_stream,
                ctx,
                pool,
                next: 0,
                latest: None,
                base: 0,
                has_base: false,
                last_time: 0,
                path,
                finalized: false,
                audio_err_logged: false,
            })
        }

        // El audio no se ancla a keyframes: cada paquete AAC es independiente. La base de
        // tiempo la fija siempre el primer frame de vídeo (note_frame), porque el PTS del
        // vídeo es sintético (cadencia CFR) y arranca en 0 en ese instante; el audio se
        // rebasa contra esa misma base. Mientras no haya base aún se descartan los paquetes
        // (no hay forma fiable de alinearlos), igual que en el Instant Replay.
        fn push_audio(&mut self, stream: u32, data: Vec<u8>, time: i64, dur: i64) {
            if !self.has_base {
                return;
            }
            let ts = (time - self.base).max(0);
            let len = data.len();
            let Ok(mf_buf) = (unsafe { MFCreateMemoryBuffer(len as u32) }) else {
                return;
            };
            let ok = unsafe {
                let mut ptr: *mut u8 = std::ptr::null_mut();
                if mf_buf.Lock(&mut ptr, None, None).is_err() {
                    false
                } else {
                    std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, len);
                    let _ = mf_buf.Unlock();
                    mf_buf.SetCurrentLength(len as u32).is_ok()
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
                if let Err(e) = self.writer.WriteSample(stream, &sample) {
                    if !self.audio_err_logged {
                        self.audio_err_logged = true;
                        eprintln!("audio (grabación manual): WriteSample del audio falló: {e:?}");
                    }
                }
            }
        }

        // Copia el frame WGC a un slot del pool (GPU→GPU, sin bajar a CPU: el camino sagrado)
        // y lo marca como el "último" disponible. NO codifica aquí: el hilo de cadencia decide
        // cuándo emitirlo. El primer frame fija la base temporal del vídeo, contra la que se
        // rebasa el audio. Solo rota a un slot nuevo; el anterior sigue intacto para que la
        // cadencia pueda duplicarlo mientras no llegue otro frame.
        fn note_frame(&mut self, src: &ID3D11Texture2D, time: i64) {
            if !self.has_base {
                self.has_base = true;
                self.base = time;
            }
            self.last_time = time;
            let idx = self.next;
            self.next = (self.next + 1) % self.pool.len();
            unsafe { self.ctx.CopyResource(&self.pool[idx], src) };
            self.latest = Some(idx);
        }

        // Slot CFR del último frame real por su timestamp (base = primer frame). None si aún no
        // llegó ninguno. La cadencia se guía por esto, no por el reloj de pared (sin batido).
        fn latest_slot(&self, interval: i64) -> Option<i64> {
            self.latest.map(|_| time_slot(self.last_time, self.base, interval))
        }

        // Escribe el último frame disponible con un PTS sintético de cadencia constante. Se
        // llama una vez por slot vencido; un slot sin frame nuevo reescribe el mismo (frame
        // duplicado), que el encoder resuelve como P-frame "skip" de coste ínfimo.
        fn emit_paced(&mut self, pts: i64, dur: i64) -> Result<()> {
            let Some(idx) = self.latest else {
                return Ok(());
            };
            let dst = self.pool[idx].clone();
            let buffer =
                unsafe { MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, &dst, 0, false)? };
            let len = unsafe { buffer.cast::<IMF2DBuffer>()?.GetContiguousLength()? };
            unsafe { buffer.SetCurrentLength(len)? };

            let sample = unsafe { MFCreateSample()? };
            unsafe {
                sample.AddBuffer(&buffer)?;
                sample.SetSampleTime(pts)?;
                sample.SetSampleDuration(dur.max(1))?;
                self.writer.WriteSample(self.stream, &sample)?;
            }
            Ok(())
        }

        fn finalize(&mut self) -> Option<String> {
            if self.finalized {
                return Some(self.path.clone());
            }
            self.finalized = true;
            unsafe { self.writer.Finalize().ok()? };
            Some(self.path.clone())
        }
    }

    // ===================== INSTANT REPLAY (Fase 2b) =====================
    //
    // A diferencia de la grabación manual (Fase 2a, que delega en IMFSinkWriter y no
    // expone los paquetes), aquí poseemos el MFT del encoder H.264 por hardware para
    // tee-ar sus paquetes ya codificados a un ring buffer en RAM. Como los encoders HW
    // comen NV12, intercalamos el Video Processor MFT (BGRA→NV12 en GPU). Se codifica
    // siempre en segundo plano con IDR forzado periódico; al guardar, se muxea desde el
    // último IDR a un MP4 con un sink writer en passthrough (sin recodificar).

    const CLSID_VIDEO_PROCESSOR_MFT: GUID = GUID::from_u128(0x88753b26_5b24_49bd_b2e7_0c445c78c982);

    // Valores de MF_EVENT_TYPE para los MFT asíncronos (no dependemos de cómo los
    // represente el crate).
    const ME_TRANSFORM_NEED_INPUT: u32 = 601;
    const ME_TRANSFORM_HAVE_OUTPUT: u32 = 602;

    struct Packet {
        data: Vec<u8>,
        time: i64,
        dur: i64,
        key: bool,
    }

    // Cada paquete AAC es independiente (no hay GOP/keyframes), así que se poda por
    // tiempo sin anclar a nada: simplemente se descartan los más viejos que la ventana.
    struct AudioTrackBuf {
        packets: VecDeque<Packet>,
        sample_rate: u32,
        channels: u16,
        bitrate: u32,
        user_data: Vec<u8>,
        payload_type: u32,
        window_ns: i64,
    }

    impl AudioTrackBuf {
        fn new(sample_rate: u32, channels: u16, bitrate: u32, window_ns: i64) -> AudioTrackBuf {
            AudioTrackBuf {
                packets: VecDeque::new(),
                sample_rate,
                channels,
                bitrate,
                user_data: Vec::new(),
                payload_type: 0,
                window_ns,
            }
        }

        fn push(&mut self, data: Vec<u8>, time: i64, dur: i64) {
            self.packets.push_back(Packet { data, time, dur, key: false });
            let Some(latest) = self.packets.back().map(|p| p.time) else {
                return;
            };
            let cutoff = latest - self.window_ns;
            while let Some(front) = self.packets.front() {
                if front.time <= cutoff {
                    self.packets.pop_front();
                } else {
                    break;
                }
            }
        }
    }

    // Copia inmutable de una pista de audio del ring buffer, lista para muxear sin
    // mantener el lock de ReplayBuffer mientras se escribe a disco.
    struct AudioMuxTrack {
        packets: Vec<(Vec<u8>, i64, i64)>,
        sample_rate: u32,
        channels: u16,
        bitrate: u32,
        user_data: Vec<u8>,
        payload_type: u32,
    }

    impl From<&AudioTrackBuf> for AudioMuxTrack {
        fn from(t: &AudioTrackBuf) -> AudioMuxTrack {
            AudioMuxTrack {
                packets: t.packets.iter().map(|p| (p.data.clone(), p.time, p.dur)).collect(),
                sample_rate: t.sample_rate,
                channels: t.channels,
                bitrate: t.bitrate,
                user_data: t.user_data.clone(),
                payload_type: t.payload_type,
            }
        }
    }

    struct ReplayBuffer {
        packets: VecDeque<Packet>,
        seq_header: Vec<u8>,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
        window_ns: i64,
        sys_audio: Option<AudioTrackBuf>,
        mic_audio: Option<AudioTrackBuf>,
    }

    impl ReplayBuffer {
        fn new(seconds: u32, width: u32, height: u32, fps: u32, bitrate: u32) -> ReplayBuffer {
            ReplayBuffer {
                packets: VecDeque::new(),
                seq_header: Vec::new(),
                width,
                height,
                fps,
                bitrate,
                window_ns: seconds.max(1) as i64 * 10_000_000,
                sys_audio: None,
                mic_audio: None,
            }
        }

        fn init_audio(
            &mut self,
            sys: Option<(u32, u16)>,
            mic: Option<(u32, u16)>,
        ) {
            // El guardado ancla el clip al IDR anterior al inicio de la ventana (hasta ~1 GOP
            // atrás), así que el audio debe conservar historia hasta ahí; con la misma ventana que
            // el vídeo, su paquete más antiguo cae en el inicio de la ventana y el clip arranca con
            // vídeo pero sin audio. Se le da al audio la ventana de vídeo + un GOP + margen; el
            // exceso lo descarta el muxer (paquetes con time < base). Coste: ~1,5 s extra de AAC.
            let gop_ns = self.fps.max(8) as i64 * 10_000_000 / self.fps.max(1) as i64;
            let audio_window = self.window_ns + gop_ns + 5_000_000;
            if let Some((rate, ch)) = sys {
                self.sys_audio = Some(AudioTrackBuf::new(rate, ch, aac_bitrate(ch), audio_window));
            }
            if let Some((rate, ch)) = mic {
                self.mic_audio = Some(AudioTrackBuf::new(rate, ch, aac_bitrate(ch), audio_window));
            }
        }

        fn track_mut(&mut self, role: AudioRole) -> Option<&mut AudioTrackBuf> {
            match role {
                AudioRole::Sys => self.sys_audio.as_mut(),
                AudioRole::Mic => self.mic_audio.as_mut(),
            }
        }

        fn push_audio(&mut self, role: AudioRole, data: Vec<u8>, time: i64, dur: i64) {
            if let Some(t) = self.track_mut(role) {
                t.push(data, time, dur);
            }
        }

        fn set_user_data(&mut self, role: AudioRole, data: Vec<u8>) {
            if let Some(t) = self.track_mut(role) {
                t.user_data = data;
            }
        }

        fn set_payload_type(&mut self, role: AudioRole, v: u32) {
            if let Some(t) = self.track_mut(role) {
                t.payload_type = v;
            }
        }

        fn push(&mut self, data: Vec<u8>, time: i64, dur: i64, key: bool) {
            self.packets.push_back(Packet { data, time, dur, key });
            self.trim();
        }

        // Mantener acotado el buffer: descartar hasta el último keyframe anterior al
        // inicio de la ventana, para que siempre podamos empezar el MP4 en un IDR.
        fn trim(&mut self) {
            let Some(latest) = self.packets.back().map(|p| p.time) else {
                return;
            };
            let cutoff = latest - self.window_ns;
            let mut anchor = 0usize;
            for (i, p) in self.packets.iter().enumerate() {
                if p.time > cutoff {
                    break;
                }
                if p.key {
                    anchor = i;
                }
            }
            for _ in 0..anchor {
                self.packets.pop_front();
            }
        }
    }

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

    struct ReplayRunning {
        stop: Arc<AtomicBool>,
        handle: Option<JoinHandle<()>>,
        buffer: Arc<Mutex<ReplayBuffer>>,
        out_dir: String,
    }

    static REPLAY_STATE: Mutex<Option<ReplayRunning>> = Mutex::new(None);

    // Textura envuelta para poder cruzar el canal: solo se usa de forma serializada
    // (FrameArrived produce, el hilo de bombeo consume) con el device multihilo-protegido.
    struct SendTex(ID3D11Texture2D);
    unsafe impl Send for SendTex {}

    // Frame BGRA CRUDO que cruza del hilo de pacing (reloj) al worker del encoder. El worker
    // hace el convert BGRA→NV12 + el ProcessInput, así el hilo del reloj no toca la GPU y no se
    // congela cuando el juego satura la GPU. `tex` es una referencia contada a una textura del
    // ring BGRA (vive mientras el worker; scope hace join antes de soltar el pipe). force_key
    // marca el frame que debe abrir un GOP nuevo (cartel↔juego).
    struct SendItem {
        tex: ID3D11Texture2D,
        pts: i64,
        force_key: bool,
        // Contador de escritura del slot del ring del que salió `tex` (u64::MAX = cartel, que no
        // vive en el ring y nunca es stale). El worker lo valida contra FeedCtx.seq antes de usarlo.
        seq: u64,
    }
    unsafe impl Send for SendItem {}

    struct FeedCtx {
        ctx: ID3D11DeviceContext,
        ring: Vec<ID3D11Texture2D>,
        next: AtomicUsize,
        // Contador global de la última escritura en cada slot del ring. El worker lo compara
        // con el que viajó junto al frame: si no coincide, el handler ya sobreescribió ese slot
        // (el ring dio la vuelta mientras el frame esperaba en cola) y el frame es basura. Así
        // se descarta en vez de encodear contenido "del futuro" con un PTS viejo.
        seq: Vec<AtomicU64>,
        tx: mpsc::Sender<(SendTex, i64, u64)>,
    }
    unsafe impl Send for FeedCtx {}
    unsafe impl Sync for FeedCtx {}

    #[allow(clippy::too_many_arguments)]
    pub fn start_replay(
        target: String,
        out_dir: String,
        seconds: u32,
        fps: u32,
        quality: String,
        resolution: u32,
        bitrate: u32,
        mic: bool,
        mic_device: String,
        encoder_pref: String,
        // Se invoca cuando la captura se re-arma contra otra ventana del juego (modo Aplicación),
        // para que la UI vuelva a mostrar el toast "Listo para clipear". Desacopla capture.rs de
        // Tauri: lib.rs pasa un cierre que emite el evento.
        on_retarget: Box<dyn Fn() + Send>,
        // Texto del cartel "fuera de foco", ya localizado por el llamador (capture.rs no conoce
        // el idioma).
        card_text: String,
    ) -> std::result::Result<(), String> {
        let mut guard = REPLAY_STATE.lock().unwrap();
        if guard.is_some() {
            return Ok(());
        }

        let fps = clamp_fps(fps);
        let factor = bitrate_factor(&quality);
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(Stats::default());
        let buffer: Arc<Mutex<ReplayBuffer>> =
            Arc::new(Mutex::new(ReplayBuffer::new(seconds, 0, 0, fps, 0)));
        let (ready_tx, ready_rx) = mpsc::channel::<std::result::Result<(), String>>();

        let stop_t = stop.clone();
        let buf_t = buffer.clone();
        let handle = std::thread::Builder::new()
            .name("flashback-replay".into())
            .spawn(move || {
                replay_thread(
                    target, seconds, fps, factor, resolution, bitrate, mic, mic_device,
                    encoder_pref, stop_t, buf_t, stats, ready_tx, on_retarget, card_text,
                )
            })
            .map_err(|e| e.to_string())?;

        match ready_rx.recv() {
            Ok(Ok(())) => {
                *guard = Some(ReplayRunning {
                    stop,
                    handle: Some(handle),
                    buffer,
                    out_dir,
                });
                Ok(())
            }
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(_) => Err("El hilo de replay terminó inesperadamente".into()),
        }
    }

    pub fn stop_replay() {
        let running = REPLAY_STATE.lock().unwrap().take();
        if let Some(mut r) = running {
            r.stop.store(true, Ordering::SeqCst);
            if let Some(h) = r.handle.take() {
                let _ = h.join();
            }
        }
    }

    pub fn replay_active() -> bool {
        REPLAY_STATE.lock().unwrap().is_some()
    }

    // Muxea los últimos N s del ring a un MP4 desde el último IDR. Se clona lo necesario
    // bajo el lock y se libera antes de tocar disco para no frenar el hilo de codificación.
    pub fn save_replay(source: &str) -> Option<String> {
        let (buffer, out_dir) = {
            let guard = REPLAY_STATE.lock().unwrap();
            let r = guard.as_ref()?;
            (r.buffer.clone(), r.out_dir.clone())
        };

        let (packets, total, seq_header, width, height, fps, bitrate, sys_audio, mic_audio) = {
            let buf = buffer.lock().unwrap();
            let start = buf.packets.iter().position(|p| p.key);
            let pkts: Vec<(Vec<u8>, i64, i64, bool)> = match start {
                Some(s) => buf
                    .packets
                    .iter()
                    .skip(s)
                    .map(|p| (p.data.clone(), p.time, p.dur, p.key))
                    .collect(),
                None => Vec::new(),
            };
            (
                pkts,
                buf.packets.len(),
                buf.seq_header.clone(),
                buf.width,
                buf.height,
                buf.fps,
                buf.bitrate,
                buf.sys_audio.as_ref().map(AudioMuxTrack::from),
                buf.mic_audio.as_ref().map(AudioMuxTrack::from),
            )
        };

        // Sin keyframe en el buffer aún no se puede empezar el MP4 en un IDR.
        if packets.is_empty() {
            if total == 0 {
                eprintln!("save_replay: el ring buffer de vídeo está vacío (el encoder aún no ha producido ningún paquete)");
            } else {
                eprintln!("save_replay: {total} paquetes en el buffer pero ninguno es keyframe todavía");
            }
            return None;
        }

        // El sink MP4 necesita el AudioSpecificConfig (user_data) de cada pista AAC para
        // escribir el `esds`; sin él, Finalize falla con MF_E_SINK_HEADERS_NOT_FOUND. Si una
        // pista no llegó a producir ese config (p. ej. el encoder AAC no pudo con el formato
        // del dispositivo) se omite, y el replay se guarda solo con vídeo en vez de fallar.
        let sys_audio = match sys_audio {
            Some(t) if !t.user_data.is_empty() && !t.packets.is_empty() => Some(t),
            Some(_) => {
                eprintln!("save_replay: pista de sistema omitida (sin config AAC válida)");
                None
            }
            None => None,
        };
        let mic_audio = match mic_audio {
            Some(t) if !t.user_data.is_empty() && !t.packets.is_empty() => Some(t),
            Some(_) => {
                eprintln!("save_replay: pista de micrófono omitida (sin config AAC válida)");
                None
            }
            None => None,
        };

        let path = format!("{out_dir}\\{}", clip_filename());
        // save_replay corre en el hilo de Tauri (STA), pero el sink MP4 con AAC crea
        // componentes de Media Foundation que exigen apartamento MTA (sin él, Finalize
        // falla con "clase no registrada"). Se muxea en un hilo propio MTA, mismo patrón
        // que los hilos de captura.
        let path_t = path.clone();
        let muxed = std::thread::spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            let r = mux_replay(
                &path_t, &packets, &seq_header, width, height, fps, bitrate, sys_audio,
                mic_audio,
            )
            .map_err(|e| format!("{e:?}"));
            unsafe { CoUninitialize() };
            r
        })
        .join();

        match muxed {
            Ok(Ok(())) => {
                if !source.is_empty() {
                    let _ = crate::library::write_embedded_source(std::path::Path::new(&path), source);
                }
                Some(path)
            }
            Ok(Err(e)) => {
                eprintln!("save_replay: fallo al muxear el MP4: {e}");
                // Finalize falló: el archivo a medio escribir quedaría corrupto en la
                // biblioteca, así que lo borramos.
                let _ = std::fs::remove_file(&path);
                None
            }
            Err(_) => {
                eprintln!("save_replay: el hilo de muxado terminó inesperadamente");
                let _ = std::fs::remove_file(&path);
                None
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn replay_thread(
        target: String,
        seconds: u32,
        fps: u32,
        factor: f64,
        resolution: u32,
        bitrate_override: u32,
        mic: bool,
        mic_device: String,
        encoder_pref: String,
        stop: Arc<AtomicBool>,
        buffer: Arc<Mutex<ReplayBuffer>>,
        stats: Arc<Stats>,
        ready: mpsc::Sender<std::result::Result<(), String>>,
        on_retarget: Box<dyn Fn() + Send>,
        card_text: String,
    ) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let _timer = TimerRes::new();
        let _mmcss = MmcssTask::new("Capture");

        let _ = seconds;

        // El instant replay vive en segundo plano. En modo ventana (juego) la ventana puede
        // estar minimizada o aún sin abrir cuando se arma: en vez de fallar, se deja armado y
        // se espera (poll) a que sea capturable, de modo que abrir/restaurar el juego después
        // empieza a bufferear solo, sin reactivar el replay a mano. Si la sesión se corta y no
        // se pidió parar, se reintenta (reconexión). En modo monitor un fallo es definitivo.
        let window_mode = target == "window";
        // El pump escribe a través de VideoPacketSink; el replay usa el ring buffer como sink.
        let video_sink: Arc<dyn VideoPacketSink> = buffer.clone();
        let mut announced = false;
        // El pump anterior salió por retarget: la próxima reconstrucción exitosa apunta a otra
        // ventana del juego, así que al lograrla se avisa a la UI (toast "Listo para clipear").
        let mut retargeting = false;
        loop {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            let built = resolve_target_item(&target).and_then(|item| {
                build_replay(
                    &buffer, &stats, item, fps, factor, resolution, bitrate_override, mic,
                    mic_device.clone(), &encoder_pref, window_mode, &card_text,
                )
                .map_err(|e| format!("{e:?}"))
            });
            match built {
                Ok(pipe) => {
                    if !announced {
                        let _ = ready.send(Ok(()));
                        announced = true;
                    } else if retargeting {
                        on_retarget();
                    }
                    run_pump(&pipe, &stop, &video_sink, window_mode);
                    let lost = pipe.device_lost.load(Ordering::SeqCst);
                    let retargeted = pipe.retarget.load(Ordering::SeqCst);
                    teardown_replay(pipe);
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    // Modo monitor sin pérdida de device: fin normal (hoy el pump solo retorna por
                    // stop, size_changed o device_lost; en monitor no hay resize, así que es
                    // defensivo). Con el device perdido se reconstruye también en modo monitor: el
                    // build siguiente crea un device nuevo y conserva el ring si el tamaño coincide.
                    if !window_mode && !lost {
                        break;
                    }
                    retargeting = retargeted;
                }
                Err(e) => {
                    // Fallo al (re)construir. Si aún no habíamos arrancado es un fallo inicial: en
                    // monitor es definitivo (se reporta y se sale); en ventana se deja armado y se
                    // espera a que el juego sea capturable. Si ya habíamos arrancado (reconstrucción
                    // tras device-lost o reconexión), se reintenta en ambos modos: el device puede
                    // tardar en volver tras un TDR.
                    if !announced {
                        if !window_mode {
                            let _ = ready.send(Err(e));
                            break;
                        }
                        let _ = ready.send(Ok(()));
                        announced = true;
                    }
                }
            }
            if wait_or_stop(&stop, Duration::from_millis(400)) {
                break;
            }
        }

        unsafe { CoUninitialize() };
    }

    // Duerme hasta `dur` en pasos cortos; devuelve true si se pidió parar mientras tanto, para
    // que el bucle de espera del replay reaccione rápido a stop_replay.
    fn wait_or_stop(stop: &Arc<AtomicBool>, dur: Duration) -> bool {
        let step = Duration::from_millis(50);
        let mut waited = Duration::ZERO;
        while waited < dur {
            if stop.load(Ordering::SeqCst) {
                return true;
            }
            std::thread::sleep(step);
            waited += step;
        }
        stop.load(Ordering::SeqCst)
    }

    // Cierra un pipeline de replay: primero las fuentes (WGC + audio) y solo después se suelta
    // el pipeline, para que el flush final del audio vea las colas completas.
    fn teardown_replay(mut pipe: ReplayPipeline) {
        let _ = pipe.frame_pool.RemoveFrameArrived(pipe.token);
        let _ = pipe.session.Close();
        for track in &mut pipe.audio_tracks {
            track.stop();
        }
        let _ = pipe.frame_pool.Close();
        drop(pipe);
    }

    struct ReplayPipeline {
        _device: ID3D11Device,
        _manager: IMFDXGIDeviceManager,
        // fps objetivo: lo usa el bombeo para la cadencia CFR (cfr_pts/fps_interval).
        fps: u32,
        converter: IMFTransform,
        encoder: IMFTransform,
        // None => encoder síncrono por software (no genera eventos): se bombea distinto.
        enc_events: Option<IMFMediaEventGenerator>,
        nv12_pool: Vec<IMFSample>,
        nv12_next: std::cell::Cell<usize>,
        converter_provides: bool,
        // Solo en el camino software: lleva el frame BGRA de GPU a CPU para alimentar
        // los MFT por software (que no leen texturas D3D).
        sw: Option<SwReadback>,
        frame_pool: Direct3D11CaptureFramePool,
        session: GraphicsCaptureSession,
        token: i64,
        rx: mpsc::Receiver<(SendTex, i64, u64)>,
        // Contexto del ring BGRA (lo comparte con el handler): el worker lee `seq` para
        // detectar frames stale (ring reciclado mientras esperaban en cola).
        feed: Arc<FeedCtx>,
        // Tiempo absoluto (WGC) del primer frame de vídeo bombeado: i64::MIN = aún sin
        // establecer. Las pistas de audio lo leen para rebasarse al mismo origen que el
        // vídeo (ver run_pump_async/sync y los AudioSink de más abajo).
        video_base: Arc<AtomicI64>,
        audio_tracks: Vec<audio::TrackHandle>,
        // Cartel "fuera de foco" (solo modo ventana): se compone una vez al minimizar y se
        // codifica en lugar de los frames congelados. `tex` es su lienzo BGRA de salida.
        card: Option<Card>,
        // Ventana del juego (modo Aplicación) para consultar el minimizado con IsIconic, sin
        // recorrer todas las ventanas (EnumWindows) en el hilo de bombeo. 0 = sin ventana.
        game_hwnd: isize,
        // Lo marca el handler de frames cuando la ventana cambia de tamaño: el bombeo sale y el
        // hilo de replay reconstruye el pipeline al nuevo tamaño.
        size_changed: Arc<AtomicBool>,
        // Lo marca el worker del encoder cuando el device D3D se pierde (TDR/reset de GPU): el
        // bombeo sale y el hilo de replay reconstruye el pipeline con un device nuevo (§4.4).
        device_lost: Arc<AtomicBool>,
        // Lo marca FocusState (modo ventana) cuando la ventana rastreada dejó de ser capturable
        // pero el juego tiene ahora OTRA ventana visible distinta: el bombeo sale y el bucle de
        // replay reconstruye la captura contra la nueva ventana (relevo launcher → juego, o
        // recreación de la ventana al alternar fullscreen), en vez de quedarse mostrando el
        // cartel sobre una ventana muerta o capturando la equivocada.
        retarget: Arc<AtomicBool>,
    }
    unsafe impl Send for ReplayPipeline {}
    // Se comparte por referencia entre el hilo de pacing y el worker del encoder (thread::scope
    // en run_pump_async). Es sound porque durante el bombeo cada campo no-Sync lo toca UN solo
    // hilo: el worker usa converter/nv12_pool/nv12_next/sw/encoder/enc_events; el pacing usa rx.
    // Los campos compartidos (video_base, size_changed) son atómicos; feed.seq se lee/escribe
    // por atómicos entre worker y handler. Ver run_pump_async.
    unsafe impl Sync for ReplayPipeline {}

    struct Card {
        overlay: crate::overlay::OutOfFocusCard,
        tex: ID3D11Texture2D,
    }

    struct SwReadback {
        ctx: ID3D11DeviceContext,
        staging: ID3D11Texture2D,
        width: u32,
        height: u32,
    }

    // Núcleo del pipeline de codificación compartido por el Instant Replay y la grabación
    // manual: WGC + conversor + encoder + bombeo. La diferencia entre modos es solo el destino
    // de los paquetes (ring buffer vs muxer en directo) y las pistas de audio, que cablea cada
    // llamador. audio_tracks vuelve vacío; el llamador lo rellena.
    struct PipelineCore {
        pipe: ReplayPipeline,
        out_w: u32,
        out_h: u32,
        bitrate: u32,
        video_base: Arc<AtomicI64>,
    }

    #[allow(clippy::too_many_arguments)]
    fn build_pipeline_core(
        stats: &Arc<Stats>,
        item: GraphicsCaptureItem,
        fps: u32,
        factor: f64,
        resolution: u32,
        bitrate_override: u32,
        encoder_pref: &str,
        window_mode: bool,
        card_text: Option<&str>,
    ) -> Result<PipelineCore> {
        ensure_mf();
        let (device, d3d_device) = create_device()?;
        // NV12/H.264 exigen dimensiones PARES. La ventana de un juego puede tener tamaño
        // impar (a diferencia de un monitor), lo que daba MF_E_INVALIDMEDIATYPE. Se redondea
        // a par hacia abajo y el frame pool se crea a ese tamaño (recorta ≤1px, imperceptible).
        let mut size = item.Size()?;
        size.Width = size.Width.max(2) & !1;
        size.Height = size.Height.max(2) & !1;
        let width = size.Width as u32;
        let height = size.Height as u32;
        // Captura a nativo (width/height); el conversor escala al objetivo (out_*), que es
        // la resolución codificada y guardada. El bitrate se calcula sobre la salida.
        let (out_w, out_h) = output_dims(width, height, resolution);
        let bitrate = resolve_bitrate(out_w, out_h, fps, factor, bitrate_override);

        // Device manager compartido (zero-copy GPU) y device protegido para multihilo.
        let mut token = 0u32;
        let mut manager: Option<IMFDXGIDeviceManager> = None;
        unsafe { MFCreateDXGIDeviceManager(&mut token, &mut manager)? };
        let manager = manager.ok_or_else(null_out)?;
        unsafe { manager.ResetDevice(&device, token)? };
        let ctx = unsafe { device.GetImmediateContext()? };
        if let Ok(mt) = ctx.cast::<ID3D11Multithread>() {
            let _ = unsafe { mt.SetMultithreadProtected(true) };
        }

        // Encoder primero: si no hay H.264 por hardware (o se fuerza), cae a software
        // (MFT síncrono, codifica en CPU). El resto del pipeline se adapta a ese modo.
        // El encoder trabaja ya en la resolución de salida (out_*).
        let (encoder, enc_events) = build_encoder(&manager, out_w, out_h, fps, bitrate, encoder_pref)?;
        let software = enc_events.is_none();

        // El conversor BGRA→NV12 escala de captura (width/height) a salida (out_*): va en
        // GPU con el encoder por hardware, y en software (sin device manager, CPU) si no.
        let converter = build_converter(
            if software { None } else { Some(&manager) },
            width,
            height,
            out_w,
            out_h,
            fps,
        )?;

        // Pool de salidas NV12 (a resolución de salida) para el conversor si no las provee
        // él: GPU en el camino hardware, memoria de sistema en el software.
        let converter_provides = unsafe {
            let info = converter.GetOutputStreamInfo(0)?;
            info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32) != 0
        };
        // El worker convierte de a un frame por vez y lo entrega al encoder; el buffer que
        // absorbe los stalls es el ring BGRA (más abajo), no este pool. Basta con cubrir los
        // frames que retiene NVENC en vuelo + margen del round-robin.
        let nv12_pool = if converter_provides {
            Vec::new()
        } else if software {
            create_nv12_cpu_samples(out_w, out_h, 16)?
        } else {
            create_nv12_samples(&device, out_w, out_h, 12)?
        };

        // Readback GPU→CPU solo en software: textura de staging + un handle al contexto
        // inmediato (compartido y protegido para multihilo) para volcar cada frame BGRA.
        let sw = if software {
            Some(SwReadback {
                ctx: unsafe { device.GetImmediateContext()? },
                staging: create_staging_bgra(&device, width, height)?,
                width,
                height,
            })
        } else {
            None
        };

        // Anillo BGRA para FrameArrived: copia GPU→GPU y manda al hilo de bombeo. En hardware
        // es además el buffer que absorbe los stalls de GPU: el pacing encola referencias a
        // estas texturas hacia el worker, así que debe caber ~cap frames en vuelo + latest +
        // margen (si no, el handler sobreescribiría una textura aún encolada). El software no
        // desacopla (bombeo síncrono), 10 basta.
        let bgra_ring_len = if software { 10 } else { enc_buffer_frames(fps) + 12 };
        let bgra_ring = create_bgra_textures(&device, width, height, bgra_ring_len)?;
        let (tx, rx) = mpsc::channel::<(SendTex, i64, u64)>();
        // seq[i] = u64::MAX hasta la primera escritura; luego el contador global de esa escritura.
        let seq = (0..bgra_ring_len).map(|_| AtomicU64::new(u64::MAX)).collect();
        let feed = Arc::new(FeedCtx {
            ctx,
            ring: bgra_ring,
            next: AtomicUsize::new(0),
            seq,
            tx,
        });

        let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &d3d_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )?;
        let session = frame_pool.CreateCaptureSession(&item)?;
        let _ = session.SetIsBorderRequired(false);
        set_capture_rate(&session, fps);

        // Límite de FPS: descartar frames antes de la copia GPU y del canal para no
        // codificar de más (clave para los perfiles ligeros tipo 480p/20 FPS).
        let interval = fps_interval(fps);
        // El frame pool se crea a un tamaño fijo (width/height). Si la ventana crece (p. ej. un
        // juego que pasa a fullscreen/borderless tras armarse el replay), WGC recorta a la
        // esquina superior izquierda de ese tamaño viejo. Al detectar el cambio se marca y el
        // bombeo sale para que el hilo reconstruya el pipeline al nuevo tamaño. Un monitor no
        // cambia de tamaño, así que esto solo afecta a la captura por ventana.
        let size_changed = Arc::new(AtomicBool::new(false));
        let selector = Arc::new(SlotSelector::new());
        let stats = stats.clone();
        let feed_h = feed.clone();
        let size_changed_h = size_changed.clone();
        let handler = TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(
            move |pool, _| {
                ensure_handler_priority();
                if let Some(pool) = pool.as_ref() {
                    if let Ok(frame) = pool.TryGetNextFrame() {
                        if let Ok(s) = frame.ContentSize() {
                            stats.width.store(s.Width.max(0) as u32, Ordering::Relaxed);
                            stats.height.store(s.Height.max(0) as u32, Ordering::Relaxed);
                            let cw = (s.Width.max(0) as u32) & !1;
                            let ch = (s.Height.max(0) as u32) & !1;
                            if cw >= 2 && ch >= 2 && (cw != width || ch != height) {
                                size_changed_h.store(true, Ordering::Relaxed);
                                let _ = frame.Close();
                                return Ok(());
                            }
                        }
                        let t = frame.SystemRelativeTime().map(|x| x.Duration).unwrap_or(0);
                        if selector.keep(t, interval) {
                            if let Ok(surface) = frame.Surface() {
                                if let Ok(access) = surface.cast::<IDirect3DDxgiInterfaceAccess>() {
                                    if let Ok(tex) =
                                        unsafe { access.GetInterface::<ID3D11Texture2D>() }
                                    {
                                        let counter =
                                            feed_h.next.fetch_add(1, Ordering::Relaxed) as u64;
                                        let idx = counter as usize % feed_h.ring.len();
                                        let dst = feed_h.ring[idx].clone();
                                        unsafe { feed_h.ctx.CopyResource(&dst, &tex) };
                                        // Publica el contador DESPUÉS de la copia: el worker que
                                        // vea este seq sabe que el contenido ya está escrito.
                                        feed_h.seq[idx].store(counter, Ordering::Release);
                                        let _ = feed_h.tx.send((SendTex(dst), t, counter));
                                    }
                                }
                            }
                            stats.frames.fetch_add(1, Ordering::Relaxed);
                        }
                        let _ = frame.Close();
                    }
                }
                Ok(())
            },
        );
        let token = frame_pool.FrameArrived(&handler)?;

        // Arrancar los MFT en streaming y la sesión WGC. START_OF_STREAM solo aplica al
        // MFT asíncrono (hardware); el síncrono por software no lo necesita.
        unsafe {
            converter.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            encoder.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            if enc_events.is_some() {
                encoder.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
            }
        }

        // Origen temporal del vídeo (escala WGC): lo fija el bombeo al emitir el primer frame.
        // Las pistas de audio (que cablea el llamador) se rebasan contra él para compartir el cero.
        let video_base = Arc::new(AtomicI64::new(i64::MIN));

        // Cartel "fuera de foco": solo en modo ventana con texto (el Instant Replay lo usa; la
        // grabación manual pasa None). Si Direct2D falla, se sigue sin cartel.
        let card = if let (true, Some(text)) = (window_mode, card_text) {
            match crate::overlay::OutOfFocusCard::new(&device, width, height, text) {
                Ok(overlay) => {
                    let tex = create_bgra_textures(&device, width, height, 1)?
                        .into_iter()
                        .next()
                        .ok_or_else(null_out)?;
                    Some(Card { overlay, tex })
                }
                Err(e) => {
                    eprintln!("overlay: no se pudo crear el cartel de fuera de foco: {e:?}");
                    None
                }
            }
        } else {
            None
        };
        // Se resuelve una sola vez aquí (fuera del hilo de bombeo): la ventana principal del
        // juego es estable mientras exista; minimizar/restaurar no cambia su HWND.
        let game_hwnd = if window_mode {
            resolve_game_window().map(|h| h.0 as isize).unwrap_or(0)
        } else {
            0
        };

        session.StartCapture()?;

        let pipe = ReplayPipeline {
            _device: device,
            _manager: manager,
            fps,
            converter,
            encoder,
            enc_events,
            nv12_pool,
            nv12_next: std::cell::Cell::new(0),
            converter_provides,
            sw,
            frame_pool,
            session,
            token,
            rx,
            feed,
            video_base: video_base.clone(),
            audio_tracks: Vec::new(),
            card,
            game_hwnd,
            size_changed,
            device_lost: Arc::new(AtomicBool::new(false)),
            retarget: Arc::new(AtomicBool::new(false)),
        };
        Ok(PipelineCore { pipe, out_w, out_h, bitrate, video_base })
    }

    // Instant Replay: monta el núcleo del pipeline hacia el ring buffer y cablea las pistas de
    // audio con ReplayAudioSink (AAC directo al ring, rebasado contra video_base).
    #[allow(clippy::too_many_arguments)]
    fn build_replay(
        buffer: &Arc<Mutex<ReplayBuffer>>,
        stats: &Arc<Stats>,
        item: GraphicsCaptureItem,
        fps: u32,
        factor: f64,
        resolution: u32,
        bitrate_override: u32,
        mic: bool,
        mic_device: String,
        encoder_pref: &str,
        window_mode: bool,
        card_text: &str,
    ) -> Result<ReplayPipeline> {
        // Sistema: loopback siempre; micrófono solo si el toggle está activo y hay dispositivo.
        let sys_native = audio::probe_format(&audio::TrackKind::SystemLoopback);
        let mic_native = if mic && !mic_device.is_empty() {
            let f = audio::probe_format(&audio::TrackKind::Microphone(mic_device.clone()));
            if f.is_none() {
                eprintln!("audio: no se pudo abrir el micrófono (device='{mic_device}')");
            }
            f
        } else {
            if mic {
                eprintln!("audio: micrófono activado pero sin dispositivo seleccionado");
            }
            None
        };
        let sys_target = sys_native.and_then(|(r, c)| audio::aac_target_format(r, c));
        let mic_target = mic_native.and_then(|(r, c)| audio::aac_target_format(r, c));

        let core = build_pipeline_core(
            stats, item, fps, factor, resolution, bitrate_override, encoder_pref, window_mode,
            Some(card_text),
        )?;
        let PipelineCore { mut pipe, out_w, out_h, bitrate, video_base } = core;

        {
            let mut b = buffer.lock().unwrap();
            // Si la ventana cambió de tamaño (rebuild por resize), los paquetes ya en el ring
            // tienen otra resolución/SPS y no se pueden muxear junto a los nuevos: se descartan.
            if b.width != out_w || b.height != out_h {
                b.packets.clear();
                b.seq_header.clear();
            }
            b.width = out_w;
            b.height = out_h;
            b.fps = fps;
            b.bitrate = bitrate;
            b.init_audio(sys_target, mic_target);
        }

        let mut audio_tracks = Vec::new();
        if let (Some((rate, ch)), Some((_, dst_ch))) = (sys_native, sys_target) {
            let sink = Arc::new(ReplayAudioSink {
                buffer: buffer.clone(),
                video_base: video_base.clone(),
                role: AudioRole::Sys,
            });
            audio_tracks.push(audio::spawn_track(
                audio::TrackKind::SystemLoopback,
                audio::Encoding::Aac(aac_bitrate(dst_ch)),
                rate,
                ch,
                sink,
                None,
            ));
        }
        if let (Some((rate, ch)), Some((_, dst_ch))) = (mic_native, mic_target) {
            let sink = Arc::new(ReplayAudioSink {
                buffer: buffer.clone(),
                video_base: video_base.clone(),
                role: AudioRole::Mic,
            });
            audio_tracks.push(audio::spawn_track(
                audio::TrackKind::Microphone(mic_device.clone()),
                audio::Encoding::Aac(aac_bitrate(dst_ch)),
                rate,
                ch,
                sink,
                None,
            ));
        }
        pipe.audio_tracks = audio_tracks;
        Ok(pipe)
    }

    // true si el device D3D se perdió (TDR, reset de GPU, driver reiniciado). Se consulta solo
    // cuando el convert/encode ya falló, para distinguir un fallo transitorio de la pérdida real
    // del dispositivo y disparar la reconstrucción del pipeline.
    fn device_removed(device: &ID3D11Device) -> bool {
        unsafe { device.GetDeviceRemovedReason().is_err() }
    }

    // Out-param nulo tras un Create* que devolvió S_OK (driver defectuoso): se propaga como error
    // recuperable en vez de romper el hilo con un panic. En el replay dispara la reconstrucción.
    fn null_out() -> windows::core::Error {
        windows::core::Error::from_hresult(E_POINTER)
    }

    // Conversor de color + escalado: entrada ARGB32 a resolución de captura (in_*),
    // salida NV12 a resolución objetivo (out_*). Si difieren, el Video Processor escala.
    fn build_converter(
        manager: Option<&IMFDXGIDeviceManager>,
        in_w: u32,
        in_h: u32,
        out_w: u32,
        out_h: u32,
        fps: u32,
    ) -> Result<IMFTransform> {
        let converter: IMFTransform =
            unsafe { CoCreateInstance(&CLSID_VIDEO_PROCESSOR_MFT, None, CLSCTX_INPROC_SERVER)? };
        // Con device manager convierte en GPU (zero-copy); sin él, en CPU (camino software).
        if let Some(manager) = manager {
            unsafe {
                let unk: windows::core::IUnknown = manager.cast()?;
                converter.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, unk.as_raw() as usize)?;
            }
        }

        let out_type = unsafe { MFCreateMediaType()? };
        unsafe {
            out_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            out_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
            out_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            out_type.SetUINT64(&MF_MT_FRAME_SIZE, pack2(out_w, out_h))?;
            out_type.SetUINT64(&MF_MT_FRAME_RATE, pack2(fps, 1))?;
            out_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack2(1, 1))?;
        }

        let in_type = unsafe { MFCreateMediaType()? };
        unsafe {
            in_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            in_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_ARGB32)?;
            in_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            in_type.SetUINT64(&MF_MT_FRAME_SIZE, pack2(in_w, in_h))?;
            in_type.SetUINT64(&MF_MT_FRAME_RATE, pack2(fps, 1))?;
            in_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack2(1, 1))?;
        }

        unsafe {
            converter.SetInputType(0, &in_type, 0)?;
            converter.SetOutputType(0, &out_type, 0)?;
        }
        Ok(converter)
    }

    // Selecciona el encoder H.264: hardware (MFT asíncrono, zero-copy GPU) si lo hay, o
    // software (CPU) como fallback. El HW MFT se usa en su modo async nativo (eventos
    // NEED_INPUT/HAVE_OUTPUT) con un device D3D11 dedicado para no competir con WGC.
    // encoder_pref: "Auto"|"NVENC"|"AMF"|"Quick Sync"|"Software"
    fn build_encoder(
        manager: &IMFDXGIDeviceManager,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
        encoder_pref: &str,
    ) -> Result<(IMFTransform, Option<IMFMediaEventGenerator>)> {
        let force_sw = std::env::var_os("FLASHBACK_FORCE_SW_ENCODER").is_some()
            || encoder_pref == "Software";

        if !force_sw {
            if let Some(activate) = pick_hw_encoder(encoder_pref)? {
                let encoder: IMFTransform = unsafe { activate.ActivateObject()? };
                unsafe {
                    let attrs = encoder.GetAttributes()?;
                    attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)?;
                    let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);
                    let unk: windows::core::IUnknown = manager.cast()?;
                    encoder.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, unk.as_raw() as usize)?;
                }
                configure_encoder_types(&encoder, width, height, fps, bitrate)?;
                let events: IMFMediaEventGenerator = encoder.cast()?;
                return Ok((encoder, Some(events)));
            }
        }

        // Fallback por software: MFT síncrono, sin device manager (codifica en CPU).
        let activate = enum_encoder(MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_TRANSCODE_ONLY)?
            .ok_or_else(|| windows::core::Error::from_hresult(MF_E_TOPO_CODEC_NOT_FOUND))?;
        let encoder: IMFTransform = unsafe { activate.ActivateObject()? };
        configure_encoder_types(&encoder, width, height, fps, bitrate)?;
        Ok((encoder, None))
    }

    // Enumera todos los encoders H.264 por hardware y devuelve el que coincide con la
    // preferencia de vendor. "Auto" devuelve el primero (MF ya los ordena por calidad).
    // Si la preferencia no coincide con ningún encoder disponible, devuelve el primero
    // como fallback en lugar de fallar.
    fn pick_hw_encoder(pref: &str) -> Result<Option<IMFActivate>> {
        let info = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_H264,
        };
        let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        unsafe {
            MFTEnumEx(
                MFT_CATEGORY_VIDEO_ENCODER,
                MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER,
                None,
                Some(&info),
                &mut activates,
                &mut count,
            )?;
        }
        if count == 0 || activates.is_null() {
            return Ok(None);
        }

        let keywords: &[&str] = match pref.to_lowercase().as_str() {
            "nvenc" => &["nvenc", "nvidia"],
            "amf" => &["amf", "amd"],
            "quick sync" => &["quick sync", "intel"],
            _ => &[],
        };

        let mut chosen: Option<IMFActivate> = None;
        for i in 0..count as usize {
            let act = unsafe { &*activates.add(i) };
            if let Some(act) = act {
                if chosen.is_none() {
                    // Siempre guardamos el primero como fallback.
                    chosen = Some(act.clone());
                }
                if !keywords.is_empty() && encoder_name_matches(act, keywords) {
                    chosen = Some(act.clone());
                    break;
                }
            }
        }

        // Liberar el array de activates: drop_in_place decrementa el refcount de cada uno.
        for i in 0..count as usize {
            unsafe { std::ptr::drop_in_place(activates.add(i)) };
        }
        unsafe { CoTaskMemFree(Some(activates as *const _)) };

        Ok(chosen)
    }

    fn encoder_name_matches(activate: &IMFActivate, keywords: &[&str]) -> bool {
        let attrs: IMFAttributes = match activate.cast() {
            Ok(a) => a,
            Err(_) => return false,
        };
        let mut buf = [0u16; 512];
        let mut len = 0u32;
        if unsafe { attrs.GetString(&MFT_FRIENDLY_NAME_Attribute, &mut buf, Some(&mut len)) }
            .is_err()
        {
            return false;
        }
        let name = String::from_utf16_lossy(&buf[..len as usize]).to_lowercase();
        keywords.iter().any(|k| name.contains(k))
    }

    // Primer encoder H.264 que cumple los flags, o None si no hay ninguno.
    fn enum_encoder(flags: MFT_ENUM_FLAG) -> Result<Option<IMFActivate>> {
        let info = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_H264,
        };
        let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        unsafe {
            MFTEnumEx(
                MFT_CATEGORY_VIDEO_ENCODER,
                flags | MFT_ENUM_FLAG_SORTANDFILTER,
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
        // Soltar y liberar el array de activates.
        for i in 0..count as usize {
            unsafe {
                let _ = std::ptr::read(activates.add(i));
            }
        }
        unsafe { CoTaskMemFree(Some(activates as *const _)) };
        Ok(first)
    }

    // Tipos de medio del encoder (output H.264 antes que input NV12, como exigen) + IDR
    // periódico. Común a hardware y software.
    fn configure_encoder_types(
        encoder: &IMFTransform,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
    ) -> Result<()> {
        // El modo de rate control (VBR) debe fijarse ANTES de SetOutputType; si se pone
        // después, el encoder H.264 de MF lo ignora y se queda en CBR. GOP ~1 s (antes 0.5 s):
        // menos IDRs caros = mejor calidad, sin perder mucha granularidad para alinear el
        // guardado del replay al keyframe anterior.
        let gop = fps.max(8);
        let codec = encoder.cast::<ICodecAPI>().ok();
        let quality_failed = codec.as_ref().map(|c| set_quality_codec_settings(c, bitrate, gop));

        let out_type = unsafe { MFCreateMediaType()? };
        unsafe {
            out_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            out_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
            out_type.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
            out_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            // Perfil High (100) + CABAC (lo activa apply_quality_codec_settings). Sin
            // B-frames (DefaultBPictureCount=0): el perfil High sin reordenado conserva el
            // orden salida=entrada, del que depende el emparejado FIFO de timestamps (el que
            // escupe el MFT es poco fiable: a veces sale a 0 y el MP4 queda con duración nula).
            out_type.SetUINT32(&MF_MT_MPEG2_PROFILE, 100)?;
            out_type.SetUINT64(&MF_MT_FRAME_SIZE, pack2(width, height))?;
            out_type.SetUINT64(&MF_MT_FRAME_RATE, pack2(fps, 1))?;
            out_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack2(1, 1))?;
            encoder.SetOutputType(0, &out_type, 0)?;
        }

        let in_type = unsafe { MFCreateMediaType()? };
        unsafe {
            in_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            in_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
            in_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            in_type.SetUINT64(&MF_MT_FRAME_SIZE, pack2(width, height))?;
            in_type.SetUINT64(&MF_MT_FRAME_RATE, pack2(fps, 1))?;
            in_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack2(1, 1))?;
            encoder.SetInputType(0, &in_type, 0)?;
        }

        if let (Some(codec), Some(failed)) = (&codec, &quality_failed) {
            log_encoder_quality(codec, "replay", bitrate, gop, failed);
        }
        Ok(())
    }

    // Valor válido de bytes/seg del encoder AAC de Media Foundation (admite 12000, 16000,
    // 20000 y 24000 = 96/128/160/192 kbps). Fuera de esa lista el encoder rechaza el tipo.
    fn aac_bitrate(channels: u16) -> u32 {
        if channels <= 1 {
            96_000
        } else {
            128_000
        }
    }

    // Declara un stream de audio en el SinkWriter: entrada PCM16 cruda, salida AAC. El
    // SinkWriter resuelve su propio MFT AAC a partir de estos tipos, igual que ya hace
    // con el H.264 de vídeo (ver Encoder::new). El encoder selecciona por
    // MF_MT_AUDIO_AVG_BYTES_PER_SECOND (no por MF_MT_AVG_BITRATE), de ahí ese atributo.
    fn add_aac_stream(writer: &IMFSinkWriter, sample_rate: u32, channels: u16) -> Result<u32> {
        let out_type = unsafe { MFCreateMediaType()? };
        unsafe {
            out_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            out_type.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
            out_type.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, sample_rate)?;
            out_type.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels as u32)?;
            out_type.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            out_type.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, aac_bitrate(channels) / 8)?;
            out_type.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)?;
        }
        let stream = unsafe { writer.AddStream(&out_type)? };

        let in_type = unsafe { MFCreateMediaType()? };
        unsafe {
            in_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            in_type.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
            in_type.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, sample_rate)?;
            in_type.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels as u32)?;
            in_type.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            in_type.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, channels as u32 * 2)?;
            in_type.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, sample_rate * channels as u32 * 2)?;
            writer.SetInputMediaType(stream, &in_type, None)?;
        }
        Ok(stream)
    }

    // Sink que envuelve el Arc<Mutex<Encoder>> de grabación manual: el mismo mutex que
    // ya serializa el push de vídeo serializa también el de audio.
    struct EncoderAudioSink {
        encoder: Arc<Mutex<Encoder>>,
        stream: u32,
    }

    impl audio::AudioSink for EncoderAudioSink {
        fn push(&self, data: Vec<u8>, time: i64, dur: i64) {
            self.encoder.lock().unwrap().push_audio(self.stream, data, time, dur);
        }
    }

    // Sink de audio del Instant Replay: empuja directamente al ring buffer (ya en AAC,
    // sin pasar por ningún SinkWriter). Se rebasa contra `video_base` para compartir
    // origen temporal con los paquetes de vídeo; mientras el vídeo no haya arrancado
    // (i64::MIN) se descartan los paquetes, ya que no hay forma fiable de alinearlos.
    #[derive(Clone, Copy)]
    enum AudioRole {
        Sys,
        Mic,
    }

    struct ReplayAudioSink {
        buffer: Arc<Mutex<ReplayBuffer>>,
        video_base: Arc<AtomicI64>,
        role: AudioRole,
    }

    impl audio::AudioSink for ReplayAudioSink {
        fn push(&self, data: Vec<u8>, time: i64, dur: i64) {
            let base = self.video_base.load(Ordering::SeqCst);
            if base == i64::MIN {
                return;
            }
            let ts = (time - base).max(0);
            self.buffer.lock().unwrap().push_audio(self.role, data, ts, dur);
        }

        fn set_user_data(&self, data: Vec<u8>) {
            self.buffer.lock().unwrap().set_user_data(self.role, data);
        }

        fn set_payload_type(&self, v: u32) {
            self.buffer.lock().unwrap().set_payload_type(self.role, v);
        }
    }

    // Sink de audio de la grabación manual: entrega los paquetes AAC (ya codificados por
    // build_aac_encoder, igual que el replay) al muxer en directo. Rebasa contra video_base
    // como ReplayAudioSink —el PTS de vídeo es CFR sintético con origen en video_base—, así el
    // audio comparte el cero con el vídeo. La cabecera del AAC (AudioSpecificConfig + payload
    // type) llega por set_user_data/set_payload_type antes del primer paquete y se reenvía una vez.
    struct MuxAudioSink {
        mux: Arc<LiveMux>,
        role: AudioRole,
        video_base: Arc<AtomicI64>,
        user_data: Mutex<Vec<u8>>,
        payload_type: AtomicU32,
        header_sent: AtomicBool,
    }

    impl MuxAudioSink {
        fn new(mux: Arc<LiveMux>, role: AudioRole, video_base: Arc<AtomicI64>) -> MuxAudioSink {
            MuxAudioSink {
                mux,
                role,
                video_base,
                user_data: Mutex::new(Vec::new()),
                payload_type: AtomicU32::new(0),
                header_sent: AtomicBool::new(false),
            }
        }

        fn maybe_send_header(&self) {
            if self.header_sent.load(Ordering::SeqCst) {
                return;
            }
            let ud = self.user_data.lock().unwrap().clone();
            if ud.is_empty() {
                return;
            }
            self.mux
                .set_audio_header(self.role, ud, self.payload_type.load(Ordering::SeqCst));
            self.header_sent.store(true, Ordering::SeqCst);
        }
    }

    impl audio::AudioSink for MuxAudioSink {
        fn push(&self, data: Vec<u8>, time: i64, dur: i64) {
            self.maybe_send_header();
            let base = self.video_base.load(Ordering::SeqCst);
            if base == i64::MIN {
                return;
            }
            let ts = (time - base).max(0);
            self.mux.push_audio(self.role, data, ts, dur);
        }

        fn set_user_data(&self, data: Vec<u8>) {
            *self.user_data.lock().unwrap() = data;
            self.maybe_send_header();
        }

        fn set_payload_type(&self, v: u32) {
            self.payload_type.store(v, Ordering::SeqCst);
        }
    }

    fn run_pump(
        pipe: &ReplayPipeline,
        stop: &Arc<AtomicBool>,
        sink: &Arc<dyn VideoPacketSink>,
        window_mode: bool,
    ) {
        if pipe.enc_events.is_some() {
            run_pump_async(pipe, stop, sink, window_mode);
        } else {
            run_pump_sync(pipe, stop, sink, window_mode);
        }
    }

    // Slots de cadencia transcurridos desde `start` (cada slot dura `interval` en 100 ns).
    fn elapsed_slots(start: Instant, interval: i64) -> i64 {
        let hns = (Instant::now().saturating_duration_since(start).as_nanos() as i64) / 100;
        hns / interval.max(1)
    }

    // Slot de cadencia al que pertenece un frame según su timestamp WGC real (SystemRelativeTime,
    // en 100 ns; mismo reloj QPC que Instant, así que no hay deriva de tasa). Redondeo al más
    // cercano: fija cada frame a su slot por su TIEMPO de presentación, no por cuándo despierta el
    // pump. Eso elimina el batido de muestreo (dup+drop periódico) que producía los micro-tirones.
    // `base` = timestamp del primer frame emitido (slot 0).
    fn time_slot(time: i64, base: i64, interval: i64) -> i64 {
        let d = time - base;
        if d <= 0 {
            0
        } else {
            (d + interval / 2) / interval.max(1)
        }
    }

    // Umbral de "stall real": si el reloj de pared se adelanta este nº de slots respecto al último
    // frame recibido, la pantalla está congelada (WGC no entrega frames en escenas estáticas) y
    // rellenamos con duplicados para no vaciar el ring. Por debajo, la cadencia la marca solo el
    // timestamp del frame. Se deja holgura (>2) para no interferir con fuentes a menos fps que el
    // clip, cuyo hueco natural entre frames es de ~2 slots (p. ej. juego a 30 en clip de 60).
    const STALL_SLOTS: i64 = 3;

    // Espera de cadencia: por debajo de un periodo de frame para no quemar CPU ni perder
    // slots. Con timeBeginPeriod(1) activo el sleep es preciso a ~1 ms.
    fn pace_sleep(interval: i64) -> Duration {
        let ms = (interval / 20_000).clamp(1, 8);
        Duration::from_millis(ms as u64)
    }

    // Alimenta una muestra NV12 al encoder asíncrono, drenando su salida y reintentando si
    // rechaza la entrada (NOTACCEPTING). Devuelve true si el encoder la aceptó.
    fn feed_encoder_async(
        enc: &IMFTransform,
        nv12: &IMFSample,
        sink: &Arc<dyn VideoPacketSink>,
        seq_grabbed: &mut bool,
        pts_fifo: &mut VecDeque<i64>,
    ) -> bool {
        for _ in 0..64 {
            match unsafe { enc.ProcessInput(0, nv12, 0) } {
                Ok(()) => return true,
                Err(e) if e.code() == MF_E_NOTACCEPTING => {
                    let n = drain_encoder_output(enc, sink, seq_grabbed, pts_fifo).unwrap_or(0);
                    if n == 0 {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
                Err(_) => return false,
            }
        }
        false
    }

    // Bombeo del encoder por hardware (MFT asíncrono), DESACOPLADO en dos hilos para que la
    // GPU de un juego a pantalla completa no congele la cadencia del clip:
    //
    //  - Hilo de pacing (este): drena WGC para mantener el "último frame BGRA", lleva la
    //    cadencia CFR dirigida por reloj y ENVÍA la referencia BGRA al worker por un canal.
    //    NO toca la GPU (ni convert ni encode), así que ningún stall de GPU lo congela.
    //  - Hilo worker (run_encoder_thread): hace el convert BGRA→NV12 y el ProcessInput (ambos
    //    trabajos de GPU, en un solo hilo para no competir por el device entre sí), y drena la
    //    salida al ring. Bajo carga el ProcessInput se bloquea cientos de ms: se absorbe como
    //    latencia en la cola (buffer BGRA), no como bajón de fps.
    //
    // `inflight` acota la cola a ~300 ms de frames: si el worker se atrasa más, el pacing
    // descarta el slot avanzando el reloj (hueco de 1 frame, degradación suave). El PTS se
    // asigna aquí, contra el reloj de pared, de modo que el vídeo sigue sincronizado con el
    // audio aunque el worker emita con retardo.
    fn run_pump_async(
        pipe: &ReplayPipeline,
        stop: &Arc<AtomicBool>,
        sink: &Arc<dyn VideoPacketSink>,
        window_mode: bool,
    ) {
        let fps = pipe.fps;
        let interval = fps_interval(fps).max(1);
        let cap = enc_buffer_frames(fps);

        // Canal pacing→worker (lleva frames BGRA crudos) y contador en vuelo (backpressure).
        let (tx, rx) = mpsc::channel::<SendItem>();
        let inflight = Arc::new(AtomicUsize::new(0));

        // thread::scope: el worker toma prestado &pipe (Sync) y se une al cerrar el scope, así
        // las texturas BGRA en vuelo (del ring de pipe) siguen vivas mientras el worker las usa.
        std::thread::scope(|s| {
            let inflight_w = inflight.clone();
            s.spawn(move || run_encoder_thread(pipe, sink, stop, rx, inflight_w));

            // Seguimiento de minimizado y composición del cartel "fuera de foco" (ver FocusState).
            let mut foc = FocusState::new();
            // Última textura BGRA real de WGC (y el contador de su slot en el ring, para que el
            // worker detecte si se recicló); se duplica en los slots sin frame nuevo.
            let mut latest: Option<ID3D11Texture2D> = None;
            let mut latest_seq: u64 = u64::MAX;
            // Cadencia: t0_time (timestamp WGC del primer frame) ancla la rejilla CFR al reloj real
            // del juego; t0 (Instant) es solo la red para congelados. `emitted` es el último slot
            // emitido y `emitted_pts` su PTS (monotonía en la frontera con el cartel).
            let mut t0_time: Option<i64> = None;
            let mut t0: Option<Instant> = None;
            let mut emitted: i64 = -1;
            let mut emitted_pts: i64 = -1;
            // Petición de keyframe pendiente: viaja con el próximo frame ENVIADO (no con uno
            // descartado), para que la transición cartel↔juego abra siempre un GOP nuevo.
            let mut pending_key = false;

            while !stop.load(Ordering::SeqCst)
            && !pipe.size_changed.load(Ordering::Relaxed)
            && !pipe.device_lost.load(Ordering::Relaxed)
            && !pipe.retarget.load(Ordering::Relaxed)
        {
                foc.poll(window_mode, pipe);

                // Drenar la cola sin bloquear: solo nos quedamos con el frame más reciente; el
                // resto se descarta porque estamos remuestreando a una cadencia fija.
                loop {
                    match pipe.rx.try_recv() {
                        Ok(f) => {
                            foc.note_real_frame(&f.0 .0, f.1);
                            if !foc.minimized {
                                latest = Some(f.0 .0);
                                latest_seq = f.2;
                            }
                        }
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => break,
                    }
                }

                if foc.take_force_key() {
                    pending_key = true;
                }

                if foc.minimized {
                    // Suspender la cadencia CFR: el puntero de slot avanza con el reloj pero no
                    // codificamos duplicados (evita un golpe al restaurar). El cartel cubre el
                    // tramo a baja cadencia, anclando su PTS a la rejilla de slots.
                    if let Some(start) = t0 {
                        emitted = emitted.max(elapsed_slots(start, interval));
                    }
                    // card_due() avanza su reloj interno como efecto secundario, así que solo se
                    // consulta cuando hay hueco en la cola (si no, perderíamos cartelitos).
                    if inflight.load(Ordering::Relaxed) < cap && foc.card_due().is_some() {
                        if let Some(c) = pipe.card.as_ref() {
                            let slot = emitted + 1;
                            let pts = cfr_pts(slot, fps).max(emitted_pts + 1);
                            inflight.fetch_add(1, Ordering::Relaxed);
                            let force_key = std::mem::take(&mut pending_key);
                            // El cartel no vive en el ring: seq=MAX => el worker no lo valida.
                            let item = SendItem { tex: c.tex.clone(), pts, force_key, seq: u64::MAX };
                            if tx.send(item).is_err() {
                                break;
                            }
                            emitted = slot;
                            emitted_pts = pts;
                        }
                    }
                } else if let Some(src) = latest.clone() {
                    // La cadencia la marca el TIMESTAMP real del frame (time_slot), no el reloj de
                    // pared: cada frame cae en su slot por su tiempo de presentación, eliminando el
                    // batido de muestreo (micro-tirones). El reloj de pared solo actúa de red en
                    // stalls reales (pantalla congelada): si se adelanta >=STALL_SLOTS, rellena con
                    // duplicados para no vaciar el ring. El ancla (t0) se fija DESPUÉS del primer
                    // envío: NVENC inicializa su sesión de forma perezosa en el primer ProcessInput
                    // (~200 ms) y ese arranque lo absorbe la cola, no un backlog de duplicados.
                    let cur = match t0_time {
                        Some(base) => {
                            let frame_slot = time_slot(foc.last_time, base, interval);
                            let wall = t0.map_or(frame_slot, |s| elapsed_slots(s, interval));
                            if wall - frame_slot >= STALL_SLOTS { wall - 1 } else { frame_slot }
                        }
                        None => emitted + 1,
                    };
                    // Red de seguridad: el pacing ya no toca la GPU (el convert vive en el
                    // worker), así que no debería atrasarse; si aun así lo hiciera, resincroniza
                    // para no desincronizar el audio.
                    if cur - emitted > fps as i64 {
                        emitted = cur - 1;
                        emitted_pts = emitted_pts.max(cfr_pts(emitted, fps));
                    }

                    // Un frame por slot vencido. Si la cola está llena (worker saturado >~cap
                    // frames), se descarta el slot avanzando el reloj: hueco de 1 frame en vez
                    // de un corte. El envío es solo clonar la referencia BGRA (cero GPU).
                    while emitted < cur {
                        let slot = emitted + 1;
                        let pts = cfr_pts(slot, fps).max(emitted_pts + 1);
                        if inflight.load(Ordering::Relaxed) >= cap {
                            emitted = slot;
                            emitted_pts = pts;
                            continue;
                        }
                        inflight.fetch_add(1, Ordering::Relaxed);
                        let force_key = std::mem::take(&mut pending_key);
                        let item = SendItem { tex: src.clone(), pts, force_key, seq: latest_seq };
                        if tx.send(item).is_err() {
                            break;
                        }
                        emitted = slot;
                        emitted_pts = pts;
                        if t0_time.is_none() {
                            // Primer frame enviado: fija el origen temporal del vídeo (escala WGC)
                            // para que el audio, rebasado contra video_base, comparta el cero con el
                            // vídeo; t0_time ancla la rejilla CFR al timestamp y t0 el reloj de red.
                            pipe.video_base.store(foc.last_time, Ordering::SeqCst);
                            t0_time = Some(foc.last_time);
                            t0 = Some(Instant::now());
                        }
                    }
                }

                std::thread::sleep(pace_sleep(interval));
            }

            // Cierra el canal: el worker ve Disconnected y sale; el scope hace join al cerrar.
            drop(tx);
        });
    }

    // Worker del encoder (thread::scope, toma prestado &pipe): recibe frames BGRA crudos del
    // pacing, hace el convert BGRA→NV12 y el ProcessInput bloqueante (ambos trabajos de GPU
    // AQUÍ, en un solo hilo, para no competir por el device con la cadencia), y vuelca la
    // salida al ring. Así el hilo del reloj no toca la GPU y no se congela cuando el juego la
    // satura: los stalls se absorben como latencia en la cola (buffer BGRA), no como bajón.
    fn run_encoder_thread(
        pipe: &ReplayPipeline,
        sink: &Arc<dyn VideoPacketSink>,
        stop: &Arc<AtomicBool>,
        rx: mpsc::Receiver<SendItem>,
        inflight: Arc<AtomicUsize>,
    ) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let _mmcss = MmcssTask::new("Capture");
        let encoder = &pipe.encoder;
        let events = pipe
            .enc_events
            .as_ref()
            .expect("async pump requiere eventos");
        let mut need: i32 = 0;
        let mut seq_grabbed = false;
        // Timestamps de entrada en orden de alimentación: como el encoder va en Baseline (sin
        // reordenar), la N-ésima salida corresponde al N-ésimo timestamp aquí.
        let mut pts_fifo: VecDeque<i64> = VecDeque::new();
        let mut convert_err_logged = false;

        while !stop.load(Ordering::SeqCst)
            && !pipe.size_changed.load(Ordering::Relaxed)
            && !pipe.device_lost.load(Ordering::Relaxed)
        {
            // Drenar eventos del encoder asíncrono sin bloquear (créditos + salida).
            loop {
                let ev = unsafe { events.GetEvent(MF_EVENT_FLAG_NO_WAIT) };
                let ev = match ev {
                    Ok(ev) => ev,
                    Err(_) => break,
                };
                let et = unsafe { ev.GetType().unwrap_or(0) };
                if et == ME_TRANSFORM_NEED_INPUT {
                    need += 1;
                } else if et == ME_TRANSFORM_HAVE_OUTPUT {
                    let _ = drain_encoder_output(encoder, sink, &mut seq_grabbed, &mut pts_fifo);
                }
            }

            // Con crédito y frames encolados: convertir y alimentar. Tanto el convert como el
            // ProcessInput pueden bloquearse aquí bajo carga de GPU: no pasa nada, no congela al
            // pacing; la cola BGRA absorbe el retardo.
            let mut did = false;
            while need > 0 {
                let item = match rx.try_recv() {
                    Ok(item) => item,
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return,
                };
                inflight.fetch_sub(1, Ordering::Relaxed);
                // Frame stale: mientras esperaba en cola, el handler recicló su slot del ring
                // (cola llena sostenida bajo carga). Su textura ya tiene contenido "del futuro";
                // descártalo (hueco limpio de 1 frame) en vez de encodear ese frame erróneo.
                if item.seq != u64::MAX
                    && pipe.feed.seq[item.seq as usize % pipe.feed.ring.len()]
                        .load(Ordering::Acquire)
                        != item.seq
                {
                    continue;
                }
                let nv12 = match convert_frame(pipe, &item.tex, item.pts) {
                    Ok(Some(s)) => s,
                    Ok(None) => continue,
                    Err(e) => {
                        // Device D3D perdido (TDR/reset de GPU): marca la pérdida para que el pump
                        // salga y el hilo de replay reconstruya el pipeline con un device nuevo
                        // (§4.4). Cualquier otro fallo se reporta una sola vez y se sigue.
                        if device_removed(&pipe._device) {
                            eprintln!("replay: device D3D perdido (convert), reconstruyendo");
                            pipe.device_lost.store(true, Ordering::Relaxed);
                            return;
                        }
                        if !convert_err_logged {
                            eprintln!("replay: fallo del conversor: {e:?}");
                            convert_err_logged = true;
                        }
                        continue;
                    }
                };
                if item.force_key {
                    force_keyframe(encoder);
                }
                if feed_encoder_async(encoder, &nv12, sink, &mut seq_grabbed, &mut pts_fifo) {
                    pts_fifo.push_back(item.pts);
                    did = true;
                } else if device_removed(&pipe._device) {
                    eprintln!("replay: device D3D perdido (encoder), reconstruyendo");
                    pipe.device_lost.store(true, Ordering::Relaxed);
                    return;
                }
                need -= 1;
            }

            // Sin trabajo (sin crédito o sin frames): cede la CPU brevemente para no girar.
            if !did {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    // Bombeo del encoder por software (MFT síncrono, sin eventos): misma cadencia CFR
    // dirigida por reloj que el camino hardware, pero codificando de forma síncrona un
    // frame por slot vencido (duplicando el último cuando no hay frame nuevo).
    fn run_pump_sync(
        pipe: &ReplayPipeline,
        stop: &Arc<AtomicBool>,
        sink: &Arc<dyn VideoPacketSink>,
        window_mode: bool,
    ) {
        let fps = pipe.fps;
        let interval = fps_interval(fps).max(1);
        let mut seq_grabbed = false;
        let mut pts_fifo: VecDeque<i64> = VecDeque::new();
        let mut foc = FocusState::new();
        let mut latest: Option<ID3D11Texture2D> = None;
        let mut t0_time: Option<i64> = None;
        let mut t0: Option<Instant> = None;
        let mut emitted: i64 = -1;
        let mut emitted_pts: i64 = -1;

        while !stop.load(Ordering::SeqCst)
            && !pipe.size_changed.load(Ordering::Relaxed)
            && !pipe.device_lost.load(Ordering::Relaxed)
            && !pipe.retarget.load(Ordering::Relaxed)
        {
            foc.poll(window_mode, pipe);

            loop {
                match pipe.rx.try_recv() {
                    Ok(f) => {
                        foc.note_real_frame(&f.0 .0, f.1);
                        if !foc.minimized {
                            latest = Some(f.0 .0);
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return,
                }
            }

            if foc.take_force_key() {
                force_keyframe(&pipe.encoder);
            }

            if foc.minimized {
                if let Some(start) = t0 {
                    emitted = emitted.max(elapsed_slots(start, interval));
                }
                if foc.card_due().is_some() {
                    if let Some(c) = pipe.card.as_ref() {
                        let slot = emitted + 1;
                        let pts = cfr_pts(slot, fps).max(emitted_pts + 1);
                        encode_one(pipe, sink, &c.tex, pts, &mut seq_grabbed, &mut pts_fifo);
                        emitted = slot;
                        emitted_pts = pts;
                    }
                }
            } else if let Some(src) = latest.clone() {
                // Cadencia anclada al timestamp real del frame (sin batido); el reloj de pared solo
                // rellena duplicados en congelados reales (>=STALL_SLOTS sin frame nuevo).
                let cur = match t0_time {
                    Some(base) => {
                        let frame_slot = time_slot(foc.last_time, base, interval);
                        let wall = t0.map_or(frame_slot, |s| elapsed_slots(s, interval));
                        if wall - frame_slot >= STALL_SLOTS { wall - 1 } else { frame_slot }
                    }
                    None => {
                        pipe.video_base.store(foc.last_time, Ordering::SeqCst);
                        t0_time = Some(foc.last_time);
                        t0 = Some(Instant::now());
                        emitted + 1
                    }
                };
                // Si el encoder software no sostiene la tasa, no acumular retraso sin fin:
                // saltar cerca del slot actual descartando duplicados (CFR best-effort, solo
                // en sobrecarga). A perfiles ligeros (p. ej. 480p/20) no ocurre.
                if cur - emitted > fps as i64 {
                    emitted = cur - 1;
                    emitted_pts = emitted_pts.max(cfr_pts(emitted, fps));
                }
                while emitted < cur {
                    let slot = emitted + 1;
                    let pts = cfr_pts(slot, fps).max(emitted_pts + 1);
                    encode_one(pipe, sink, &src, pts, &mut seq_grabbed, &mut pts_fifo);
                    emitted = slot;
                    emitted_pts = pts;
                }
            }

            std::thread::sleep(pace_sleep(interval));
        }
    }

    // Codifica un frame BGRA con un PTS sintético ya calculado (cadencia CFR): BGRA→NV12 y
    // entrega al encoder, drenando su salida al ring. El PTS llega ya monotónico desde el
    // bombeo, así que aquí no se rebasa nada.
    fn encode_one(
        pipe: &ReplayPipeline,
        sink: &Arc<dyn VideoPacketSink>,
        bgra: &ID3D11Texture2D,
        pts: i64,
        seq_grabbed: &mut bool,
        pts_fifo: &mut VecDeque<i64>,
    ) {
        let nv12 = match convert_frame(pipe, bgra, pts) {
            Ok(Some(s)) => s,
            Ok(None) => return,
            Err(_) => {
                // Device D3D perdido (TDR/reset): marca la pérdida para que el pump síncrono
                // salga y el hilo de replay reconstruya con un device nuevo (§4.4).
                if device_removed(&pipe._device) {
                    eprintln!("replay: device D3D perdido (convert sw), reconstruyendo");
                    pipe.device_lost.store(true, Ordering::Relaxed);
                }
                return;
            }
        };

        let mut fed_ok = false;
        for _ in 0..64 {
            match unsafe { pipe.encoder.ProcessInput(0, &nv12, 0) } {
                Ok(()) => {
                    fed_ok = true;
                    break;
                }
                Err(e) if e.code() == MF_E_NOTACCEPTING => {
                    let _ = drain_encoder_output(&pipe.encoder, sink, seq_grabbed, pts_fifo);
                }
                Err(_) => {
                    if device_removed(&pipe._device) {
                        pipe.device_lost.store(true, Ordering::Relaxed);
                    }
                    break;
                }
            }
        }
        if fed_ok {
            pts_fifo.push_back(pts);
            let _ = drain_encoder_output(&pipe.encoder, sink, seq_grabbed, pts_fifo);
        }
    }

    // Seguimiento de "fuera de foco" para el bombeo del replay: detecta cuándo el juego está
    // minimizado (sin ventana capturable) y compone una sola vez el cartel a partir del
    // último frame real. Mientras siga minimizado, `card_due` marca cuándo emitir el cartel.
    // Cadencia del cartel: estático, así que con pocos frames basta para ocupar el tiempo en
    // el clip y mantener keyframes. ~2 fps (coste ínfimo).
    const CARD_INTERVAL: Duration = Duration::from_millis(500);

    struct FocusState {
        minimized: bool,
        last_min_check: std::time::Instant,
        last_tex: Option<ID3D11Texture2D>,
        last_time: i64,
        t_anchor: std::time::Instant,
        card_ready: bool,
        last_card_emit: std::time::Instant,
        // El próximo frame codificado debe forzar un IDR. Se activa al entrar y al salir de
        // "fuera de foco" para que la transición cartel↔juego no arrastre referencias (evita
        // ghosting) y para poder empezar el clip justo en el cambio de escena.
        force_key: bool,
    }

    impl FocusState {
        fn new() -> Self {
            let now = std::time::Instant::now();
            Self {
                minimized: false,
                last_min_check: now,
                last_tex: None,
                last_time: 0,
                t_anchor: now,
                card_ready: false,
                last_card_emit: now,
                force_key: false,
            }
        }

        fn note_real_frame(&mut self, tex: &ID3D11Texture2D, time: i64) {
            self.last_tex = Some(tex.clone());
            self.last_time = time;
            self.t_anchor = std::time::Instant::now();
        }

        fn poll(&mut self, window_mode: bool, pipe: &ReplayPipeline) {
            if !window_mode || self.last_min_check.elapsed() < Duration::from_millis(250) {
                return;
            }
            self.last_min_check = std::time::Instant::now();

            // Re-target principal (barato, sin EnumWindows): si el proceso de la ventana que
            // capturamos ya no es el juego en primer plano rastreado, rearmamos contra la ventana
            // correcta. Cubre el launcher dejado abierto: el replay se armó sobre él y, al abrir el
            // juego encima (aunque el launcher siga VISIBLE detrás, en fullscreen), el foreground
            // pasa al juego, que es otro PID. Tras un rearme la ventana capturada pertenece a ese
            // PID, así que en juego normal esto coincide y no dispara nada.
            if let Some(want) = crate::detect::current_game_pid() {
                if window_pid(pipe.game_hwnd) != Some(want) {
                    pipe.retarget.store(true, Ordering::Relaxed);
                    return;
                }
            }

            let now_min = window_minimized(pipe.game_hwnd);

            // Fallback para swaps de ventana con el MISMO PID (recreación al alternar fullscreen):
            // la ventana rastreada dejó de ser capturable pero el juego tiene otra ventana visible
            // distinta. resolve_game_window() salta ventanas minimizadas, así que un alt-tab real
            // (única ventana del juego, minimizada) devuelve None y conserva el cartel.
            if now_min {
                if let Some(hwnd) = resolve_game_window() {
                    if hwnd.0 as isize != pipe.game_hwnd {
                        pipe.retarget.store(true, Ordering::Relaxed);
                        return;
                    }
                }
            }

            if now_min && !self.minimized {
                // Transición a minimizado: componer el cartel del último frame real.
                self.card_ready = false;
                if let (Some(c), Some(src)) = (pipe.card.as_ref(), self.last_tex.as_ref()) {
                    match c.overlay.render(src, &c.tex) {
                        Ok(()) => {
                            self.card_ready = true;
                            self.force_key = true;
                            // Emitir el primero de inmediato.
                            self.last_card_emit = std::time::Instant::now() - CARD_INTERVAL;
                        }
                        Err(e) => eprintln!("overlay: fallo al componer el cartel: {e:?}"),
                    }
                }
            } else if !now_min && self.minimized {
                // Vuelta al juego: el primer frame real arranca un GOP nuevo.
                self.force_key = true;
            }
            self.minimized = now_min;
        }

        // Devuelve el timestamp (escala WGC) del cartel si toca emitirlo, o None.
        fn card_due(&mut self) -> Option<i64> {
            if !self.minimized || !self.card_ready || self.last_card_emit.elapsed() < CARD_INTERVAL {
                return None;
            }
            self.last_card_emit = std::time::Instant::now();
            let dt = (self.t_anchor.elapsed().as_nanos() as i64) / 100;
            Some(self.last_time + dt)
        }

        // True una sola vez tras una transición: el llamador debe forzar un keyframe antes
        // del siguiente ProcessInput.
        fn take_force_key(&mut self) -> bool {
            std::mem::take(&mut self.force_key)
        }
    }

    // ¿La ventana del juego está minimizada (u oculta)? Consulta barata por HWND, sin recorrer
    // todas las ventanas; pensada para llamarse desde el hilo de bombeo. 0 = sin ventana.
    fn window_minimized(hwnd_raw: isize) -> bool {
        if hwnd_raw == 0 {
            return true;
        }
        let hwnd = HWND(hwnd_raw as *mut core::ffi::c_void);
        unsafe { IsIconic(hwnd).as_bool() || !IsWindowVisible(hwnd).as_bool() }
    }

    // PID dueño de una ventana (0 = sin ventana o HWND ya destruida). Consulta O(1) por HWND, sin
    // recorrer ventanas; la usa FocusState para comparar el proceso capturado con el juego rastreado.
    fn window_pid(hwnd_raw: isize) -> Option<u32> {
        if hwnd_raw == 0 {
            return None;
        }
        let hwnd = HWND(hwnd_raw as *mut core::ffi::c_void);
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        (pid != 0).then_some(pid)
    }

    // Pide al encoder que el siguiente frame de salida sea un IDR (best-effort vía ICodecAPI).
    fn force_keyframe(enc: &IMFTransform) {
        if let Ok(codec) = enc.cast::<ICodecAPI>() {
            let _ = unsafe { codec.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &variant_u32(1)) };
        }
    }

    // BGRA→NV12 con el conversor (síncrono). Devuelve la muestra NV12 con su tiempo
    // ya fijado, o None si el conversor no produjo salida para esta entrada.
    //
    // Tras un ProcessInput hay que drenar con ProcessOutput **hasta** MF_E_TRANSFORM_
    // NEED_MORE_INPUT; si no, el MFT se queda "con salida pendiente" y rechaza el
    // siguiente input con NOTACCEPTING (justo lo que pasaba a partir del 2º frame).
    fn convert_frame(
        pipe: &ReplayPipeline,
        bgra: &ID3D11Texture2D,
        time: i64,
    ) -> Result<Option<IMFSample>> {
        let in_sample = make_converter_input(pipe, bgra, time)?;
        unsafe { pipe.converter.ProcessInput(0, &in_sample, 0)? };

        let mut result: Option<IMFSample> = None;
        loop {
            let mut out = MFT_OUTPUT_DATA_BUFFER::default();
            if !pipe.converter_provides {
                let idx = pipe.nv12_next.get();
                pipe.nv12_next.set((idx + 1) % pipe.nv12_pool.len().max(1));
                out.pSample = ManuallyDrop::new(Some(pipe.nv12_pool[idx].clone()));
            }
            let mut status = 0u32;
            let hr = unsafe {
                pipe.converter
                    .ProcessOutput(0, std::slice::from_mut(&mut out), &mut status)
            };
            let taken = unsafe { ManuallyDrop::take(&mut out.pSample) };
            match hr {
                Ok(()) => {
                    if result.is_none() {
                        if let Some(s) = taken {
                            unsafe {
                                s.SetSampleTime(time)?;
                                s.SetSampleDuration(166_667)?;
                            }
                            result = Some(s);
                        }
                    }
                }
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => break,
                Err(e) => return Err(e),
            }
        }
        Ok(result)
    }

    // Muestra de entrada (ARGB32) para el conversor. En hardware envuelve la textura BGRA
    // directamente (zero-copy GPU); en software la baja a memoria de sistema por staging.
    fn make_converter_input(
        pipe: &ReplayPipeline,
        bgra: &ID3D11Texture2D,
        time: i64,
    ) -> Result<IMFSample> {
        if let Some(sw) = &pipe.sw {
            return readback_argb(sw, bgra, time);
        }
        let buffer = unsafe { MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, bgra, 0, false)? };
        let len = unsafe { buffer.cast::<IMF2DBuffer>()?.GetContiguousLength()? };
        unsafe { buffer.SetCurrentLength(len)? };
        let in_sample = unsafe { MFCreateSample()? };
        unsafe {
            in_sample.AddBuffer(&buffer)?;
            in_sample.SetSampleTime(time)?;
            in_sample.SetSampleDuration(166_667)?;
        }
        Ok(in_sample)
    }

    // Vuelca la textura BGRA de GPU a una muestra ARGB32 en memoria de sistema vía una
    // textura de staging (CPU read). Solo en el camino software; aquí sí hay copia
    // GPU→CPU porque los MFT por software no leen texturas D3D.
    fn readback_argb(sw: &SwReadback, bgra: &ID3D11Texture2D, time: i64) -> Result<IMFSample> {
        let stride = sw.width as usize * 4;
        let total = stride * sw.height as usize;
        unsafe {
            sw.ctx.CopyResource(&sw.staging, bgra);
            let mut map = D3D11_MAPPED_SUBRESOURCE::default();
            sw.ctx
                .Map(&sw.staging, 0, D3D11_MAP_READ, 0, Some(&mut map))?;
            let buffer = MFCreateMemoryBuffer(total as u32)?;
            let mut ptr: *mut u8 = std::ptr::null_mut();
            buffer.Lock(&mut ptr, None, None)?;
            for row in 0..sw.height as usize {
                let src = (map.pData as *const u8).add(row * map.RowPitch as usize);
                std::ptr::copy_nonoverlapping(src, ptr.add(row * stride), stride);
            }
            buffer.Unlock()?;
            buffer.SetCurrentLength(total as u32)?;
            sw.ctx.Unmap(&sw.staging, 0);

            let in_sample = MFCreateSample()?;
            in_sample.AddBuffer(&buffer)?;
            in_sample.SetSampleTime(time)?;
            in_sample.SetSampleDuration(166_667)?;
            Ok(in_sample)
        }
    }

    fn drain_encoder_output(
        enc: &IMFTransform,
        sink: &Arc<dyn VideoPacketSink>,
        seq_grabbed: &mut bool,
        pts_fifo: &mut VecDeque<i64>,
    ) -> Result<usize> {
        let mut drained = 0usize;
        loop {
            let mut out = MFT_OUTPUT_DATA_BUFFER::default();
            let mut status = 0u32;
            let hr = unsafe { enc.ProcessOutput(0, std::slice::from_mut(&mut out), &mut status) };
            match hr {
                Ok(()) => {}
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => break,
                Err(e) => return Err(e),
            }

            if !*seq_grabbed {
                if let Ok(mt) = unsafe { enc.GetOutputCurrentType(0) } {
                    if let Some(h) = blob(&mt, &MF_MT_MPEG_SEQUENCE_HEADER) {
                        sink.set_seq_header(h);
                        *seq_grabbed = true;
                    }
                }
            }

            let sample = unsafe { ManuallyDrop::take(&mut out.pSample) };
            if let Some(sample) = sample {
                if let Some((data, enc_time, dur, key)) = read_sample(&sample) {
                    // Fallback de cabecera de secuencia: muchos encoders por hardware no
                    // exponen MF_MT_MPEG_SEQUENCE_HEADER en su tipo de salida y entregan el
                    // SPS/PPS en banda (Annex B) dentro del keyframe. Sin esa cabecera el
                    // sink MP4 no puede escribir el `avcC` (MF_E_SINK_HEADERS_NOT_FOUND),
                    // así que la extraemos del propio bitstream. Se intenta en cada paquete
                    // (no solo en los keyframe) porque algunos encoders entregan el SPS/PPS
                    // en un paquete de configuración aparte; solo corre hasta lograrlo.
                    if !*seq_grabbed {
                        let ps = extract_param_sets(&data);
                        if !ps.is_empty() {
                            sink.set_seq_header(ps);
                            *seq_grabbed = true;
                        }
                    }
                    // El tiempo real lo pone el FIFO de entrada; el del encoder solo
                    // sirve de respaldo si por algún motivo el FIFO se vaciara.
                    let time = pts_fifo.pop_front().unwrap_or(enc_time);
                    sink.push_video(data, time, dur, key);
                    drained += 1;
                }
            }
        }
        Ok(drained)
    }

    fn read_sample(sample: &IMFSample) -> Option<(Vec<u8>, i64, i64, bool)> {
        unsafe {
            let buf = sample.ConvertToContiguousBuffer().ok()?;
            let mut ptr: *mut u8 = std::ptr::null_mut();
            let mut cur = 0u32;
            buf.Lock(&mut ptr, None, Some(&mut cur)).ok()?;
            let data = std::slice::from_raw_parts(ptr, cur as usize).to_vec();
            let _ = buf.Unlock();
            let time = sample.GetSampleTime().unwrap_or(0);
            let dur = sample.GetSampleDuration().unwrap_or(166_667);
            // Algunos encoders por hardware no marcan MFSampleExtension_CleanPoint en los
            // IDR; sin ese flag el ring buffer nunca reconoce un keyframe y no se puede
            // guardar el replay. Como respaldo, detectamos el IDR (NAL tipo 5) en el
            // bitstream, que es la marca definitiva e independiente del encoder.
            let key = sample.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) == 1
                || contains_idr(&data);
            Some((data, time, dur, key))
        }
    }

    // True si el bitstream Annex B contiene una unidad NAL IDR (tipo 5): el inicio de un
    // GOP por el que se puede empezar a decodificar (y por tanto a muxear el replay).
    fn contains_idr(data: &[u8]) -> bool {
        let mut i = 0usize;
        while i + 3 <= data.len() {
            if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
                let header_pos = i + 3;
                if header_pos < data.len() && (data[header_pos] & 0x1F) == 5 {
                    return true;
                }
                i += 3;
            } else {
                i += 1;
            }
        }
        false
    }

    // Extrae las unidades NAL de parámetros (SPS=7, PPS=8) en Annex B, con su start code,
    // de un bitstream H.264, para reconstruir MF_MT_MPEG_SEQUENCE_HEADER cuando el encoder
    // no lo expone en su tipo de salida (ver drain_encoder_output). El sink MP4 usa este
    // blob para el `avcC`; el formato esperado es exactamente el de la propia transmisión.
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
            if header_pos >= data.len() {
                break;
            }
            // Incluir el 00 inicial del start code de 4 bytes (00 00 00 01) si está presente.
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

    struct LiveMux {
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
        fn new(
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
            let timeout = *self.header_timeout.lock().unwrap();
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
    fn mux_replay(
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

    fn create_bgra_textures(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        count: usize,
    ) -> Result<Vec<ID3D11Texture2D>> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE).0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let mut t: Option<ID3D11Texture2D> = None;
            unsafe { device.CreateTexture2D(&desc, None, Some(&mut t))? };
            out.push(t.ok_or_else(null_out)?);
        }
        Ok(out)
    }

    fn create_nv12_samples(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        count: usize,
    ) -> Result<Vec<IMFSample>> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_NV12,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let mut t: Option<ID3D11Texture2D> = None;
            unsafe { device.CreateTexture2D(&desc, None, Some(&mut t))? };
            let tex = t.ok_or_else(null_out)?;
            let buf = unsafe { MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, &tex, 0, false)? };
            let sample = unsafe { MFCreateSample()? };
            unsafe { sample.AddBuffer(&buf)? };
            out.push(sample);
        }
        Ok(out)
    }

    // Salidas NV12 en memoria de sistema para el conversor por software (NV12 contiguo:
    // plano Y de w*h + plano UV intercalado de w*h/2).
    fn create_nv12_cpu_samples(width: u32, height: u32, count: usize) -> Result<Vec<IMFSample>> {
        let size = width * height + (width * height) / 2;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let buf = unsafe { MFCreateMemoryBuffer(size)? };
            let sample = unsafe { MFCreateSample()? };
            unsafe { sample.AddBuffer(&buf)? };
            out.push(sample);
        }
        Ok(out)
    }

    // Textura de staging BGRA legible por CPU para el readback del camino software.
    fn create_staging_bgra(
        device: &ID3D11Device,
        width: u32,
        height: u32,
    ) -> Result<ID3D11Texture2D> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut t: Option<ID3D11Texture2D> = None;
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut t))? };
        Ok(t.ok_or_else(null_out)?)
    }

    fn pack2(high: u32, low: u32) -> u64 {
        ((high as u64) << 32) | low as u64
    }

    // Bits por píxel y frame según calidad (bitrate = ancho·alto·fps·factor). Calibrado con
    // SteelSeries Moments a 1080p60: Bajo ≈ 19, Medio ≈ 34, Alto ≈ 50, Muy alta ≈ 90,
    // Ultra ≈ 130 Mbps. Debe coincidir con qualityFactor() del frontend (estimación de tamaño).
    fn bitrate_factor(quality: &str) -> f64 {
        match quality {
            "low" => 0.15,
            "normal" => 0.27,
            "veryhigh" => 0.72,
            "ultra" => 1.05,
            _ => 0.40, // "high" (Alto, por defecto)
        }
    }

    fn clamp_fps(fps: u32) -> u32 {
        fps.clamp(10, 240)
    }

    // Piso de 1 Mbps: solo como red de seguridad para combos extremos (p. ej. 480p/20fps/Bajo);
    // por encima de eso los cuatro niveles de calidad se diferencian en todas las resoluciones.
    fn target_bitrate(width: u32, height: u32, fps: u32, factor: f64) -> u32 {
        (((width as u64 * height as u64 * fps as u64) as f64 * factor) as u32).max(1_000_000)
    }

    // Bitrate final del encoder: el valor personalizado (bps) si el usuario lo fijó
    // (override > 0), o el automático según resolución/fps/calidad.
    fn resolve_bitrate(width: u32, height: u32, fps: u32, factor: f64, override_bps: u32) -> u32 {
        if override_bps > 0 {
            override_bps
        } else {
            target_bitrate(width, height, fps, factor)
        }
    }

    // Dimensiones de salida dadas las de captura y un alto objetivo (0 = nativo). Se
    // mantiene el aspecto, se redondea a par (NV12/H.264) y NUNCA se hace upscaling:
    // si el objetivo es >= al nativo se graba a nativo (subir resolución solo infla el
    // archivo sin añadir detalle). El escalado real lo hace el encoder/conversor.
    fn output_dims(cap_w: u32, cap_h: u32, target_h: u32) -> (u32, u32) {
        if target_h == 0 || target_h >= cap_h || cap_h == 0 {
            return (cap_w, cap_h);
        }
        let out_h = (target_h & !1).max(2);
        let out_w = ((((cap_w as u64 * out_h as u64) / cap_h as u64) as u32) & !1).max(2);
        (out_w, out_h)
    }

    // WGC limita FrameArrived a ~60 Hz por defecto mediante MinUpdateInterval. Lo bajamos a la
    // mitad del periodo objetivo para que entregue candidatos de sobra y el limitador clave los
    // FPS exactos (y permita >60 en monitores de alta tasa). En Windows 10 la propiedad no
    // existe: la llamada falla sin efecto y se mantiene el tope por defecto.
    fn set_capture_rate(session: &GraphicsCaptureSession, fps: u32) {
        let dur = (fps_interval(fps) / 2).max(20_000);
        let _ = session.SetMinUpdateInterval(windows::Foundation::TimeSpan { Duration: dur });
    }

    // Intervalo mínimo (en unidades de 100 ns) entre frames codificados para no superar
    // los FPS objetivo. 0 = sin límite. El límite real se aplica descartando frames en el
    // handler de captura, antes de tocar la GPU/encoder: menos FPS = menos trabajo.
    fn fps_interval(fps: u32) -> i64 {
        if fps == 0 {
            0
        } else {
            10_000_000 / fps as i64
        }
    }

    // Profundidad de la cola entre el hilo de pacing y el del encoder: ~300 ms de frames.
    // Es el margen que absorbe los stalls de ProcessInput de NVENC bajo carga de GPU sin
    // congelar la cadencia; más allá se descartan frames sueltos (degradación suave). Se
    // acota para limitar la VRAM del pool NV12 (crece con la resolución de salida).
    fn enc_buffer_frames(fps: u32) -> usize {
        ((fps as usize * 3 + 9) / 10).clamp(6, 24)
    }

    // PTS absoluto (en unidades de 100 ns) del slot CFR `n` a `fps`. No acumulado:
    // se calcula sobre el índice del slot para que no haya deriva aunque 10⁷/fps no sea
    // entero (60 → 166666/166667 alternando, media exacta). La duración del sample n es
    // cfr_pts(n+1) - cfr_pts(n). Es la cadencia constante que sustituye a los timestamps
    // irregulares de WGC: emitimos un frame por slot, duplicando el último si no llegó uno.
    fn cfr_pts(slot: i64, fps: u32) -> i64 {
        if fps == 0 {
            0
        } else {
            (slot * 10_000_000) / fps as i64
        }
    }

    // La cadencia CFR se apoya en sleeps por debajo del periodo de frame (hasta ~4 ms a
    // 240 fps). La resolución por defecto del timer de Windows (~15 ms) los volvería muy
    // imprecisos, así que subimos la resolución a 1 ms mientras dura la captura y la
    // restauramos al salir. Es la práctica estándar en apps multimedia.
    struct TimerRes;
    impl TimerRes {
        fn new() -> Self {
            unsafe { timeBeginPeriod(1) };
            TimerRes
        }
    }
    impl Drop for TimerRes {
        fn drop(&mut self) {
            unsafe { timeEndPeriod(1) };
        }
    }

    // MMCSS: registra el hilo actual en el Multimedia Class Scheduler Service. Sin esto, el hilo
    // de bombeo es de prioridad normal y un juego a pantalla completa que satura los núcleos lo
    // dejaba sin CPU ~1 s en ráfagas de carga: el reloj de cadencia seguía corriendo y el resync
    // tapaba el atraso saltando ~1 s, dejando congelaciones y tirones en el clip. MMCSS le
    // garantiza scheduling frente al juego. Se revierte al terminar el hilo. Best-effort: si
    // avrt falla, se sigue sin la garantía antes que abortar la captura (CLAUDE.md §4.4).
    struct MmcssTask(HANDLE);

    impl MmcssTask {
        fn new(task: &str) -> Option<MmcssTask> {
            let name = HSTRING::from(task);
            let mut idx = 0u32;
            match unsafe { AvSetMmThreadCharacteristicsW(PCWSTR(name.as_ptr()), &mut idx) } {
                Ok(h) if !h.is_invalid() => {
                    let _ = unsafe { AvSetMmThreadPriority(h, AVRT_PRIORITY_HIGH) };
                    Some(MmcssTask(h))
                }
                Ok(_) => None,
                Err(e) => {
                    eprintln!("mmcss: no se pudo registrar el hilo en '{task}': {e:?}");
                    None
                }
            }
        }
    }

    impl Drop for MmcssTask {
        fn drop(&mut self) {
            let _ = unsafe { AvRevertMmThreadCharacteristics(self.0) };
        }
    }

    // El callback FrameArrived de WGC corre en hilos del threadpool del sistema (prioridad
    // normal) y hace CopyResource sobre el contexto D3D11 compartido (multihilo-protegido). Si
    // el juego lo preempta con ese lock tomado, bloquea al pump aunque este ya sea de alta
    // prioridad: una inversión de prioridad que dejaba algún freeze residual de ~1 s. Elevamos
    // también ese hilo a MMCSS, una sola vez por hilo del pool (se reutilizan), vía thread-local:
    // registrar en cada frame sería caro y se revierte solo al morir el hilo. El bool evita
    // reintentar en cada callback si avrt fallara.
    thread_local! {
        static HANDLER_MMCSS: std::cell::RefCell<(bool, Option<MmcssTask>)> =
            const { std::cell::RefCell::new((false, None)) };
    }

    fn ensure_handler_priority() {
        HANDLER_MMCSS.with(|cell| {
            let mut s = cell.borrow_mut();
            if !s.0 {
                s.0 = true;
                s.1 = MmcssTask::new("Capture");
            }
        });
    }

    // Selección CFR: conserva UN frame por slot sobre la MISMA rejilla `time_slot` que emite el
    // pump. La reja anterior avanzaba por su cuenta (umbral con holgura), desfasada de la del pump:
    // ese doble cuantizado conservaba frames casi contiguos (pares casi-duplicados, incluidos los
    // bursts sub-ms que WGC entrega al presentar dos veces) y saltaba huecos. Eso era el batido que
    // se veía al grabar 60 desde juegos a tasa no múltiplo (p. ej. 110 fps). Al quedarnos con el
    // primer frame de cada slot nuevo, no puede haber dos frames en el mismo slot y el espaciado
    // queda ~uniforme (verificado contra SteelSeries Moments). Un hueco real (juego congelado) hace
    // que el slot salte; el pump lo absorbe con su guarda de reloj de pared (STALL_SLOTS).
    struct SlotSelector {
        base: AtomicI64,
        // Último slot conservado. i64::MIN = aún sin base (ningún frame visto).
        last_slot: AtomicI64,
    }

    impl SlotSelector {
        fn new() -> SlotSelector {
            SlotSelector {
                base: AtomicI64::new(0),
                last_slot: AtomicI64::new(i64::MIN),
            }
        }

        // true si `t` (SystemRelativeTime del frame) abre un slot nuevo. WGC serializa el callback,
        // así que no hay carrera real; los atómicos solo aportan la mutabilidad interior que exige Fn.
        fn keep(&self, t: i64, interval: i64) -> bool {
            if interval <= 0 {
                return true;
            }
            if self.last_slot.load(Ordering::Relaxed) == i64::MIN {
                self.base.store(t, Ordering::Relaxed);
                self.last_slot.store(0, Ordering::Relaxed);
                return true;
            }
            let slot = time_slot(t, self.base.load(Ordering::Relaxed), interval);
            if slot > self.last_slot.load(Ordering::Relaxed) {
                self.last_slot.store(slot, Ordering::Relaxed);
                true
            } else {
                false
            }
        }
    }

    // VARIANT de tipo VT_UI4 para las propiedades de ICodecAPI (GOP, etc.).
    fn variant_u32(val: u32) -> VARIANT {
        let mut v = VARIANT::default();
        unsafe {
            let inner = &mut *v.Anonymous.Anonymous;
            inner.vt = VT_UI4;
            inner.Anonymous.ulVal = val;
        }
        v
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

    // Pico de VBR = 1.75x la media: da margen a las escenas complejas sin disparar el
    // tamaño. saturating_mul evita overflow con bitrates muy altos (4K).
    fn peak_bitrate(mean: u32) -> u32 {
        mean.saturating_mul(7) / 4
    }

    // Fija la política de calidad: Peak-Constrained VBR (modo 1) con media/pico, CABAC y
    // B-frames=0. Devuelve los ajustes que el encoder rechazó (vacío = todos aceptados).
    // Todo best-effort (CLAUDE.md §4.4). IMPORTANTE: debe llamarse ANTES de SetOutputType;
    // el encoder H.264 de MF ignora el modo de rate control si se fija después (se queda en
    // CBR). B-frames=0 puede salir en la lista de rechazados sin consecuencia (ya es el
    // default), por eso no cuenta para detectar "este transform no es el encoder".
    fn set_quality_codec_settings(codec: &ICodecAPI, mean_bitrate: u32, gop: u32) -> Vec<&'static str> {
        unsafe {
            let sets: [(&'static str, Result<()>); 6] = [
                ("RateControlMode", codec.SetValue(&CODECAPI_AVEncCommonRateControlMode, &variant_u32(1))),
                ("MeanBitRate", codec.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &variant_u32(mean_bitrate))),
                ("MaxBitRate", codec.SetValue(&CODECAPI_AVEncCommonMaxBitRate, &variant_u32(peak_bitrate(mean_bitrate)))),
                ("CABAC", codec.SetValue(&CODECAPI_AVEncH264CABACEnable, &variant_bool(true))),
                ("GOPSize", codec.SetValue(&CODECAPI_AVEncMPVGOPSize, &variant_u32(gop))),
                ("BPictureCount", codec.SetValue(&CODECAPI_AVEncMPVDefaultBPictureCount, &variant_u32(0))),
            ];
            sets.iter().filter(|(_, r)| r.is_err()).map(|(n, _)| *n).collect()
        }
    }

    // Lee de vuelta el modo de rate control (1=VBR) para confirmar —no suponer— que quedó, y
    // registra el resultado. Si el encoder rechazó TODO menos B-frames, es que este transform
    // no es el encoder (p. ej. el conversor de color del SinkWriter): no registra (evita ruido).
    fn log_encoder_quality(codec: &ICodecAPI, label: &str, mean_bitrate: u32, gop: u32, failed: &[&str]) {
        let real_failed: Vec<&&str> = failed.iter().filter(|n| **n != "BPictureCount").collect();
        if real_failed.len() >= 5 {
            return;
        }
        let mode = unsafe { codec.GetValue(&CODECAPI_AVEncCommonRateControlMode) }
            .ok()
            .map(|v| unsafe { (*v.Anonymous.Anonymous).Anonymous.ulVal });
        if failed.is_empty() {
            eprintln!("encoder[{label}]: calidad aplicada (VBR mean={mean_bitrate} pico={} gop={gop}); rate control leído={mode:?} (1=VBR)", peak_bitrate(mean_bitrate));
        } else {
            eprintln!("encoder[{label}]: ajustes rechazados {failed:?}; rate control leído={mode:?} (1=VBR)");
        }
    }

    fn clip_filename() -> String {
        let st: SYSTEMTIME = unsafe { GetLocalTime() };
        format!(
            "Flashback_{:04}-{:02}-{:02}_{:02}-{:02}-{:02}.mp4",
            st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
        )
    }

    fn capture_item_for_monitor(monitor: HMONITOR) -> Result<GraphicsCaptureItem> {
        let interop =
            windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        unsafe { interop.CreateForMonitor(monitor) }
    }

    fn capture_item_for_window(hwnd: HWND) -> Result<GraphicsCaptureItem> {
        let interop =
            windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        unsafe { interop.CreateForWindow(hwnd) }
    }

    // Resuelve el objetivo de captura a un GraphicsCaptureItem. `"window"` = la ventana
    // del juego detectado (modo Aplicación); cualquier otro valor = nombre de dispositivo
    // de monitor (`\\.\DISPLAYn`). Se ejecuta dentro del hilo de captura (apartamento MTA).
    fn resolve_target_item(target: &str) -> std::result::Result<GraphicsCaptureItem, String> {
        if target == "window" {
            let hwnd = resolve_game_window()
                .ok_or_else(|| "No hay ventana de juego para capturar".to_string())?;
            capture_item_for_window(hwnd).map_err(|e| format!("{e:?}"))
        } else {
            let hmon =
                resolve_monitor(target).ok_or_else(|| "Monitor no encontrado".to_string())?;
            capture_item_for_monitor(hmon).map_err(|e| format!("{e:?}"))
        }
    }

    // Ventana principal del juego rastreado: la top-level visible y no minimizada de
    // mayor área que pertenece a su PID. Sirve para CreateForWindow en modo Aplicación.
    fn resolve_game_window() -> Option<HWND> {
        let pid = crate::detect::current_game_pid()?;
        find_main_window(pid)
    }

    struct WinSearch {
        pid: u32,
        best: Option<HWND>,
        best_area: i32,
    }

    fn find_main_window(pid: u32) -> Option<HWND> {
        let mut search = WinSearch {
            pid,
            best: None,
            best_area: 0,
        };
        unsafe {
            let _ = EnumWindows(
                Some(window_proc),
                LPARAM(&mut search as *mut WinSearch as isize),
            );
        }
        search.best
    }

    unsafe extern "system" fn window_proc(hwnd: HWND, data: LPARAM) -> BOOL {
        let search = &mut *(data.0 as *mut WinSearch);
        let mut wpid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut wpid));
        if wpid != search.pid
            || !IsWindowVisible(hwnd).as_bool()
            || IsIconic(hwnd).as_bool()
            || GetWindow(hwnd, GW_OWNER).map(|h| !h.0.is_null()).unwrap_or(false)
        {
            return BOOL(1);
        }
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
            return BOOL(1);
        }
        let area = (rect.right - rect.left) * (rect.bottom - rect.top);
        if area > search.best_area {
            search.best_area = area;
            search.best = Some(hwnd);
        }
        BOOL(1)
    }

    fn enum_monitors() -> Vec<HMONITOR> {
        let mut list: Vec<HMONITOR> = Vec::new();
        unsafe {
            let _ = EnumDisplayMonitors(
                None,
                None,
                Some(enum_proc),
                LPARAM(&mut list as *mut Vec<HMONITOR> as isize),
            );
        }
        list
    }

    unsafe extern "system" fn enum_proc(
        hmon: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let list = &mut *(data.0 as *mut Vec<HMONITOR>);
        list.push(hmon);
        BOOL(1)
    }

    fn monitor_info(hmon: HMONITOR, index: usize, screen_dc: HDC) -> Option<MonitorInfo> {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        let ok = unsafe { GetMonitorInfoW(hmon, &mut info as *mut _ as *mut MONITORINFO) };
        if !ok.as_bool() {
            return None;
        }
        let rc = info.monitorInfo.rcMonitor;
        Some(MonitorInfo {
            id: device_name(&info),
            label: format!("Pantalla {}", index + 1),
            width: (rc.right - rc.left).max(0) as u32,
            height: (rc.bottom - rc.top).max(0) as u32,
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
            thumb: snapshot(screen_dc, rc),
        })
    }

    // Foto fija de una pantalla por GDI: BitBlt reescalado a un BMP pequeño que se
    // devuelve como data URL. Es para la miniatura del selector, no para capturar.
    fn snapshot(screen_dc: HDC, rc: RECT) -> Option<String> {
        if screen_dc.is_invalid() {
            return None;
        }
        let sw = rc.right - rc.left;
        let sh = rc.bottom - rc.top;
        if sw <= 0 || sh <= 0 {
            return None;
        }
        let tw: i32 = 320;
        let th: i32 = ((tw as i64 * sh as i64) / sw as i64).max(1) as i32;

        unsafe {
            let mem = CreateCompatibleDC(Some(screen_dc));
            if mem.is_invalid() {
                return None;
            }
            let bmp = CreateCompatibleBitmap(screen_dc, tw, th);
            if bmp.is_invalid() {
                let _ = DeleteDC(mem);
                return None;
            }
            let mem_dc = HDC(mem.0);
            let old = SelectObject(mem_dc, bmp.into());
            SetStretchBltMode(mem_dc, HALFTONE);
            let blit = StretchBlt(
                mem_dc, 0, 0, tw, th, Some(screen_dc), rc.left, rc.top, sw, sh, SRCCOPY,
            );

            let result = if blit.as_bool() {
                let mut bmi = BITMAPINFO::default();
                bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                bmi.bmiHeader.biWidth = tw;
                bmi.bmiHeader.biHeight = th; // positivo => bottom-up, como espera el BMP
                bmi.bmiHeader.biPlanes = 1;
                bmi.bmiHeader.biBitCount = 32;
                bmi.bmiHeader.biCompression = 0; // BI_RGB
                let mut pixels = vec![0u8; (tw * th * 4) as usize];
                let lines = GetDIBits(
                    mem_dc,
                    bmp,
                    0,
                    th as u32,
                    Some(pixels.as_mut_ptr() as *mut _),
                    &mut bmi,
                    DIB_RGB_COLORS,
                );
                (lines != 0).then(|| bmp_data_url(tw, th, &pixels))
            } else {
                None
            };

            SelectObject(mem_dc, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(mem);
            result
        }
    }

    fn bmp_data_url(w: i32, h: i32, pixels: &[u8]) -> String {
        use base64::Engine;
        let pix_size = pixels.len() as u32;
        let file_size = 14 + 40 + pix_size;
        let mut buf = Vec::with_capacity(file_size as usize);
        buf.extend_from_slice(b"BM");
        buf.extend_from_slice(&file_size.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&54u32.to_le_bytes());
        buf.extend_from_slice(&40u32.to_le_bytes());
        buf.extend_from_slice(&w.to_le_bytes());
        buf.extend_from_slice(&h.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&32u16.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&pix_size.to_le_bytes());
        buf.extend_from_slice(&0i32.to_le_bytes());
        buf.extend_from_slice(&0i32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(pixels);
        let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
        format!("data:image/bmp;base64,{b64}")
    }

    // El nombre de dispositivo (\\.\DISPLAY1) es estable en la sesión: lo usamos
    // como id para volver a resolver el HMONITOR al arrancar la captura.
    fn resolve_monitor(id: &str) -> Option<HMONITOR> {
        enum_monitors().into_iter().find(|&hmon| {
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            let ok = unsafe { GetMonitorInfoW(hmon, &mut info as *mut _ as *mut MONITORINFO) };
            ok.as_bool() && device_name(&info) == id
        })
    }

    fn device_name(info: &MONITORINFOEXW) -> String {
        let len = info
            .szDevice
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(info.szDevice.len());
        String::from_utf16_lossy(&info.szDevice[..len])
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn clamp_fps_bounds() {
            assert_eq!(clamp_fps(5), 10);
            assert_eq!(clamp_fps(60), 60);
            assert_eq!(clamp_fps(1000), 240);
        }

        #[test]
        fn fps_interval_values() {
            assert_eq!(fps_interval(0), 0);
            assert_eq!(fps_interval(60), 166_666);
            assert_eq!(fps_interval(240), 41_666);
        }

        #[test]
        fn cfr_pts_has_no_drift() {
            // El slot `fps` cae exactamente en el segundo, sin acumular error aunque 10⁷/fps no
            // sea entero: es la garantía del CFR por índice de slot.
            for fps in [20u32, 30, 60, 120, 144, 240] {
                assert_eq!(cfr_pts(fps as i64, fps), 10_000_000, "fps={fps}");
            }
            assert_eq!(cfr_pts(0, 60), 0);
            // Monotonía estricta.
            assert!(cfr_pts(2, 60) > cfr_pts(1, 60));
        }

        #[test]
        fn time_slot_snaps_to_nearest() {
            let iv = fps_interval(60); // 166_666
            // Frame en el instante base => slot 0.
            assert_eq!(time_slot(1_000, 1_000, iv), 0);
            // Antes del base (jitter) => nunca negativo.
            assert_eq!(time_slot(500, 1_000, iv), 0);
            // Redondeo al slot más cercano, no floor: 0.6 de slot cae en el 1.
            assert_eq!(time_slot(iv * 6 / 10, 0, iv), 1);
            assert_eq!(time_slot(iv * 4 / 10, 0, iv), 0);
            // Fuente a 60 exactos: cada frame en su slot, monótono y sin colisiones.
            for k in 0..120i64 {
                assert_eq!(time_slot(k * iv, 0, iv), k, "k={k}");
            }
            // Fuente a 30 en clip de 60: los frames caen en slots pares (0,2,4…) => el
            // duplicado de relleno los convierte en 60 sin batido.
            let iv30 = fps_interval(30);
            for k in 0..60i64 {
                assert_eq!(time_slot(k * iv30, 0, iv), k * 2, "k={k}");
            }
        }

        #[test]
        fn enc_buffer_frames_clamped() {
            assert_eq!(enc_buffer_frames(10), 6);
            assert_eq!(enc_buffer_frames(20), 6);
            assert_eq!(enc_buffer_frames(60), 18);
            assert_eq!(enc_buffer_frames(240), 24);
        }

        #[test]
        fn output_dims_never_upscales() {
            assert_eq!(output_dims(1920, 1080, 0), (1920, 1080));
            assert_eq!(output_dims(1920, 1080, 1080), (1920, 1080));
            assert_eq!(output_dims(1920, 1080, 2160), (1920, 1080));
            assert_eq!(output_dims(1920, 1080, 720), (1280, 720));
            // Redondeo a par manteniendo aspecto.
            assert_eq!(output_dims(1918, 1080, 721), (1278, 720));
        }

        #[test]
        fn resolve_bitrate_override_and_floor() {
            assert_eq!(resolve_bitrate(1920, 1080, 60, 0.40, 5_000_000), 5_000_000);
            // Combo ligero: por debajo del piso de 1 Mbps.
            assert_eq!(target_bitrate(640, 360, 20, 0.15), 1_000_000);
            // Más resolución/fps/calidad => más bitrate (sin depender de la exactitud del f64).
            assert!(target_bitrate(1920, 1080, 60, 0.40) > target_bitrate(1280, 720, 60, 0.40));
            assert!(target_bitrate(1920, 1080, 60, 0.40) > target_bitrate(1920, 1080, 30, 0.40));
        }

        #[test]
        fn peak_bitrate_is_1_75x() {
            assert_eq!(peak_bitrate(40_000_000), 70_000_000);
            assert_eq!(peak_bitrate(0), 0);
        }

        #[test]
        fn selector_keeps_one_frame_per_slot() {
            let sel = SlotSelector::new();
            let interval = 100;
            // Primer frame: abre el slot 0.
            assert!(sel.keep(1000, interval));
            // Dentro del mismo slot: se descarta (no dos frames en un slot).
            assert!(!sel.keep(1040, interval));
            // Slot 1 (centro 1100, redondeo al más cercano): se guarda.
            assert!(sel.keep(1100, interval));
            // Otro dentro del slot 1: se descarta.
            assert!(!sel.keep(1130, interval));
            // interval<=0 => sin límite.
            assert!(sel.keep(0, 0));
        }

        // Núcleo del arreglo del judder: dos frames casi contiguos (burst sub-slot que WGC entrega
        // al presentar dos veces) caen en el mismo slot => solo se conserva UNO. La reja anterior,
        // desfasada de la del pump, podía conservar ambos (par casi-duplicado = batido).
        #[test]
        fn selector_collapses_subslot_burst() {
            let sel = SlotSelector::new();
            let interval = 166_666; // 60 fps
            assert!(sel.keep(0, interval)); // slot 0
            assert!(!sel.keep(1_000, interval)); // +0.1 ms: mismo slot
            assert!(!sel.keep(2_000, interval)); // +0.2 ms: mismo slot
            assert!(sel.keep(interval, interval)); // slot 1
        }

        // Un juego a 110 fps muestreado a 60: ningún par de frames conservados cae en el mismo slot
        // (sin agrupamiento) y el recuento queda en ~60. Es el caso que se veía a tirones.
        #[test]
        fn selector_no_clustering_at_110fps() {
            let sel = SlotSelector::new();
            let interval = fps_interval(60);
            let src_period = 10_000_000f64 / 110.0;
            let mut base = None;
            let mut last_slot = i64::MIN;
            let mut kept = 0;
            for k in 0..110i64 {
                let t = (k as f64 * src_period) as i64;
                if sel.keep(t, interval) {
                    kept += 1;
                    let slot = time_slot(t, *base.get_or_insert(t), interval);
                    // Estrictamente creciente: nunca dos frames en el mismo slot.
                    assert!(slot > last_slot, "slot {slot} no supera a {last_slot}");
                    last_slot = slot;
                }
            }
            assert!((59..=61).contains(&kept), "kept={kept} fuera de ~60");
        }

        // Hueco real (juego congelado/minimizado): al reanudar, el slot salta y se conserva el
        // frame; el pump absorbe el salto con su guarda STALL. No hay spam ni recuento inflado.
        #[test]
        fn selector_jumps_after_freeze() {
            let sel = SlotSelector::new();
            let interval = 100;
            assert!(sel.keep(0, interval)); // slot 0
            assert!(sel.keep(2_000, interval)); // hueco de 20 slots: se conserva el primero tras el hueco
            assert!(!sel.keep(2_030, interval)); // mismo slot que el anterior: se descarta
        }

        // El guardado ancla el clip al IDR anterior al inicio de la ventana (hasta ~1 GOP atrás).
        // El ring de audio debe conservar historia hasta ese keyframe; si no, el clip arranca con
        // vídeo pero sin audio (el hueco de "el primer medio segundo").
        #[test]
        fn audio_reaches_back_to_video_anchor_keyframe() {
            let fps = 20u32;
            let mut buf = ReplayBuffer::new(2, 1920, 1080, fps, 8_000_000); // ventana 2 s
            buf.init_audio(Some((48_000, 2)), None);

            let vframe = 10_000_000 / fps as i64; // 1/fps en unidades de 100 ns
            let gop = fps as i64; // keyframe cada ~1 s
            // 5,5 s de vídeo: fuerza la poda con el cutoff cayendo entre dos keyframes.
            let nframes = fps as i64 * 55 / 10;
            for i in 0..nframes {
                buf.push(vec![0u8; 16], i * vframe, vframe, i % gop == 0);
            }
            // Audio continuo (~21 ms por paquete AAC a 48 kHz) sobre el mismo lapso.
            let aframe = 1024 * 10_000_000 / 48_000;
            let mut t = 0i64;
            while t <= (nframes - 1) * vframe {
                buf.push_audio(AudioRole::Sys, vec![0u8; 8], t, aframe);
                t += aframe;
            }

            let anchor = buf.packets.iter().find(|p| p.key).map(|p| p.time).unwrap();
            let earliest_audio = buf.sys_audio.as_ref().unwrap().packets.front().unwrap().time;
            assert!(
                earliest_audio <= anchor,
                "audio arranca en {earliest_audio} pero el clip se ancla al keyframe {anchor}: \
                 hueco inicial sin audio",
            );
        }

        // El muxer en directo no debe abrir el SinkWriter hasta tener las cabeceras de vídeo Y de
        // cada pista de audio esperada. (La validez del MP4 con bitstream H.264/AAC real la cubre
        // la verificación manual; aquí, con paquetes sintéticos, solo se prueba el handshake.)
        #[test]
        fn livemux_waits_for_headers_before_writing() {
            ensure_mf();
            let path = std::env::temp_dir()
                .join("flashback_livemux_test1.mp4")
                .to_string_lossy()
                .into_owned();
            let _ = std::fs::remove_file(&path);
            let mux = LiveMux::new(path.clone(), 1920, 1080, 30, 4_000_000, Some((48_000, 2)), None);

            mux.set_seq_header(vec![0u8; 32]);
            mux.push_video(vec![0u8; 64], 0, 333_333, true);
            assert!(
                !mux.is_writing(),
                "no debe arrancar sin la cabecera de la pista de audio esperada"
            );

            mux.set_audio_header(AudioRole::Sys, vec![0u8; 2], 0);
            mux.push_audio(AudioRole::Sys, vec![0u8; 16], 0, 213_333);
            assert!(mux.is_writing(), "con vídeo + cabecera de audio debe estar escribiendo");

            let _ = mux.finalize(); // no se asegura un MP4 válido con datos falsos
            let _ = std::fs::remove_file(&path);
        }

        // Si una pista de audio esperada nunca reporta cabecera, el timeout la descarta y el resto
        // arranca igual (un micrófono muerto no debe bloquear el archivo para siempre).
        #[test]
        fn livemux_timeout_drops_missing_audio_track() {
            ensure_mf();
            let path = std::env::temp_dir()
                .join("flashback_livemux_test2.mp4")
                .to_string_lossy()
                .into_owned();
            let _ = std::fs::remove_file(&path);
            let mux = LiveMux::new(
                path.clone(),
                1920,
                1080,
                30,
                4_000_000,
                Some((48_000, 2)),
                Some((48_000, 1)),
            );
            mux.set_header_timeout(Duration::from_millis(0));

            mux.set_seq_header(vec![0u8; 32]);
            mux.set_audio_header(AudioRole::Sys, vec![0u8; 2], 0);
            mux.push_video(vec![0u8; 64], 0, 333_333, true);
            assert!(
                mux.is_writing(),
                "con timeout vencido debe arrancar descartando el micrófono"
            );
            // El micrófono sin cabecera quedó descartado: no debe existir su stream.
            assert!(mux.st.lock().unwrap().mic_stream.is_none());

            let _ = mux.finalize();
            let _ = std::fs::remove_file(&path);
        }

        // Sin vídeo no hay base temporal: no se abre archivo y finalize devuelve None.
        #[test]
        fn livemux_no_video_returns_none() {
            ensure_mf();
            let mux =
                LiveMux::new("nonexistent.mp4".into(), 1920, 1080, 30, 4_000_000, Some((48_000, 2)), None);
            assert_eq!(mux.finalize(), None);
        }
    }
}
