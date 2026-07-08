#[cfg(target_os = "windows")]
pub use win::{Toast, ToastData, ToastKind};

#[cfg(target_os = "windows")]
mod win {
    use std::cell::RefCell;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread;

    use windows::core::{w, Interface, Result};
    use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::Graphics::Direct2D::Common::{
        D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F,
    };
    use windows::Win32::Graphics::Direct2D::{
        D2D1CreateDevice, ID2D1DeviceContext, ID2D1SolidColorBrush, D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
        D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
        D2D1_ROUNDED_RECT,
    };
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
    };
    use windows::Win32::Graphics::DirectComposition::{
        DCompositionCreateDevice, IDCompositionDevice, IDCompositionSurface, IDCompositionTarget,
        IDCompositionVisual,
    };
    use windows::Win32::Graphics::Dxgi::Common::{
        DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM,
    };
    use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGISurface};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, HMONITOR, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, LoadCursorW,
        PostMessageW, PostQuitMessage, RegisterClassW, SetTimer, SetWindowPos, ShowWindow,
        TranslateMessage, HWND_TOPMOST, IDC_ARROW, MSG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SW_HIDE, SW_SHOWNOACTIVATE, WM_APP, WM_DESTROY, WM_TIMER, WNDCLASSW, WS_EX_NOACTIVATE,
        WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum ToastKind {
        Info,
        Ready,
        Saved,
        Error,
    }

    impl ToastKind {
        pub fn from_str(s: &str) -> ToastKind {
            match s {
                "ready" => ToastKind::Ready,
                "saved" => ToastKind::Saved,
                "error" => ToastKind::Error,
                _ => ToastKind::Info,
            }
        }
    }

    // Los campos se transportan de extremo a extremo ya, pero aún no se leen: el render de
    // esta tarea pinta una lengüeta sólida sin texto ni logo (eso llega en tareas posteriores).
    #[allow(dead_code)]
    #[derive(Clone)]
    pub struct ToastData {
        pub title: String,
        pub body: String,
        pub keys: Vec<String>,
        pub kind: ToastKind,
    }

    #[allow(dead_code)]
    enum Cmd {
        Show(ToastData),
        Hide,
    }

    const WM_APP_WAKE: u32 = WM_APP + 1;
    const TIMER_ID: usize = 1;
    const HIDE_MS: u32 = 2800;
    const TAB_W: f32 = 360.0;
    const TAB_H: f32 = 76.0;
    const CORNER: f32 = 8.0;
    const TOP_MARGIN: f32 = 12.0;

    thread_local! {
        static RENDERER: RefCell<Option<Renderer>> = const { RefCell::new(None) };
        static RX: RefCell<Option<Receiver<Cmd>>> = const { RefCell::new(None) };
    }

    pub struct Toast {
        tx: Sender<Cmd>,
        hwnd: isize,
    }

    impl Toast {
        pub fn spawn() -> Toast {
            let (tx, rx) = mpsc::channel::<Cmd>();
            let (hwnd_tx, hwnd_rx) = mpsc::channel::<isize>();

            thread::spawn(move || unsafe {
                let hwnd = match create_window() {
                    Ok(h) => h,
                    Err(_) => {
                        let _ = hwnd_tx.send(0);
                        return;
                    }
                };
                let renderer = match Renderer::new(hwnd) {
                    Ok(r) => r,
                    Err(_) => {
                        let _ = hwnd_tx.send(0);
                        return;
                    }
                };
                RENDERER.with(|c| *c.borrow_mut() = Some(renderer));
                RX.with(|c| *c.borrow_mut() = Some(rx));
                let _ = hwnd_tx.send(hwnd.0 as isize);

                let mut msg = MSG::default();
                loop {
                    let r = GetMessageW(&mut msg, None, 0, 0);
                    if r.0 <= 0 {
                        break;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            });

            let hwnd = hwnd_rx.recv().unwrap_or(0);
            Toast { tx, hwnd }
        }

        pub fn show(&self, data: ToastData) {
            if self.tx.send(Cmd::Show(data)).is_ok() {
                self.wake();
            }
        }

        pub fn hide(&self) {
            if self.tx.send(Cmd::Hide).is_ok() {
                self.wake();
            }
        }

        fn wake(&self) {
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(self.hwnd as *mut _)),
                    WM_APP_WAKE,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    }

    struct Renderer {
        hwnd: HWND,
        _d3d: ID3D11Device,
        ctx: ID2D1DeviceContext,
        brush: ID2D1SolidColorBrush,
        dcomp: IDCompositionDevice,
        _target: IDCompositionTarget,
        visual: IDCompositionVisual,
        surface: Option<IDCompositionSurface>,
        surf_w: u32,
        surf_h: u32,
    }

    impl Renderer {
        unsafe fn new(hwnd: HWND) -> Result<Renderer> {
            let mut device: Option<ID3D11Device> = None;
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
            let d3d = device.expect("D3D11CreateDevice no devolvió device");

            let dxgi: IDXGIDevice = d3d.cast()?;
            let d2d_device = D2D1CreateDevice(&dxgi, None)?;
            let ctx = d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let brush = ctx.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0x14 as f32 / 255.0,
                    g: 0x14 as f32 / 255.0,
                    b: 0x16 as f32 / 255.0,
                    a: 0.98,
                },
                None,
            )?;

            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
            let visual = dcomp.CreateVisual()?;
            target.SetRoot(&visual)?;

            Ok(Renderer {
                hwnd,
                _d3d: d3d,
                ctx,
                brush,
                dcomp,
                _target: target,
                visual,
                surface: None,
                surf_w: 0,
                surf_h: 0,
            })
        }

        unsafe fn ensure_surface(&mut self, w: u32, h: u32) -> Result<()> {
            if self.surface.is_some() && self.surf_w == w && self.surf_h == h {
                return Ok(());
            }
            let surface = self.dcomp.CreateSurface(
                w,
                h,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                DXGI_ALPHA_MODE_PREMULTIPLIED,
            )?;
            self.visual.SetContent(&surface)?;
            self.surface = Some(surface);
            self.surf_w = w;
            self.surf_h = h;
            Ok(())
        }

        // Pinta la lengüeta en la superficie de DirectComposition. La superficie puede
        // vivir en un atlas interno de DComp, así que BeginDraw devuelve un desfase que hay
        // que sumar a las coordenadas de dibujo. El rectángulo se extiende `radius` más a la
        // derecha del ancho real para que DComp recorte las esquinas derechas y solo queden
        // redondeadas las de la izquierda.
        unsafe fn render(&mut self, w: u32, h: u32, scale: f32) -> Result<()> {
            self.ensure_surface(w, h)?;
            let radius = CORNER * scale;

            let surface = self.surface.as_ref().unwrap();
            let mut offset = POINT::default();
            let dxgi: IDXGISurface = surface.BeginDraw(None, &mut offset)?;

            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                ..Default::default()
            };
            let bmp = self.ctx.CreateBitmapFromDxgiSurface(&dxgi, Some(&props))?;

            let ox = offset.x as f32;
            let oy = offset.y as f32;
            let rect = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: ox,
                    top: oy,
                    right: ox + w as f32 + radius,
                    bottom: oy + h as f32,
                },
                radiusX: radius,
                radiusY: radius,
            };

            self.ctx.SetTarget(&bmp);
            self.ctx.BeginDraw();
            self.ctx.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));
            self.ctx.FillRoundedRectangle(&rect, &self.brush);
            self.ctx.EndDraw(None, None)?;
            self.ctx.SetTarget(None);

            surface.EndDraw()?;
            self.dcomp.Commit()?;
            Ok(())
        }

        unsafe fn show(&mut self) {
            let mon = primary_monitor();
            let scale = monitor_scale(mon);
            let w = (TAB_W * scale).round() as i32;
            let h = (TAB_H * scale).round() as i32;
            place(self.hwnd, w, h, scale);
            if self.render(w as u32, h as u32, scale).is_err() {
                return;
            }
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            );
            SetTimer(Some(self.hwnd), TIMER_ID, HIDE_MS, None);
        }

        unsafe fn hide(&self) {
            let _ = KillTimer(Some(self.hwnd), TIMER_ID);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    unsafe fn create_window() -> Result<HWND> {
        let class = w!("FlashbackToastWindow");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&wc);
        CreateWindowExW(
            WS_EX_TOPMOST
                | WS_EX_TRANSPARENT
                | WS_EX_NOACTIVATE
                | WS_EX_TOOLWINDOW
                | WS_EX_NOREDIRECTIONBITMAP,
            class,
            w!("Flashback"),
            WS_POPUP,
            0,
            0,
            10,
            10,
            None,
            None,
            None,
            None,
        )
    }

    unsafe fn primary_monitor() -> HMONITOR {
        MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY)
    }

    unsafe fn monitor_scale(mon: HMONITOR) -> f32 {
        let mut dpi_x = 96u32;
        let mut dpi_y = 96u32;
        if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_ok() {
            dpi_x as f32 / 96.0
        } else {
            1.0
        }
    }

    unsafe fn place(hwnd: HWND, w: i32, h: i32, scale: f32) {
        let mon = primary_monitor();
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return;
        }
        let x = mi.rcWork.right - w;
        let y = mi.rcWork.top + (TOP_MARGIN * scale).round() as i32;
        let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOACTIVATE);
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_APP_WAKE => {
                RX.with(|rx| {
                    if let Some(rx) = rx.borrow().as_ref() {
                        while let Ok(cmd) = rx.try_recv() {
                            RENDERER.with(|r| {
                                if let Some(rend) = r.borrow_mut().as_mut() {
                                    match cmd {
                                        Cmd::Show(_) => rend.show(),
                                        Cmd::Hide => rend.hide(),
                                    }
                                }
                            });
                        }
                    }
                });
                LRESULT(0)
            }
            WM_TIMER => {
                RENDERER.with(|r| {
                    if let Some(rend) = r.borrow().as_ref() {
                        rend.hide();
                    }
                });
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn kind_parsing_defaults_to_info() {
            assert_eq!(ToastKind::from_str("error"), ToastKind::Error);
            assert_eq!(ToastKind::from_str("ready"), ToastKind::Ready);
            assert_eq!(ToastKind::from_str("saved"), ToastKind::Saved);
            assert_eq!(ToastKind::from_str("nonsense"), ToastKind::Info);
        }
    }
}
