// Marca de agua opcional. Se hornea SOLO en la exportación (nunca en captura ni replay: el camino
// de captura es sagrado y la edición es no destructiva). El isotipo mono (SVG) se rasteriza una
// vez a un bitmap BGRA premultiplicado del tamaño objetivo, con la opacidad ya aplicada, y se
// mezcla (alpha-over) sobre cada frame RGB32 antes de reencodar.

#[cfg(target_os = "windows")]
pub use win::Logo;

#[derive(Clone, Copy)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    pub fn parse(s: &str) -> Corner {
        match s {
            "tl" => Corner::TopLeft,
            "tr" => Corner::TopRight,
            "bl" => Corner::BottomLeft,
            _ => Corner::BottomRight,
        }
    }
}

// Alto del logo respecto al alto del vídeo, opacidad y margen a los bordes. Defaults fijos: la UI
// solo expone on/off y la esquina (mejores defaults > más opciones).
const HEIGHT_FRAC: f32 = 0.055;
const OPACITY: f32 = 0.70;
const MARGIN_FRAC: f32 = 0.03;

// Esquina superior-izquierda del logo dentro del frame, con margen. Puro y testeable.
fn place(corner: Corner, fw: u32, fh: u32, lw: u32, lh: u32, margin: i64) -> (i64, i64) {
    let (fw, fh, lw, lh) = (fw as i64, fh as i64, lw as i64, lh as i64);
    match corner {
        Corner::TopLeft => (margin, margin),
        Corner::TopRight => (fw - lw - margin, margin),
        Corner::BottomLeft => (margin, fh - lh - margin),
        Corner::BottomRight => (fw - lw - margin, fh - lh - margin),
    }
}

// Alpha-over con fuente premultiplicada: out = src + dst*(255-a)/255. `inv` = 255 - alpha_fuente.
fn over_u8(src_premul: u8, dst: u8, inv: u32) -> u8 {
    (src_premul as u32 + (dst as u32 * inv) / 255) as u8
}

#[cfg(target_os = "windows")]
mod win {
    use super::{over_u8, place, Corner, HEIGHT_FRAC, MARGIN_FRAC, OPACITY};
    use windows::core::{Interface, Result};
    use windows::Win32::Foundation::{E_FAIL, HMODULE};
    use windows::Win32::Graphics::Direct2D::Common::{
        D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
    };
    use windows::Win32::Graphics::Direct2D::{
        D2D1CreateDevice, ID2D1Bitmap1, ID2D1DeviceContext5,
        D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_CPU_READ, D2D1_BITMAP_OPTIONS_TARGET,
        D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_MAP_OPTIONS_READ,
        D2D1_MAPPED_RECT,
    };
    use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
    };
    use windows::Win32::Graphics::Direct2D::Common::{D2D_SIZE_F, D2D_SIZE_U};
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
    use windows::Win32::Graphics::Dxgi::IDXGIDevice;
    use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, IWICImagingFactory};
    use windows::Win32::System::Com::{CoCreateInstance, IStream, CLSCTX_INPROC_SERVER};

    const MARK_SVG: &str = include_str!("../../static/flashback-mono.svg");

    // Logo rasterizado en BGRA premultiplicado, con opacidad ya horneada, listo para el blend.
    pub struct Logo {
        width: u32,
        height: u32,
        bgra: Vec<u8>,
        corner: Corner,
    }

    fn create_d3d_device() -> Result<ID3D11Device> {
        for dt in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
            let mut device: Option<ID3D11Device> = None;
            let ok = unsafe {
                D3D11CreateDevice(
                    None,
                    dt,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    None,
                )
            };
            if ok.is_ok() {
                if let Some(d) = device {
                    return Ok(d);
                }
            }
        }
        Err(windows::core::Error::from(E_FAIL))
    }

    impl Logo {
        // Rasteriza el SVG mono a un cuadrado de lado `out_h*HEIGHT_FRAC` (el isotipo es 1:1) con la
        // opacidad aplicada. Coste: una vez por exportación. Errores: los propaga el llamador para
        // exportar sin marca (best-effort), sin romper el export.
        pub fn rasterize(out_w: u32, out_h: u32, corner: Corner) -> Result<Logo> {
            let side = ((out_h as f32 * HEIGHT_FRAC).round() as u32).clamp(8, out_h.max(8));
            let _ = out_w;

            let device = create_d3d_device()?;
            let dxgi: IDXGIDevice = device.cast()?;
            let d2d_device = unsafe { D2D1CreateDevice(&dxgi, None)? };
            let ctx = unsafe { d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)? };
            let ctx5: ID2D1DeviceContext5 = ctx.cast()?;

            let factory: IWICImagingFactory =
                unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };
            let wic_stream = unsafe { factory.CreateStream()? };
            // ID2D1SvgDocument respeta el width/height explícito del <svg> raíz (1024): con ambos
            // presentes nada cae dentro del viewport pequeño. Sin ellos, el viewBox escala al
            // viewport (misma corrección que en toast.rs).
            let svg_data = MARK_SVG
                .replace(" width=\"1024\"", "")
                .replace(" height=\"1024\"", "");
            unsafe { wic_stream.InitializeFromMemory(svg_data.as_bytes())? };
            let stream: IStream = wic_stream.cast()?;

            let fmt = D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            };
            let target_props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: fmt,
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
                ..Default::default()
            };
            let size_u = D2D_SIZE_U { width: side, height: side };
            let target = unsafe { ctx.CreateBitmap(size_u, None, 0, &target_props)? };
            let svg = unsafe {
                ctx5.CreateSvgDocument(&stream, D2D_SIZE_F { width: side as f32, height: side as f32 })?
            };

            unsafe {
                ctx.SetTarget(&target);
                ctx.BeginDraw();
                ctx.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
                ctx5.DrawSvgDocument(&svg);
                ctx.EndDraw(None, None)?;
                ctx.SetTarget(None);
            }

            // Bitmap accesible por CPU para leer los píxeles rasterizados.
            let cpu_props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: fmt,
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                ..Default::default()
            };
            let cpu: ID2D1Bitmap1 = unsafe { ctx.CreateBitmap(size_u, None, 0, &cpu_props)? };
            unsafe { cpu.CopyFromBitmap(None, &target, None)? };

            let mut bgra = vec![0u8; (side * side * 4) as usize];
            unsafe {
                let map: D2D1_MAPPED_RECT = cpu.Map(D2D1_MAP_OPTIONS_READ)?;
                let row_bytes = (side * 4) as usize;
                for y in 0..side as usize {
                    let src = map.bits.add(y * map.pitch as usize);
                    let dst = &mut bgra[y * row_bytes..y * row_bytes + row_bytes];
                    std::ptr::copy_nonoverlapping(src, dst.as_mut_ptr(), row_bytes);
                }
                let _ = cpu.Unmap();
            }

            // Aplicar opacidad: escalar los cuatro canales premultiplicados mantiene el buffer válido.
            if OPACITY < 1.0 {
                let k = (OPACITY * 255.0).round() as u32;
                for b in bgra.iter_mut() {
                    *b = ((*b as u32 * k) / 255) as u8;
                }
            }

            Ok(Logo { width: side, height: side, bgra, corner })
        }

        // Mezcla el logo sobre un frame BGRA/RGB32. `scan0` apunta a la fila superior de la imagen y
        // `pitch` es el stride en bytes con signo (IMF2DBuffer::Lock2D: negativo si el buffer es
        // bottom-up). Recorta a los bordes del frame.
        pub unsafe fn blend(&self, scan0: *mut u8, pitch: isize, fw: u32, fh: u32) {
            let margin = (fh as f32 * MARGIN_FRAC).round() as i64;
            let (ox, oy) = place(self.corner, fw, fh, self.width, self.height, margin);
            for ly in 0..self.height as i64 {
                let fy = oy + ly;
                if fy < 0 || fy >= fh as i64 {
                    continue;
                }
                let frow = scan0.offset(fy as isize * pitch);
                let lrow = (ly as usize) * self.width as usize * 4;
                for lx in 0..self.width as i64 {
                    let fx = ox + lx;
                    if fx < 0 || fx >= fw as i64 {
                        continue;
                    }
                    let s = lrow + (lx as usize) * 4;
                    let sa = self.bgra[s + 3];
                    if sa == 0 {
                        continue;
                    }
                    let inv = 255 - sa as u32;
                    let d = frow.offset(fx as isize * 4);
                    *d = over_u8(self.bgra[s], *d, inv);
                    *d.add(1) = over_u8(self.bgra[s + 1], *d.add(1), inv);
                    *d.add(2) = over_u8(self.bgra[s + 2], *d.add(2), inv);
                    *d.add(3) = 255;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corner_parse_defaults_to_br() {
        assert!(matches!(Corner::parse("zzz"), Corner::BottomRight));
        assert!(matches!(Corner::parse("tl"), Corner::TopLeft));
        assert!(matches!(Corner::parse("tr"), Corner::TopRight));
        assert!(matches!(Corner::parse("bl"), Corner::BottomLeft));
    }

    #[test]
    fn place_corners_with_margin() {
        // Frame 1000x500, logo 100x80, margen 10.
        assert_eq!(place(Corner::TopLeft, 1000, 500, 100, 80, 10), (10, 10));
        assert_eq!(place(Corner::TopRight, 1000, 500, 100, 80, 10), (890, 10));
        assert_eq!(place(Corner::BottomLeft, 1000, 500, 100, 80, 10), (10, 410));
        assert_eq!(place(Corner::BottomRight, 1000, 500, 100, 80, 10), (890, 410));
    }

    #[test]
    fn over_u8_extremes() {
        // Fuente opaca (a=255 => inv=0): gana el src premultiplicado.
        assert_eq!(over_u8(200, 50, 0), 200);
        // Fuente transparente (a=0 => inv=255): queda el destino.
        assert_eq!(over_u8(0, 137, 255), 137);
        // Media: src=100 (premul), dst=200, a=128 => inv=127 => 100 + 200*127/255 = 199.
        assert_eq!(over_u8(100, 200, 127), 199);
    }
}
