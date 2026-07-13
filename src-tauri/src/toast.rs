#[cfg(target_os = "windows")]
pub use win::{Toast, ToastData, ToastKind};

#[cfg(target_os = "windows")]
mod win {
    use std::cell::RefCell;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread;
    use std::time::{Duration, Instant};

    use windows::core::{w, Interface, Result};
    use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::Graphics::Direct2D::Common::{
        D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F,
    };
    use windows::Win32::Graphics::Direct2D::Common::{D2D_SIZE_F, D2D_SIZE_U};
    use windows::Win32::Graphics::Direct2D::{
        D2D1CreateDevice, ID2D1Bitmap1, ID2D1DeviceContext, ID2D1DeviceContext5,
        ID2D1SolidColorBrush, D2D1_ANTIALIAS_MODE_ALIASED, D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
        D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
        D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_ROUNDED_RECT,
    };
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
    };
    use windows::Win32::Graphics::DirectWrite::{
        DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
        DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL,
        DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL,
        DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
        DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_METRICS,
    };
    use windows::Win32::Graphics::DirectComposition::{
        DCompositionCreateDevice, IDCompositionDevice, IDCompositionSurface, IDCompositionTarget,
        IDCompositionVisual,
    };
    use windows::Win32::Graphics::Dxgi::Common::{
        DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM,
    };
    use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGISurface};
    use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, IWICImagingFactory};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, IStream, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
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

    #[derive(Clone)]
    pub struct ToastData {
        pub title: String,
        pub body: String,
        pub keys: Vec<String>,
        pub kind: ToastKind,
    }

    enum Cmd {
        Show(ToastData),
        Hide,
    }

    const WM_APP_WAKE: u32 = WM_APP + 1;
    const TIMER_ID: usize = 1;
    const FRAME_MS: u32 = 16;
    const IN_MS: u64 = 220;
    const OUT_MS: u64 = 300;
    const VISIBLE_MS: u64 = 2800;
    const SLIDE: f32 = 24.0;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Phase {
        Hidden,
        In,
        Visible,
        Out,
    }

    fn ease_out(p: f32) -> f32 {
        1.0 - (1.0 - p).powi(3)
    }

    const TAB_H: f32 = 78.0;
    const CORNER: f32 = 8.0;
    const TOP_MARGIN: f32 = 12.0;
    const TEXT_LEFT: f32 = 60.0;
    const RIGHT_PAD: f32 = 26.0;
    const TITLE_TOP: f32 = 16.0;
    const TITLE_LINE: f32 = 24.0;
    const BODY_TOP: f32 = 40.0;
    const BODY_LINE: f32 = 22.0;
    const BAR_W: f32 = 4.0;

    const MARK_SVG: &str = include_str!("../../static/flashback-mono.svg");

    type Rect = D2D_RECT_F;

    fn keycap_layout(key_widths: &[f32], plus_width: f32, gap: f32, pad_x: f32) -> (Vec<Rect>, f32) {
        let mut rects = Vec::with_capacity(key_widths.len());
        let mut x = 0.0f32;
        for (i, kw) in key_widths.iter().enumerate() {
            if i > 0 {
                x += gap + plus_width + gap;
            }
            let chip_w = kw + pad_x * 2.0;
            rects.push(Rect { left: x, top: 0.0, right: x + chip_w, bottom: 0.0 });
            x += chip_w;
        }
        (rects, x)
    }

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
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
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
        dwrite: IDWriteFactory,
        title_format: IDWriteTextFormat,
        body_format: IDWriteTextFormat,
        chip_format: IDWriteTextFormat,
        text_bright: ID2D1SolidColorBrush,
        text_dim: ID2D1SolidColorBrush,
        title_error: ID2D1SolidColorBrush,
        chip_bg: ID2D1SolidColorBrush,
        chip_text: ID2D1SolidColorBrush,
        bar: ID2D1SolidColorBrush,
        border: ID2D1SolidColorBrush,
        mark: ID2D1Bitmap1,
        dcomp: IDCompositionDevice,
        _target: IDCompositionTarget,
        visual: IDCompositionVisual,
        surface: Option<IDCompositionSurface>,
        surf_w: u32,
        surf_h: u32,
        data: Option<ToastData>,
        phase: Phase,
        start: Instant,
        deadline: Instant,
        cur_w: u32,
        cur_h: u32,
        cur_scale: f32,
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
                    r: 0x08 as f32 / 255.0,
                    g: 0x08 as f32 / 255.0,
                    b: 0x08 as f32 / 255.0,
                    a: 0.98,
                },
                None,
            )?;

            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let title_format = dwrite.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                18.0,
                w!("en-us"),
            )?;
            title_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)?;
            title_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            let body_format = dwrite.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                15.0,
                w!("en-us"),
            )?;
            body_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)?;
            body_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            let chip_format = dwrite.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                12.5,
                w!("en-us"),
            )?;
            chip_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            chip_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;

            let text_bright = ctx.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
                None,
            )?;
            let text_dim = ctx.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0xc8 as f32 / 255.0,
                    g: 0xcf as f32 / 255.0,
                    b: 0xda as f32 / 255.0,
                    a: 1.0,
                },
                None,
            )?;
            let title_error = ctx.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0xff as f32 / 255.0,
                    g: 0x5b as f32 / 255.0,
                    b: 0x5b as f32 / 255.0,
                    a: 1.0,
                },
                None,
            )?;
            let chip_bg = ctx.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0x2a as f32 / 255.0,
                    g: 0x2a as f32 / 255.0,
                    b: 0x30 as f32 / 255.0,
                    a: 1.0,
                },
                None,
            )?;
            let chip_text = ctx.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0xf0 as f32 / 255.0,
                    g: 0xf2 as f32 / 255.0,
                    b: 0xf7 as f32 / 255.0,
                    a: 0.85,
                },
                None,
            )?;
            let bar = ctx.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.88,
                },
                None,
            )?;
            let border = ctx.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0x34 as f32 / 255.0,
                    g: 0x34 as f32 / 255.0,
                    b: 0x36 as f32 / 255.0,
                    a: 1.0,
                },
                None,
            )?;

            let mark = load_mark(&ctx, monitor_scale(primary_monitor()))?;

            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
            let visual = dcomp.CreateVisual()?;
            target.SetRoot(&visual)?;

            Ok(Renderer {
                hwnd,
                _d3d: d3d,
                ctx,
                brush,
                dwrite,
                title_format,
                body_format,
                chip_format,
                text_bright,
                text_dim,
                title_error,
                chip_bg,
                chip_text,
                bar,
                border,
                mark,
                dcomp,
                _target: target,
                visual,
                surface: None,
                surf_w: 0,
                surf_h: 0,
                data: None,
                phase: Phase::Hidden,
                start: Instant::now(),
                deadline: Instant::now(),
                cur_w: 0,
                cur_h: 0,
                cur_scale: 1.0,
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
        unsafe fn render(&mut self, w: u32, h: u32, scale: f32, opacity: f32) -> Result<()> {
            self.ensure_surface(w, h)?;
            let radius = CORNER * scale;

            self.brush.SetOpacity(opacity);
            self.text_bright.SetOpacity(opacity);
            self.text_dim.SetOpacity(opacity);
            self.title_error.SetOpacity(opacity);
            self.chip_bg.SetOpacity(opacity);
            self.chip_text.SetOpacity(opacity);
            self.bar.SetOpacity(opacity);
            self.border.SetOpacity(opacity);

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

            // Hairline de 1px para despegar la tarjeta casi negra de escenas oscuras. El trazo va
            // centrado en el borde, así que se insetan 0.5px para que caiga dentro; el lado derecho
            // se sale de la superficie (esquinas recortadas) y no se ve.
            let bw = 1.0 * scale;
            let border_rect = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: ox + bw * 0.5,
                    top: oy + bw * 0.5,
                    right: ox + w as f32 + radius,
                    bottom: oy + h as f32 - bw * 0.5,
                },
                radiusX: radius - bw * 0.5,
                radiusY: radius - bw * 0.5,
            };
            self.ctx.DrawRoundedRectangle(&border_rect, &self.border, bw, None);

            // Barra de acento en el borde izquierdo, estilo Moments: se repinta el mismo rectángulo
            // redondeado pero recortado a un ancho fijo, así comparte el radio de la esquina
            // izquierda y queda recto por la derecha. Va encima del hairline para tapar su tramo
            // izquierdo y dejar el borde de la tarjeta limpio.
            let bar = D2D_RECT_F {
                left: ox,
                top: oy,
                right: ox + BAR_W * scale,
                bottom: oy + h as f32,
            };
            self.ctx.PushAxisAlignedClip(&bar, D2D1_ANTIALIAS_MODE_ALIASED);
            self.ctx.FillRoundedRectangle(&rect, &self.bar);
            self.ctx.PopAxisAlignedClip();

            let mark_size = 28.0 * scale;
            let mark_left = ox + 16.0 * scale;
            let mark_top = oy + (h as f32 - mark_size) / 2.0;
            let mark_rect = D2D_RECT_F {
                left: mark_left,
                top: mark_top,
                right: mark_left + mark_size,
                bottom: mark_top + mark_size,
            };
            self.ctx.DrawBitmap(
                &self.mark,
                Some(&mark_rect),
                opacity,
                D2D1_INTERPOLATION_MODE_LINEAR,
                None,
                None,
            );

            if let Some(data) = self.data.as_ref() {
                let text_left = ox + TEXT_LEFT * scale;
                let right = ox + w as f32;
                let title: Vec<u16> = data.title.encode_utf16().collect();
                let body: Vec<u16> = data.body.encode_utf16().collect();
                let title_rect = D2D_RECT_F {
                    left: text_left,
                    top: oy + TITLE_TOP * scale,
                    right,
                    bottom: oy + (TITLE_TOP + TITLE_LINE) * scale,
                };
                let title_brush = if data.kind == ToastKind::Error {
                    &self.title_error
                } else {
                    &self.text_bright
                };
                self.ctx.DrawText(
                    &title,
                    &self.title_format,
                    &title_rect,
                    title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                    DWRITE_MEASURING_MODE_NATURAL,
                );

                let mut body_left = text_left;
                if !data.keys.is_empty() {
                    let gap = 4.0 * scale;
                    let pad_x = 7.0 * scale;
                    let chip_h = 20.0 * scale;
                    let chip_radius = 5.0 * scale;
                    let center_y = oy + (BODY_TOP + BODY_LINE / 2.0) * scale;
                    let plus: Vec<u16> = "+".encode_utf16().collect();
                    let plus_w = self.line_width(&plus, &self.body_format);
                    let key_widths: Vec<f32> = data
                        .keys
                        .iter()
                        .map(|k| {
                            let u: Vec<u16> = k.encode_utf16().collect();
                            self.line_width(&u, &self.chip_format)
                        })
                        .collect();
                    let (rects, keys_block_width) =
                        keycap_layout(&key_widths, plus_w, gap, pad_x);
                    for (i, r) in rects.iter().enumerate() {
                        let cl = text_left + r.left;
                        let cr = text_left + r.right;
                        let chip_rect = D2D_RECT_F {
                            left: cl,
                            top: center_y - chip_h / 2.0,
                            right: cr,
                            bottom: center_y + chip_h / 2.0,
                        };
                        let chip = D2D1_ROUNDED_RECT {
                            rect: chip_rect,
                            radiusX: chip_radius,
                            radiusY: chip_radius,
                        };
                        self.ctx.FillRoundedRectangle(&chip, &self.chip_bg);
                        let key_u: Vec<u16> = data.keys[i].encode_utf16().collect();
                        self.ctx.DrawText(
                            &key_u,
                            &self.chip_format,
                            &chip_rect,
                            &self.chip_text,
                            D2D1_DRAW_TEXT_OPTIONS_NONE,
                            DWRITE_MEASURING_MODE_NATURAL,
                        );
                        if i + 1 < rects.len() {
                            let plus_left = cr + gap;
                            let plus_rect = D2D_RECT_F {
                                left: plus_left,
                                top: oy + BODY_TOP * scale,
                                right: plus_left + plus_w,
                                bottom: oy + (BODY_TOP + BODY_LINE) * scale,
                            };
                            self.ctx.DrawText(
                                &plus,
                                &self.body_format,
                                &plus_rect,
                                &self.text_dim,
                                D2D1_DRAW_TEXT_OPTIONS_NONE,
                                DWRITE_MEASURING_MODE_NATURAL,
                            );
                        }
                    }
                    body_left = text_left + keys_block_width + gap;
                }

                let body_rect = D2D_RECT_F {
                    left: body_left,
                    top: oy + BODY_TOP * scale,
                    right,
                    bottom: oy + (BODY_TOP + BODY_LINE) * scale,
                };
                self.ctx.DrawText(
                    &body,
                    &self.body_format,
                    &body_rect,
                    &self.text_dim,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
            }

            self.ctx.EndDraw(None, None)?;
            self.ctx.SetTarget(None);

            surface.EndDraw()?;
            self.dcomp.Commit()?;
            Ok(())
        }

        // Ancho de una línea en DIPs (tamaño de fuente lógico); el llamador lo escala a píxeles.
        unsafe fn line_width(&self, text: &[u16], format: &IDWriteTextFormat) -> f32 {
            let Ok(layout) = self.dwrite.CreateTextLayout(text, format, 1000.0, TAB_H) else {
                return 0.0;
            };
            let mut m = DWRITE_TEXT_METRICS::default();
            if layout.GetMetrics(&mut m).is_ok() {
                m.width
            } else {
                0.0
            }
        }

        unsafe fn measure(&self, data: &ToastData, scale: f32) -> (f32, f32) {
            let title: Vec<u16> = data.title.encode_utf16().collect();
            let body: Vec<u16> = data.body.encode_utf16().collect();
            let title_w = self.line_width(&title, &self.title_format);
            let body_w = self.line_width(&body, &self.body_format);

            let second_line = if data.keys.is_empty() {
                body_w
            } else {
                let gap = 4.0 * scale;
                let pad_x = 7.0 * scale;
                let plus: Vec<u16> = "+".encode_utf16().collect();
                let plus_w = self.line_width(&plus, &self.body_format);
                let key_widths: Vec<f32> = data
                    .keys
                    .iter()
                    .map(|k| {
                        let u: Vec<u16> = k.encode_utf16().collect();
                        self.line_width(&u, &self.chip_format)
                    })
                    .collect();
                let (_rects, keys_block_width) =
                    keycap_layout(&key_widths, plus_w, gap, pad_x);
                keys_block_width + gap + body_w
            };

            let line = title_w.max(second_line);
            let width = TEXT_LEFT + line + RIGHT_PAD;
            (width * scale, TAB_H * scale)
        }

        unsafe fn show(&mut self, data: ToastData) {
            let mon = primary_monitor();
            let scale = monitor_scale(mon);
            let (fw, fh) = self.measure(&data, scale);
            let w = fw.round() as u32;
            let h = fh.round() as u32;
            self.data = Some(data);
            place(self.hwnd, w as i32, h as i32, scale);
            self.cur_w = w;
            self.cur_h = h;
            self.cur_scale = scale;
            self.phase = Phase::In;
            self.start = Instant::now();
            self.deadline = self.start + Duration::from_millis(VISIBLE_MS);
            let _ = self.visual.SetOffsetX2(SLIDE * scale);
            if self.render(w, h, scale, 0.0).is_err() {
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
            SetTimer(Some(self.hwnd), TIMER_ID, FRAME_MS, None);
        }

        // Cierre suave: si el toast está a la vista, arranca la fase Out en lugar de
        // ocultarlo de golpe; el timer de animación completa el fundido.
        unsafe fn hide(&mut self) {
            if self.phase == Phase::In || self.phase == Phase::Visible {
                self.phase = Phase::Out;
                self.start = Instant::now();
            }
        }

        unsafe fn tick(&mut self) {
            let scale = self.cur_scale;
            let (w, h) = (self.cur_w, self.cur_h);
            let elapsed = self.start.elapsed().as_secs_f32() * 1000.0;
            match self.phase {
                Phase::In => {
                    let p = (elapsed / IN_MS as f32).clamp(0.0, 1.0);
                    let t = ease_out(p);
                    let _ = self.visual.SetOffsetX2((1.0 - t) * SLIDE * scale);
                    let _ = self.render(w, h, scale, t);
                    if p >= 1.0 {
                        self.phase = Phase::Visible;
                    }
                }
                Phase::Visible => {
                    if Instant::now() >= self.deadline {
                        self.phase = Phase::Out;
                        self.start = Instant::now();
                    }
                }
                Phase::Out => {
                    let p = (elapsed / OUT_MS as f32).clamp(0.0, 1.0);
                    let t = ease_out(p);
                    let _ = self.visual.SetOffsetX2(t * SLIDE * scale);
                    let _ = self.render(w, h, scale, 1.0 - t);
                    if p >= 1.0 {
                        let _ = ShowWindow(self.hwnd, SW_HIDE);
                        let _ = KillTimer(Some(self.hwnd), TIMER_ID);
                        self.phase = Phase::Hidden;
                    }
                }
                Phase::Hidden => {
                    let _ = KillTimer(Some(self.hwnd), TIMER_ID);
                }
            }
        }
    }

    unsafe fn load_mark(ctx: &ID2D1DeviceContext, scale: f32) -> Result<ID2D1Bitmap1> {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let wic_stream = factory.CreateStream()?;
        // ID2D1SvgDocument respeta el width/height explícito del <svg> raíz (1024) en vez de
        // ajustar el viewBox al viewport de CreateSvgDocument, así que con ambos presentes no
        // cae nada dentro del viewport pequeño. Sin width/height, el viewBox escala al viewport.
        let svg_data = MARK_SVG
            .replace(" width=\"1024\"", "")
            .replace(" height=\"1024\"", "");
        wic_stream.InitializeFromMemory(svg_data.as_bytes())?;
        let stream: IStream = wic_stream.cast()?;

        let ctx5: ID2D1DeviceContext5 = ctx.cast()?;
        let size = 32.0 * scale;
        let px = size.round() as u32;
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
            ..Default::default()
        };
        let mark = ctx.CreateBitmap(D2D_SIZE_U { width: px, height: px }, None, 0, &props)?;
        let svg = ctx5.CreateSvgDocument(&stream, D2D_SIZE_F { width: size, height: size })?;

        ctx.SetTarget(&mark);
        ctx.BeginDraw();
        ctx.Clear(Some(&D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }));
        ctx5.DrawSvgDocument(&svg);
        ctx.EndDraw(None, None)?;
        ctx.SetTarget(None);
        Ok(mark)
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
                                        Cmd::Show(data) => rend.show(data),
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
                    if let Some(rend) = r.borrow_mut().as_mut() {
                        rend.tick();
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

        #[test]
        fn ease_out_hits_endpoints() {
            assert_eq!(ease_out(0.0), 0.0);
            assert_eq!(ease_out(1.0), 1.0);
        }

        #[test]
        fn keycap_layout_positions_chips_with_plus_separators() {
            // two keys of width 30 and 20, plus glyph width 10, gap 6, chip horizontal pad 8
            let (rects, total) = keycap_layout(&[30.0, 20.0], 10.0, 6.0, 8.0);
            assert_eq!(rects.len(), 2);
            // first chip starts at 0, width = 30 + 2*8 = 46
            assert!((rects[0].left - 0.0).abs() < 0.01);
            assert!((rects[0].right - 46.0).abs() < 0.01);
            // gap, plus(10), gap before second chip: 46 + 6 + 10 + 6 = 68
            assert!((rects[1].left - 68.0).abs() < 0.01);
            // second chip width = 20 + 16 = 36 -> right = 104
            assert!((rects[1].right - 104.0).abs() < 0.01);
            assert!((total - 104.0).abs() < 0.01);
        }
    }
}
