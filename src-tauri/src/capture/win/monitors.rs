use super::*;

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
pub(super) fn resolve_target_item(target: &str) -> std::result::Result<GraphicsCaptureItem, String> {
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
pub(super) fn resolve_game_window() -> Option<HWND> {
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

pub(super) fn enum_monitors() -> Vec<HMONITOR> {
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

pub(super) fn monitor_info(hmon: HMONITOR, index: usize, screen_dc: HDC) -> Option<MonitorInfo> {
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
