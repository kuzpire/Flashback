// Cartel "fuera de foco" que se compone cuando el juego está minimizado: el último frame
// difuminado y oscurecido, con el logo de la app y un texto. Se usa para no codificar frames
// congelados. Todo en GPU con Direct2D/DirectWrite sobre el mismo ID3D11Device del pipeline
// de captura (zero-copy): D2D escribe directamente sobre la textura BGRA de salida.

#[cfg(target_os = "windows")]
pub use win::OutOfFocusCard;

#[cfg(not(target_os = "windows"))]
pub struct OutOfFocusCard;

#[cfg(target_os = "windows")]
mod win {
    use windows::core::{w, Interface, Result};
    use windows::Win32::Graphics::Direct2D::Common::{
        D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_COMPOSITE_MODE_SOURCE_OVER, D2D1_PIXEL_FORMAT,
        D2D_RECT_F,
    };
    use windows::Win32::Graphics::Direct2D::Common::D2D1_BORDER_MODE_HARD;
    use windows::Win32::Graphics::Direct2D::{
        D2D1CreateDevice, ID2D1Bitmap1, ID2D1DeviceContext, ID2D1Effect, ID2D1SolidColorBrush,
        CLSID_D2D1GaussianBlur, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET,
        D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_DRAW_TEXT_OPTIONS_NONE,
        D2D1_GAUSSIANBLUR_OPTIMIZATION_SPEED, D2D1_GAUSSIANBLUR_PROP_BORDER_MODE,
        D2D1_GAUSSIANBLUR_PROP_OPTIMIZATION, D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION,
        D2D1_INTERPOLATION_MODE_LINEAR, D2D1_PROPERTY_TYPE_ENUM, D2D1_PROPERTY_TYPE_FLOAT,
    };
    use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D};
    use windows::Win32::Graphics::DirectWrite::{
        DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
        DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
        DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
    };
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
    use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGISurface};
    use windows::Win32::Graphics::Imaging::{
        CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory,
        WICBitmapDitherTypeNone, WICBitmapPaletteTypeMedianCut, WICDecodeMetadataCacheOnLoad,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

    const LOGO_PNG: &[u8] = include_bytes!("../icons/128x128@2x.png");

    pub struct OutOfFocusCard {
        ctx: ID2D1DeviceContext,
        logo: ID2D1Bitmap1,
        text_format: IDWriteTextFormat,
        // Texto del cartel ya en UTF-16 (lo elige el llamador según el idioma).
        text: Vec<u16>,
        white: ID2D1SolidColorBrush,
        dim: ID2D1SolidColorBrush,
        blur: ID2D1Effect,
        width: u32,
        height: u32,
    }
    unsafe impl Send for OutOfFocusCard {}

    impl OutOfFocusCard {
        pub fn new(device: &ID3D11Device, width: u32, height: u32, text: &str) -> Result<Self> {
            let dxgi: IDXGIDevice = device.cast()?;
            let d2d_device = unsafe { D2D1CreateDevice(&dxgi, None)? };
            let ctx = unsafe { d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)? };

            let logo = load_logo(&ctx)?;

            let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
            let font_size = (height as f32 * 0.040).max(16.0);
            let text_format = unsafe {
                dwrite.CreateTextFormat(
                    w!("Segoe UI"),
                    None,
                    DWRITE_FONT_WEIGHT_SEMI_BOLD,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    font_size,
                    w!("en-us"),
                )?
            };
            unsafe {
                text_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
                text_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            }

            let white = unsafe {
                ctx.CreateSolidColorBrush(
                    &D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.92 },
                    None,
                )?
            };
            let dim = unsafe {
                ctx.CreateSolidColorBrush(
                    &D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.45 },
                    None,
                )?
            };

            let blur = unsafe { ctx.CreateEffect(&CLSID_D2D1GaussianBlur)? };
            let deviation = (height as f32 * 0.014).max(8.0);
            unsafe {
                blur.SetValue(
                    D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32,
                    D2D1_PROPERTY_TYPE_FLOAT,
                    &deviation.to_le_bytes(),
                )?;
                // SPEED: el blur se calcula una sola vez al minimizar, pero a 1440p/4K esto
                // recorta su coste sin pérdida perceptible. HARD evita que el borde se aclare.
                blur.SetValue(
                    D2D1_GAUSSIANBLUR_PROP_OPTIMIZATION.0 as u32,
                    D2D1_PROPERTY_TYPE_ENUM,
                    &(D2D1_GAUSSIANBLUR_OPTIMIZATION_SPEED.0 as u32).to_le_bytes(),
                )?;
                blur.SetValue(
                    D2D1_GAUSSIANBLUR_PROP_BORDER_MODE.0 as u32,
                    D2D1_PROPERTY_TYPE_ENUM,
                    &(D2D1_BORDER_MODE_HARD.0 as u32).to_le_bytes(),
                )?;
            }

            Ok(Self {
                ctx,
                logo,
                text_format,
                text: text.encode_utf16().collect(),
                white,
                dim,
                blur,
                width,
                height,
            })
        }

        // Compone el cartel a partir de `src` (último frame del juego) y lo escribe en `dst`.
        // Ambas son texturas BGRA del mismo tamaño que la captura.
        pub fn render(&self, src: &ID3D11Texture2D, dst: &ID3D11Texture2D) -> Result<()> {
            let src_surface: IDXGISurface = src.cast()?;
            let dst_surface: IDXGISurface = dst.cast()?;

            let src_props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_IGNORE,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                ..Default::default()
            };
            let src_bmp =
                unsafe { self.ctx.CreateBitmapFromDxgiSurface(&src_surface, Some(&src_props))? };

            let dst_props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_IGNORE,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                ..Default::default()
            };
            let target =
                unsafe { self.ctx.CreateBitmapFromDxgiSurface(&dst_surface, Some(&dst_props))? };

            let w = self.width as f32;
            let h = self.height as f32;
            let cx = w / 2.0;
            let cy = h / 2.0;

            let logo_box = unsafe { self.logo.GetSize() };
            let logo_h = (h * 0.16).max(48.0);
            let logo_w = logo_h * (logo_box.width / logo_box.height.max(1.0));
            let logo_rect = D2D_RECT_F {
                left: cx - logo_w / 2.0,
                top: cy - logo_h - h * 0.02,
                right: cx + logo_w / 2.0,
                bottom: cy - h * 0.02,
            };
            let text_rect = D2D_RECT_F {
                left: 0.0,
                top: cy + h * 0.02,
                right: w,
                bottom: cy + h * 0.02 + h * 0.12,
            };
            let full = D2D_RECT_F {
                left: 0.0,
                top: 0.0,
                right: w,
                bottom: h,
            };
            unsafe {
                self.ctx.SetTarget(&target);
                self.blur.SetInput(0, &src_bmp, true);
                self.ctx.BeginDraw();
                let out = self.blur.GetOutput()?;
                self.ctx.DrawImage(
                    &out,
                    None,
                    None,
                    D2D1_INTERPOLATION_MODE_LINEAR,
                    D2D1_COMPOSITE_MODE_SOURCE_OVER,
                );
                self.ctx.FillRectangle(&full, &self.dim);
                self.ctx.DrawBitmap(
                    &self.logo,
                    Some(&logo_rect),
                    1.0,
                    D2D1_INTERPOLATION_MODE_LINEAR,
                    None,
                    None,
                );
                self.ctx.DrawText(
                    &self.text,
                    &self.text_format,
                    &text_rect,
                    &self.white,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
                self.ctx.EndDraw(None, None)?;
                self.ctx.SetTarget(None);
            }
            Ok(())
        }
    }

    fn load_logo(ctx: &ID2D1DeviceContext) -> Result<ID2D1Bitmap1> {
        let factory: IWICImagingFactory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };
        let stream = unsafe { factory.CreateStream()? };
        let mut data = LOGO_PNG.to_vec();
        unsafe { stream.InitializeFromMemory(&mut data)? };
        let decoder = unsafe {
            factory.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnLoad)?
        };
        let frame = unsafe { decoder.GetFrame(0)? };
        let converter = unsafe { factory.CreateFormatConverter()? };
        unsafe {
            converter.Initialize(
                &frame,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeMedianCut,
            )?;
        }
        let bitmap = unsafe { ctx.CreateBitmapFromWicBitmap(&converter, None)? };
        Ok(bitmap)
    }
}
