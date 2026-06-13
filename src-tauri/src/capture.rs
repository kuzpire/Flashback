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
pub fn start(_monitor_id: String) -> Result<(), String> {
    Err("La captura solo está disponible en Windows".into())
}

#[cfg(not(target_os = "windows"))]
pub fn stop() {}

#[cfg(not(target_os = "windows"))]
pub fn status() -> CaptureStatus {
    CaptureStatus::default()
}

#[cfg(target_os = "windows")]
mod win {
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
    use std::sync::{mpsc, Arc, Condvar, Mutex};
    use std::thread::JoinHandle;
    use std::time::Instant;

    use windows::core::{IInspectable, Interface, Result, BOOL};
    use windows::Devices::Enumeration::{DeviceClass, DeviceInformation};
    use windows::Foundation::TypedEventHandler;
    use windows::Graphics::Capture::{
        Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
    };
    use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
    use windows::Graphics::DirectX::DirectXPixelFormat;
    use windows::Win32::Foundation::{HMODULE, LPARAM, RECT};
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
    };
    use windows::Win32::Graphics::Dxgi::IDXGIDevice;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, EnumDisplayMonitors,
        GetDC, GetDIBits, GetMonitorInfoW, ReleaseDC, SelectObject, SetStretchBltMode, StretchBlt,
        BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HALFTONE, HDC, HMONITOR, MONITORINFO,
        MONITORINFOEXW, SRCCOPY,
    };

    // Valor Win32 de MONITORINFOF_PRIMARY (no lo genera el crate windows).
    const MONITORINFOF_PRIMARY: u32 = 1;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
    use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
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
    }

    static STATE: Mutex<Option<Running>> = Mutex::new(None);

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

    pub fn start(monitor_id: String) -> std::result::Result<(), String> {
        let mut guard = STATE.lock().unwrap();
        if guard.is_some() {
            return Ok(());
        }

        let stats = Arc::new(Stats::default());
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let (ready_tx, ready_rx) = mpsc::channel::<std::result::Result<(), String>>();

        let stats_t = stats.clone();
        let stop_t = stop.clone();
        let handle = std::thread::Builder::new()
            .name("flashback-capture".into())
            .spawn(move || capture_thread(monitor_id, stop_t, stats_t, ready_tx))
            .map_err(|e| e.to_string())?;

        // El hilo construye el pipeline WGC y reporta éxito o error antes de
        // ponerse a recibir frames; así start() puede devolver un fallo real.
        match ready_rx.recv() {
            Ok(Ok(())) => {
                *guard = Some(Running {
                    stop,
                    handle: Some(handle),
                    stats,
                    started: Instant::now(),
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

    pub fn stop() {
        let running = STATE.lock().unwrap().take();
        if let Some(mut running) = running {
            let (lock, cv) = &*running.stop;
            *lock.lock().unwrap() = true;
            cv.notify_all();
            if let Some(h) = running.handle.take() {
                let _ = h.join();
            }
        }
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
        stop: Arc<(Mutex<bool>, Condvar)>,
        stats: Arc<Stats>,
        ready: mpsc::Sender<std::result::Result<(), String>>,
    ) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        let engine = match resolve_monitor(&monitor_id).ok_or_else(|| "Monitor no encontrado".to_string())
            .and_then(|hmon| build_engine(&stats, hmon).map_err(|e| format!("{e:?}")))
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

        drop(engine);
        unsafe { CoUninitialize() };
    }

    struct Engine {
        _device: ID3D11Device,
        frame_pool: Direct3D11CaptureFramePool,
        session: GraphicsCaptureSession,
        token: i64,
    }

    impl Drop for Engine {
        fn drop(&mut self) {
            let _ = self.frame_pool.RemoveFrameArrived(self.token);
            let _ = self.session.Close();
            let _ = self.frame_pool.Close();
        }
    }

    fn build_engine(stats: &Arc<Stats>, monitor: HMONITOR) -> Result<Engine> {
        let (device, d3d_device) = create_device()?;
        let item = capture_item_for_monitor(monitor)?;
        let size = item.Size()?;

        let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &d3d_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )?;
        let session = frame_pool.CreateCaptureSession(&item)?;

        // El handler corre en el pool de hilos del sistema (frame pool free-threaded):
        // recoge el frame para que el anillo siga fluyendo y, de momento, solo cuenta.
        let stats = stats.clone();
        let handler = TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(
            move |pool, _| {
                if let Some(pool) = pool.as_ref() {
                    if let Ok(frame) = pool.TryGetNextFrame() {
                        if let Ok(s) = frame.ContentSize() {
                            stats.width.store(s.Width.max(0) as u32, Ordering::Relaxed);
                            stats.height.store(s.Height.max(0) as u32, Ordering::Relaxed);
                        }
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
        })
    }

    // Device D3D11 con soporte BGRA (obligatorio para el interop de WGC) y su
    // equivalente WinRT IDirect3DDevice, que es lo que consume el frame pool.
    fn create_device() -> Result<(ID3D11Device, IDirect3DDevice)> {
        let mut device: Option<ID3D11Device> = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
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
