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
pub use win::{list_audio_inputs, list_monitors, start, status, stop};

#[cfg(not(target_os = "windows"))]
pub fn list_monitors() -> Vec<MonitorInfo> {
    Vec::new()
}

#[cfg(not(target_os = "windows"))]
pub fn list_audio_inputs() -> Vec<AudioInput> {
    Vec::new()
}

#[cfg(not(target_os = "windows"))]
pub fn start(_monitor_id: String, _out_dir: String) -> Result<(), String> {
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

#[cfg(target_os = "windows")]
mod win {
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
    use std::sync::{mpsc, Arc, Condvar, Mutex, Once};
    use std::thread::JoinHandle;
    use std::time::Instant;

    use windows::core::{IInspectable, Interface, Result, BOOL, HSTRING};
    use windows::Devices::Enumeration::{DeviceClass, DeviceInformation};
    use windows::Foundation::TypedEventHandler;
    use windows::Graphics::Capture::{
        Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
    };
    use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
    use windows::Graphics::DirectX::DirectXPixelFormat;
    use windows::Win32::Foundation::{HMODULE, LPARAM, RECT, SYSTEMTIME};
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D,
        D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_FLAG,
        D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
        D3D11_RESOURCE_MISC_FLAG, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    };
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
    use windows::Win32::Graphics::Dxgi::IDXGIDevice;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, EnumDisplayMonitors,
        GetDC, GetDIBits, GetMonitorInfoW, ReleaseDC, SelectObject, SetStretchBltMode, StretchBlt,
        BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HALFTONE, HDC, HMONITOR, MONITORINFO,
        MONITORINFOEXW, SRCCOPY,
    };
    use windows::Win32::Media::MediaFoundation::*;

    // Valor Win32 de MONITORINFOF_PRIMARY (no lo genera el crate windows).
    const MONITORINFOF_PRIMARY: u32 = 1;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
    use windows::Win32::System::SystemInformation::GetLocalTime;
    use windows::Win32::System::WinRT::Direct3D11::{
        CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
    };
    use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

    use super::{AudioInput, CaptureStatus, MonitorInfo};

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

    pub fn start(monitor_id: String, out_dir: String) -> std::result::Result<(), String> {
        let mut guard = STATE.lock().unwrap();
        if guard.is_some() {
            return Ok(());
        }

        let stats = Arc::new(Stats::default());
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let result = Arc::new(Mutex::new(None));
        let (ready_tx, ready_rx) = mpsc::channel::<std::result::Result<(), String>>();

        let stats_t = stats.clone();
        let stop_t = stop.clone();
        let result_t = result.clone();
        let handle = std::thread::Builder::new()
            .name("flashback-capture".into())
            .spawn(move || capture_thread(monitor_id, out_dir, stop_t, stats_t, result_t, ready_tx))
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
    fn capture_thread(
        monitor_id: String,
        out_dir: String,
        stop: Arc<(Mutex<bool>, Condvar)>,
        stats: Arc<Stats>,
        result: Arc<Mutex<Option<String>>>,
        ready: mpsc::Sender<std::result::Result<(), String>>,
    ) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        let engine = match resolve_monitor(&monitor_id)
            .ok_or_else(|| "Monitor no encontrado".to_string())
            .and_then(|hmon| build_engine(&stats, hmon, &out_dir).map_err(|e| format!("{e:?}")))
        {
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

        let (lock, cv) = &*stop;
        let mut stopped = lock.lock().unwrap();
        while !*stopped {
            stopped = cv.wait(stopped).unwrap();
        }
        drop(stopped);

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
    }

    impl Engine {
        fn shutdown(&self) {
            let _ = self.frame_pool.RemoveFrameArrived(self.token);
            let _ = self.session.Close();
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

    fn build_engine(stats: &Arc<Stats>, monitor: HMONITOR, out_dir: &str) -> Result<Engine> {
        let (device, d3d_device) = create_device()?;
        let item = capture_item_for_monitor(monitor)?;
        let size = item.Size()?;
        let width = size.Width.max(1) as u32;
        let height = size.Height.max(1) as u32;

        let out_path = format!("{out_dir}\\{}", clip_filename());
        let encoder = Arc::new(Mutex::new(Encoder::new(&device, width, height, out_path)?));

        let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &d3d_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )?;
        let session = frame_pool.CreateCaptureSession(&item)?;

        // El handler corre en el pool de hilos del sistema (frame pool free-threaded):
        // recoge la textura del frame y la empuja al encoder por hardware. La textura
        // WGC se copia GPU→GPU dentro del encoder (no baja a CPU): el camino sagrado.
        let stats = stats.clone();
        let enc = encoder.clone();
        let handler = TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(
            move |pool, _| {
                if let Some(pool) = pool.as_ref() {
                    if let Ok(frame) = pool.TryGetNextFrame() {
                        if let Ok(s) = frame.ContentSize() {
                            stats.width.store(s.Width.max(0) as u32, Ordering::Relaxed);
                            stats.height.store(s.Height.max(0) as u32, Ordering::Relaxed);
                        }
                        if let Ok(surface) = frame.Surface() {
                            if let Ok(access) = surface.cast::<IDirect3DDxgiInterfaceAccess>() {
                                if let Ok(tex) = unsafe { access.GetInterface::<ID3D11Texture2D>() } {
                                    let t = frame
                                        .SystemRelativeTime()
                                        .map(|x| x.Duration)
                                        .unwrap_or(0);
                                    let _ = enc.lock().unwrap().push(&tex, t);
                                }
                            }
                        }
                        let _ = frame.Close();
                        stats.frames.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Ok(())
            },
        );
        let token = frame_pool.FrameArrived(&handler)?;
        session.StartCapture()?;

        Ok(Engine {
            _device: device,
            frame_pool,
            session,
            token,
            encoder,
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
        ctx: ID3D11DeviceContext,
        pool: Vec<ID3D11Texture2D>,
        next: usize,
        base: i64,
        last: i64,
        has_base: bool,
        path: String,
        finalized: bool,
    }

    // El handler de FrameArrived exige Send+Sync. El Encoder solo se toca bajo el
    // Mutex y desde el callback (que WGC serializa), con el device protegido para
    // multihilo, así que moverlo entre hilos de forma sincronizada es seguro.
    unsafe impl Send for Encoder {}

    impl Encoder {
        fn new(device: &ID3D11Device, width: u32, height: u32, path: String) -> Result<Encoder> {
            ensure_mf();

            // Compartir el mismo device con MF: así el encoder lee nuestras texturas
            // sin copiarlas a CPU. El device debe estar protegido para multihilo.
            let mut token = 0u32;
            let mut manager: Option<IMFDXGIDeviceManager> = None;
            unsafe { MFCreateDXGIDeviceManager(&mut token, &mut manager)? };
            let manager = manager.unwrap();
            unsafe { manager.ResetDevice(device, token)? };

            let ctx = unsafe { device.GetImmediateContext()? };
            if let Ok(mt) = ctx.cast::<ID3D11Multithread>() {
                let _ = unsafe { mt.SetMultithreadProtected(true) };
            }

            let attrs = unsafe {
                let mut a: Option<IMFAttributes> = None;
                MFCreateAttributes(&mut a, 3)?;
                let a = a.unwrap();
                a.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
                a.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
                a.SetUnknown(&MF_SINK_WRITER_D3D_MANAGER, &manager)?;
                a
            };

            let url = HSTRING::from(path.as_str());
            let writer = unsafe { MFCreateSinkWriterFromURL(&url, None, &attrs)? };

            // fps nominal: WGC no entrega duplicados, así que el ritmo real lo marcan
            // los timestamps por muestra; 60 es solo metadato/objetivo del rate control.
            let fps = 60u32;
            let bitrate = (((width as u64 * height as u64 * fps as u64) as f64 * 0.08) as u32)
                .max(2_000_000);

            let out_type = unsafe { MFCreateMediaType()? };
            unsafe {
                out_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                out_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
                out_type.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
                out_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                out_type.SetUINT64(&MF_MT_FRAME_SIZE, pack2(width, height))?;
                out_type.SetUINT64(&MF_MT_FRAME_RATE, pack2(fps, 1))?;
                out_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack2(1, 1))?;
            }
            let stream = unsafe { writer.AddStream(&out_type)? };

            let in_type = unsafe { MFCreateMediaType()? };
            unsafe {
                in_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                in_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_ARGB32)?;
                in_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                in_type.SetUINT64(&MF_MT_FRAME_SIZE, pack2(width, height))?;
                in_type.SetUINT64(&MF_MT_FRAME_RATE, pack2(fps, 1))?;
                in_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack2(1, 1))?;
                writer.SetInputMediaType(stream, &in_type, None)?;
            }

            unsafe { writer.BeginWriting()? };

            // Anillo de texturas propias: WGC reutiliza las suyas en cuanto soltamos
            // el frame, pero el encoder es asíncrono y puede leerlas más tarde; copiar
            // a una textura nuestra (rotando varias) evita esa carrera.
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
                CPUAccessFlags: D3D11_CPU_ACCESS_FLAG(0).0 as u32,
                MiscFlags: D3D11_RESOURCE_MISC_FLAG(0).0 as u32,
            };
            let mut pool = Vec::with_capacity(6);
            for _ in 0..6 {
                let mut t: Option<ID3D11Texture2D> = None;
                unsafe { device.CreateTexture2D(&desc, None, Some(&mut t))? };
                pool.push(t.unwrap());
            }

            Ok(Encoder {
                writer,
                stream,
                ctx,
                pool,
                next: 0,
                base: 0,
                last: 0,
                has_base: false,
                path,
                finalized: false,
            })
        }

        fn push(&mut self, src: &ID3D11Texture2D, time: i64) -> Result<()> {
            let ts = if self.has_base {
                time - self.base
            } else {
                self.has_base = true;
                self.base = time;
                0
            };

            let dst = self.pool[self.next].clone();
            self.next = (self.next + 1) % self.pool.len();
            unsafe { self.ctx.CopyResource(&dst, src) };

            let buffer =
                unsafe { MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, &dst, 0, false)? };
            let len = unsafe { buffer.cast::<IMF2DBuffer>()?.GetContiguousLength()? };
            unsafe { buffer.SetCurrentLength(len)? };

            let sample = unsafe { MFCreateSample()? };
            let dur = if ts > self.last { ts - self.last } else { 166_667 };
            self.last = ts;
            unsafe {
                sample.AddBuffer(&buffer)?;
                sample.SetSampleTime(ts)?;
                sample.SetSampleDuration(dur)?;
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

    fn pack2(high: u32, low: u32) -> u64 {
        ((high as u64) << 32) | low as u64
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
}
