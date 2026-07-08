# Native Toast Overlay Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the WebView2 toast (`overlay` window + `static/overlay.html`) with a native Win32 layered window rendered with Direct2D/DirectWrite/DirectComposition, redesigned as a two-line right-edge tab (brand + message/keycaps) inspired by SteelSeries Moments.

**Architecture:** A dedicated Windows-only Rust module (`toast.rs`) owns a click-through, non-activating, topmost `WS_POPUP` window on its own thread with a message pump. It renders with a private D3D11 device + Direct2D device context, composited via DirectComposition (per-pixel alpha, GPU). The Tauri `toast` command sends structured data (`title`, `body`, `keys`, `kind`) over a channel to that thread. The capture/encoder pipeline is never touched.

**Tech Stack:** Rust, `windows` crate 0.61 (Direct2D, DirectWrite, DirectComposition, D3D11, DXGI), Tauri 2, Svelte 5 (frontend payload + i18n).

## Global Constraints

- **Windows-only:** all new Rust code lives behind `#[cfg(target_os = "windows")]`, mirroring `overlay.rs`.
- **Language:** chat/answers in Spanish; commit messages in English; UI copy via i18n (ES + EN).
- **Commit authorship:** single author only. NEVER add `Co-Authored-By: Claude` or "Generated with Claude Code" to commits or PRs.
- **No decorative comments:** comment only genuinely non-obvious *why* (performance, counter-intuitive decisions).
- **Capture path is sacred:** the toast uses its OWN D3D11 device, never the capture/encoder device; nothing here may block or share resources with capture threads.
- **No new heavyweight deps:** use native Windows APIs only; do not add crates.
- **`windows` crate version stays `0.61`** (see `src-tauri/Cargo.toml`).

## Testing note

Native window creation and GPU drawing have no cheap headless assertion. For those tasks the gate is: `cargo build --manifest-path src-tauri/Cargo.toml` succeeds AND a stated manual observation in `pnpm tauri dev`. Pure logic (kind parsing, layout math from measured sizes) gets real unit tests via `cargo test`.

## File Structure

- `src-tauri/Cargo.toml` — add missing `windows` features (`Win32_Graphics_DirectComposition`, `Win32_UI_HiDpi`).
- `src-tauri/src/toast.rs` — **new.** The entire native toast: window, rendering, animation thread, public `Toast` handle. One responsibility: show/hide the toast.
- `src-tauri/src/lib.rs` — hold a `Toast` handle in Tauri state; rewrite `toast`/`dismiss_toast` commands; remove the `overlay` webview setup; add `mod toast`.
- `src/routes/+layout.svelte` — `toast()` helper sends the new payload; the "replay ready" call passes real keys.
- `src/lib/i18n.svelte.ts` — new hint key; adjust ready-toast copy.
- `src-tauri/tauri.conf.json` — remove the `overlay` window.
- `src-tauri/capabilities/overlay.json` — delete.
- `static/overlay.html` — delete.

---

### Task 1: Native window scaffold + end-to-end wiring (solid dark tab)

Stand up the module, the window, the DirectComposition surface, and the thread, and wire the real Tauri command to it so a solid dark rounded tab appears top-right when any toast fires. No text/logo yet.

**Files:**
- Modify: `src-tauri/Cargo.toml` (dependency features)
- Create: `src-tauri/src/toast.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod toast`; state; rewrite `toast`/`dismiss_toast`; frontend still compiles)
- Modify: `src/routes/+layout.svelte:143-146` (send new payload shape)

**Interfaces:**
- Produces (`toast.rs`):
  - `pub struct ToastData { pub title: String, pub body: String, pub keys: Vec<String>, pub kind: ToastKind }`
  - `pub enum ToastKind { Info, Ready, Saved, Error }` with `pub fn from_str(s: &str) -> ToastKind`
  - `pub struct Toast` (a `Send + Sync` handle) with `pub fn spawn() -> Toast`, `pub fn show(&self, data: ToastData)`, `pub fn hide(&self)`
- Consumes: none (first task).

- [ ] **Step 1: Add windows-crate features**

In `src-tauri/Cargo.toml`, inside the `[target.'cfg(windows)'.dependencies] windows = { features = [ … ] }` list, add these two entries (keep the rest):

```toml
    "Win32_Graphics_DirectComposition",
    "Win32_UI_HiDpi",
```

- [ ] **Step 2: Write the failing unit test for `ToastKind::from_str`**

Create `src-tauri/src/toast.rs` with only:

```rust
#[cfg(target_os = "windows")]
pub use win::{Toast, ToastData, ToastKind};

#[cfg(target_os = "windows")]
mod win {
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
```

- [ ] **Step 3: Run the test to verify it fails to compile/pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml kind_parsing`
Expected: fails first because `toast` module is not declared in `lib.rs` (add `#[cfg(target_os = "windows")] mod toast;` near the other `mod` lines at `src-tauri/src/lib.rs:1-13`), then passes once declared. (Declaring the module is required for the test to build.)

- [ ] **Step 4: Add `mod toast;` and re-run the test**

In `src-tauri/src/lib.rs`, after line 12 (`mod overlay;`) add:

```rust
#[cfg(target_os = "windows")]
mod toast;
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml kind_parsing`
Expected: PASS.

- [ ] **Step 5: Implement `ToastData`, the window, DComp surface, thread, and `Toast` handle**

Append to the `mod win` block in `src-tauri/src/toast.rs`. This creates a `WS_POPUP` window with extended styles `WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP`, a private D3D11 device (`D3D11_CREATE_DEVICE_BGRA_SUPPORT`, no video flag), a Direct2D device context, and a DirectComposition device/target/visual/surface. A background thread owns everything and runs a message pump; commands arrive via `std::sync::mpsc` and are delivered into the pump with `PostMessageW(hwnd, WM_APP_SHOW/WM_APP_HIDE, ...)` so drawing happens on the thread that owns the HWND.

Key implementation requirements (write concrete code following `overlay.rs` idioms for `unsafe`/`windows` usage):

- `pub struct ToastData { pub title: String, pub body: String, pub keys: Vec<String>, pub kind: ToastKind }` — derive `Clone`.
- `pub struct Toast { tx: std::sync::mpsc::Sender<Cmd>, hwnd: isize }` where `enum Cmd { Show(ToastData), Hide }`. Mark `unsafe impl Send for Toast {}` / `Sync` as needed (the `isize` HWND is just used for `PostMessageW`).
- `Toast::spawn()`: spawn a thread; inside, register the window class (`RegisterClassW` with a `wndproc`), create the window hidden, build D3D11+D2D+DComp resources into a thread-local/`RefCell`-owned `Renderer`, stash `Cmd` receiver, then run `GetMessageW` loop. Use a `OnceLock`/channel to hand the created `HWND` back to the constructor before entering the loop. The `wndproc` handles `WM_APP_SHOW` (index into a slot holding the latest `ToastData`), `WM_APP_HIDE`, `WM_TIMER` (animation — added in Task 5; for now a fixed `SetTimer` that hides after 2800 ms), and `WM_DESTROY`.
- `Toast::show(&self, data)`: push `Cmd::Show(data)` into a shared slot (`Mutex<Option<ToastData>>`) and `PostMessageW(hwnd, WM_APP_SHOW, 0, 0)`.
- `Toast::hide(&self)`: `PostMessageW(hwnd, WM_APP_HIDE, 0, 0)`.
- Window sizing/position for now: fixed logical `360 x 76` scaled by the primary monitor DPI (`GetDpiForMonitor` on `MonitorFromWindow`/primary); position `x = work_right - width`, `y = work_top + round(12 * scale)` using `MonitorFromPoint`/`GetMonitorInfoW` on the primary monitor. Extract this into `fn place(hwnd, w, h)`.
- Rendering (`Renderer::render(&self, data: &ToastData)`): `surface.BeginDraw` → get `IDXGISurface` + offset `POINT`; create a D2D bitmap from it; `ctx.SetTarget`; `ctx.SetTransform` to translate by the offset; `BeginDraw`; `Clear(transparent)`; fill a rounded rectangle (`ID2D1RoundedRectangleGeometry` or `FillRoundedRectangle`) covering the whole surface but with **only the left corners rounded** (radius `8 * scale`): draw as a `PathGeometry` = left side rounded, right side square, or simplest: fill a `D2D1_ROUNDED_RECT` slightly wider than the window so the right corners are clipped off the right edge. Fill color `#141416` at alpha `0.98`. `EndDraw`; `SetTarget(None)`; `surface.EndDraw()`; `dcomp_device.Commit()`.
- On show: `place(...)`, render, `ShowWindow(hwnd, SW_SHOWNOACTIVATE)`, `SetWindowPos` reaffirming `HWND_TOPMOST` with `SWP_NOACTIVATE`. On hide: `ShowWindow(hwnd, SW_HIDE)`.

- [ ] **Step 6: Rewrite the Tauri commands to use the native Toast**

In `src-tauri/src/lib.rs`, replace the `ToastPayload` struct (`:330-333`) and the `toast`/`dismiss_toast` commands (`:340-376`) with:

```rust
#[derive(serde::Deserialize)]
struct ToastPayload {
    title: String,
    body: String,
    #[serde(default)]
    keys: Vec<String>,
    kind: String,
}

#[cfg(target_os = "windows")]
#[tauri::command]
fn toast(toast: tauri::State<'_, toast::Toast>, payload: ToastPayload) {
    toast.show(toast::ToastData {
        title: payload.title,
        body: payload.body,
        keys: payload.keys,
        kind: toast::ToastKind::from_str(&payload.kind),
    });
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
fn toast(_payload: ToastPayload) {}

#[cfg(target_os = "windows")]
#[tauri::command]
fn dismiss_toast(toast: tauri::State<'_, toast::Toast>) {
    toast.hide();
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
fn dismiss_toast() {}
```

Note: Tauri passes command args by the JS key. Because the JS now calls `invoke('toast', { payload: {...} })` (Step 8), the Rust arg is named `payload`. Keep `toast`/`dismiss_toast` in the existing `generate_handler!` list (`:509-510`).

- [ ] **Step 7: Register the `Toast` in Tauri state and drop the overlay webview setup**

In `src-tauri/src/lib.rs` `run()` setup (`:403+`), after the window setup, create and manage the toast, and remove the `overlay` webview block at `:425-429`:

```rust
#[cfg(target_os = "windows")]
app.manage(toast::Toast::spawn());
```

Delete the `if let Some(o) = app.get_webview_window("overlay") { … }` block (`:425-429`). Leave the `overlay` window in `tauri.conf.json` for now (removed in Task 8); it will just sit unused.

- [ ] **Step 8: Update the frontend `toast()` helper to send the new payload**

In `src/routes/+layout.svelte`, replace the helper (`:143-146`) with:

```ts
type ToastKind = 'info' | 'ready' | 'saved' | 'error';
function toast(text: string, kind: ToastKind = 'info', keys: string[] = []) {
  invoke('toast', { payload: { title: 'Flashback', body: text, keys, kind } }).catch(() => {});
}
```

Leave all existing call sites unchanged (they pass `text, kind`; `keys` defaults to `[]`). The `dismiss_toast` invoke inside `overlay.html` is removed in Task 8; nothing else calls it.

- [ ] **Step 9: Build and manually verify**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: compiles.

Run: `pnpm tauri dev`, then trigger a toast (e.g. press the record hotkey with no target selected → "Selecciona una pantalla…", or enable background replay → ready). 
Expected observation: a dark rounded tab appears at the **top-right**, flush to the right edge, only left corners rounded, no border; it does **not** steal focus from the foreground; mouse clicks pass through it; it disappears after ~2.8 s.

- [ ] **Step 10: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/toast.rs src-tauri/src/lib.rs src/routes/+layout.svelte
git commit -m "Add native toast window scaffold and wire toast command"
```

---

### Task 2: Two-line text (title + body) with DirectWrite

Draw "Flashback" (title) and the `body` string as a second line, left-aligned to the right of where the logo will go.

**Files:**
- Modify: `src-tauri/src/toast.rs`

**Interfaces:**
- Consumes: `Renderer` from Task 1, `ToastData { title, body, .. }`.
- Produces: DWrite text formats stored on `Renderer` (`title_format: IDWriteTextFormat`, `body_format: IDWriteTextFormat`), and a `measure(data) -> (u32, u32)` helper returning content pixel size (used to size the window instead of the fixed 360×76).

- [ ] **Step 1: Create DWrite factory + two text formats in `Renderer::new`**

Following `overlay.rs:65-81`: create `IDWriteFactory` via `DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)`. Title format: `Segoe UI`, `DWRITE_FONT_WEIGHT_SEMI_BOLD`, size `15.0`. Body format: `Segoe UI`, `DWRITE_FONT_WEIGHT_NORMAL`, size `13.0`. Both `DWRITE_TEXT_ALIGNMENT_LEADING`, `DWRITE_PARAGRAPH_ALIGNMENT_CENTER`. Store both on `Renderer`. Create two brushes: `text_bright` (`#f0f2f7` a=1.0) and `text_dim` (`#f0f2f7` a=0.72).

- [ ] **Step 2: Measure content and size the window from it**

Add `fn measure(&self, data: &ToastData) -> (f32, f32)`: create an `IDWriteTextLayout` for `title` and one for `body` (max width large, e.g. 1000). Content width = `logo_col (44px) + padding + max(title_width, body_line_width) + right_padding (24px)`. Height fixed at `76px` logical (two lines). Return device pixels (multiply by `scale`). In the show path, call `measure` and use it in `place`/window resize (`SetWindowPos` with `SWP_NOACTIVATE | SWP_NOZORDER`), then recreate/resize the DComp surface to match (`dcomp_device.CreateSurface(w, h, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_ALPHA_MODE_PREMULTIPLIED)` and `visual.SetContent`). Keep `keys` width out of `body_line_width` for now (added in Task 4).

- [ ] **Step 3: Draw the two lines in `render`**

After filling the background rect (Task 1), draw title at `(text_left, top_padding)` and body at `(text_left, title_baseline_gap)` using `ctx.DrawText` with the respective format/brush. `text_left = 44 * scale` (reserve the logo column). Title uses `text_bright`; body uses `text_dim`. Use `DWRITE_MEASURING_MODE_NATURAL`.

- [ ] **Step 4: Build and manually verify**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: compiles.

Run: `pnpm tauri dev`; trigger a toast.
Expected: the tab now shows **Flashback** on top and the message below, left side reserving empty space for the logo; the tab width fits the longest line.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/toast.rs
git commit -m "Render toast title and body with DirectWrite"
```

---

### Task 3: Logo mark (Direct2D SVG)

Render the existing `flashback-mono.svg` as the left, vertically-centered mark.

**Files:**
- Modify: `src-tauri/src/toast.rs`

**Interfaces:**
- Consumes: `Renderer` device context (must be castable to `ID2D1DeviceContext5`).
- Produces: `mark: ID2D1Bitmap1` cached on `Renderer` (the SVG rendered once to an offscreen bitmap).

- [ ] **Step 1: Embed the SVG and render it once to a cached bitmap**

Add `const MARK_SVG: &str = include_str!("../../static/flashback-mono.svg");`. In `Renderer::new`, after the D2D context exists:
- Get an `IStream` over the SVG bytes: reuse the WIC stream trick from `overlay.rs:225-230` (`IWICImagingFactory::CreateStream` + `InitializeFromMemory`) and `.cast::<IStream>()`.
- `let ctx5: ID2D1DeviceContext5 = ctx.cast()?;`
- Create an offscreen target bitmap `mark` of size `32x32 * scale` with `D2D1_BITMAP_OPTIONS_TARGET`, `DXGI_FORMAT_B8G8R8A8_UNORM`, `D2D1_ALPHA_MODE_PREMULTIPLIED`.
- `let svg = ctx5.CreateSvgDocument(&stream, D2D_SIZE_F { width: 32.0*scale, height: 32.0*scale })?;`
- `ctx.SetTarget(&mark); ctx.BeginDraw(); ctx.Clear(transparent); ctx5.DrawSvgDocument(&svg); ctx.EndDraw(None, None)?; ctx.SetTarget(None);`
- Store `mark` on `Renderer`.

The SVG fill is white (`#ffffff`); no recolor needed. (Error accent uses the title color, not the mark — Task 6.)

- [ ] **Step 2: Draw the cached mark in `render`, vertically centered**

In `render`, `DrawBitmap` the `mark` into a rect: `left = 12*scale`, size `28*scale`, `top = (height - 28*scale)/2`. Interpolation `D2D1_INTERPOLATION_MODE_LINEAR`.

- [ ] **Step 3: Build and manually verify**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: compiles.

Run: `pnpm tauri dev`; trigger a toast.
Expected: the white Flashback mark appears at the left, vertically centered, crisp at the current display scale.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/toast.rs
git commit -m "Render Flashback mark from SVG in toast"
```

---

### Task 4: Keycap chips

When `keys` is non-empty, draw each key in a rounded chip before the `body` text on the second line.

**Files:**
- Modify: `src-tauri/src/toast.rs`

**Interfaces:**
- Consumes: `ToastData.keys: Vec<String>`, `Renderer` formats/brushes.
- Produces: `chip_format: IDWriteTextFormat` (Segoe UI Semibold 11.5), `chip_bg`/`chip_text` brushes; a pure `fn keycap_layout(key_widths: &[f32], plus_width: f32, gap: f32, pad_x: f32) -> (Vec<Rect>, f32)` returning per-chip rects (x-relative to the keys origin) and total keys-block width. This is unit-testable.

- [ ] **Step 1: Write the failing unit test for `keycap_layout`**

Add to the `#[cfg(test)] mod tests`:

```rust
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
```

Use `windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F` as `Rect` (alias `type Rect = D2D_RECT_F;`).

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml keycap_layout`
Expected: FAIL — `keycap_layout` not found.

- [ ] **Step 3: Implement `keycap_layout`**

```rust
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
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --manifest-path src-tauri/Cargo.toml keycap_layout`
Expected: PASS.

- [ ] **Step 5: Add chip format/brushes and include keys in `measure`**

Create `chip_format` (Segoe UI Semibold 11.5, centered both axes), `chip_bg` brush (`#2a2a30` a=1.0), `chip_text` brush (`#f0f2f7` a=0.85). In `measure`, when `keys` is non-empty, compute each key's text width with a transient `IDWriteTextLayout` (chip_format), run `keycap_layout` (pad_x `7*scale`, gap `4*scale`, plus_width = measured "+" width in body_format), and set the second-line width = `keys_block_width + gap + body_width`.

- [ ] **Step 6: Draw chips in `render`**

On the second line, if `keys` non-empty: for each chip rect (offset to the line origin, height `18*scale` centered on the line), fill a `D2D1_ROUNDED_RECT` (radius `4*scale`) with `chip_bg`, draw the key text centered with `chip_text`, and draw a "+" (body_format, `text_dim`) in each gap. Then draw `body` after the keys block. If `keys` empty, draw `body` at the line origin (Task 2 behavior).

- [ ] **Step 7: Build and manually verify (temporary keys)**

Temporarily change one call site, e.g. `+layout.svelte:432`, to `toast(t('toast.replayReady'), 'ready', ['ALT','F8'])`.
Run: `cargo build --manifest-path src-tauri/Cargo.toml` then `pnpm tauri dev`; enable background replay from off.
Expected: second line shows `[ALT] + [F8]` as rounded chips, then the text. Revert the temporary change before committing (real wiring is Task 7).

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/toast.rs
git commit -m "Draw keycap chips on toast second line"
```

---

### Task 5: Slide + fade animation and auto-hide

Animate entrance (slide from right + fade in) and exit (fade out + slide), and auto-hide after the visible window, replacing any in-flight toast.

**Files:**
- Modify: `src-tauri/src/toast.rs`

**Interfaces:**
- Consumes: `Renderer`, the `visual: IDCompositionVisual` and `dcomp_device`.
- Produces: animation state on the renderer/thread (`phase`, `start_instant`), driven by `WM_TIMER`.

- [ ] **Step 1: Add animation state and a frame timer**

On `WM_APP_SHOW`: set the latest `ToastData`, resize/place, render once, `ShowWindow(SW_SHOWNOACTIVATE)`, set `phase = In`, record `Instant::now()`, and `SetTimer(hwnd, ANIM_TIMER, 16, None)`. Store the auto-hide deadline (`shown_at + 2800ms`). A new `WM_APP_SHOW` while visible just resets `data`, `phase = In`, and `start` (replace behavior).

- [ ] **Step 2: Drive the animation on `WM_TIMER`**

On each `ANIM_TIMER` tick compute progress:
- Phase `In` (0→220 ms): `t = ease_out(progress)`, `offset_x = (1 - t) * 24 * scale`, `opacity = t`. At end → `phase = Visible`.
- Phase `Visible`: when `now >= deadline` → `phase = Out`, reset `start`.
- Phase `Out` (0→300 ms): `offset_x = t * 24 * scale`, `opacity = 1 - t`. At end → `ShowWindow(SW_HIDE)`, `KillTimer`, `phase = Hidden`.

Apply each tick: `visual.SetOffsetX(offset_x)`; apply opacity by rendering the surface content pre-multiplied by `opacity` (multiply every brush/bitmap draw alpha by `opacity`, or draw into a layer with `PushLayer` opacity). Simplest reliable route: pass `opacity` into `render` and multiply brush alphas + `DrawBitmap` opacity by it. `dcomp_device.Commit()` after `SetOffsetX`. Use `ease_out(p) = 1 - (1 - p).powi(3)` (add a pure `fn ease_out(p: f32) -> f32` with a small unit test asserting `ease_out(0.0)==0.0`, `ease_out(1.0)==1.0`).

- [ ] **Step 3: `WM_APP_HIDE` starts the Out phase**

On `WM_APP_HIDE`: if visible, set `phase = Out`, reset `start` so it fades out (rather than hiding instantly).

- [ ] **Step 4: Build and manually verify**

Run: `cargo build --manifest-path src-tauri/Cargo.toml` then `pnpm tauri dev`; trigger toasts.
Expected: the tab slides in from the right while fading in, holds ~2.8 s, then slides out while fading; firing a new toast during one restarts the entrance cleanly.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/toast.rs
git commit -m "Animate toast slide and fade with auto-hide"
```

---

### Task 6: Error accent

Distinguish `ToastKind::Error` with a subtle red accent (red title text) — no border.

**Files:**
- Modify: `src-tauri/src/toast.rs`

**Interfaces:**
- Consumes: `ToastData.kind`, `Renderer` brushes.
- Produces: `title_error` brush.

- [ ] **Step 1: Add the error title brush and use it by kind**

Create `title_error` brush = `#ff5b5b` a=1.0 (the existing `--rec` color from `overlay.html:10`). In `render`, choose the title brush: `if data.kind == ToastKind::Error { &self.title_error } else { &self.text_bright }`. Body and mark stay unchanged.

- [ ] **Step 2: Build and manually verify**

Run: `cargo build --manifest-path src-tauri/Cargo.toml` then `pnpm tauri dev`; trigger an error toast (e.g. stop a recording that errors, or force `toast.startFailed`).
Expected: the **Flashback** title renders red; no border appears; other kinds stay light.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/toast.rs
git commit -m "Add red title accent for error toasts"
```

---

### Task 7: Frontend — real keys on the ready toast + i18n

Make the "replay ready" toast show the actual configured save-replay hotkey as keycaps, with a translated hint after it.

**Files:**
- Modify: `src/lib/i18n.svelte.ts` (both `en` and `es` maps)
- Modify: `src/routes/+layout.svelte:432` (pass keys) and imports

**Interfaces:**
- Consumes: `labelTokens` from `src/lib/hotkeys.svelte.ts:86` (already exported), `hotkeys.saveReplay`.
- Produces: new i18n key `toast.replayReadyHint`.

- [ ] **Step 1: Add the hint i18n key**

In `src/lib/i18n.svelte.ts`, add to the English map (near `:159`):

```ts
  'toast.replayReadyHint': 'to clip',
```

and to the Spanish map (near `:380`):

```ts
  'toast.replayReadyHint': 'para hacer un clip',
```

- [ ] **Step 2: Import `labelTokens` and pass keys on the ready toast**

In `src/routes/+layout.svelte`, extend the existing hotkeys import (`:11`) to include `labelTokens`:

```ts
import { hotkeys, capture, labelFor, labelTokens } from '$lib/hotkeys.svelte';
```

Replace the ready-toast call (`:432`) with:

```ts
if (wasOff) toast(t('toast.replayReadyHint'), 'ready', labelTokens(hotkeys.saveReplay));
```

- [ ] **Step 3: Typecheck and manually verify**

Run: `pnpm check` (svelte-check). Expected: no new errors.

Run: `pnpm tauri dev`; enable background replay from off.
Expected: the toast reads **Flashback** / `[ALT] + [F8]` (or the user's configured key) followed by "para hacer un clip". Change the save-replay hotkey in Settings, re-arm replay, and confirm the chips update to the new combo.

- [ ] **Step 4: Commit**

```bash
git add src/lib/i18n.svelte.ts src/routes/+layout.svelte
git commit -m "Show configured save-replay hotkey as keycaps in ready toast"
```

---

### Task 8: Remove the WebView2 overlay

Delete the now-dead WebView2 overlay window and its assets/permissions.

**Files:**
- Delete: `static/overlay.html`
- Delete: `src-tauri/capabilities/overlay.json`
- Modify: `src-tauri/tauri.conf.json:25-38` (remove the `overlay` window object)

**Interfaces:**
- Consumes: nothing (cleanup). Confirms no remaining references to the `overlay` webview label.

- [ ] **Step 1: Remove the overlay window from Tauri config**

In `src-tauri/tauri.conf.json`, delete the entire second window object (`{ "label": "overlay", … }`, lines `25-38`), leaving only the main window in the `windows` array. Ensure the JSON stays valid (remove the trailing comma after the first window object).

- [ ] **Step 2: Delete the overlay assets**

```bash
git rm static/overlay.html src-tauri/capabilities/overlay.json
```

- [ ] **Step 3: Verify no dangling references**

Run: `rg -n "overlay" src-tauri/src src src-tauri/tauri.conf.json`
Expected: only `src-tauri/src/overlay.rs` (the unrelated out-of-focus card module) and its `mod overlay;`/usages remain. No references to the `overlay` **webview label**, `overlay.html`, `emit_to("overlay", …)`, `get_webview_window("overlay")`, `show-toast`, or `dismiss_toast` from JS.

- [ ] **Step 4: Build and manually verify full flow**

Run: `cargo build --manifest-path src-tauri/Cargo.toml` then `pnpm tauri dev`.
Expected: app launches with no `overlay` webview; all toast types (select-screen, recording, clip saved, ready with keycaps, error in red) render via the native window; RAM shows one fewer WebView2 instance.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/tauri.conf.json
git commit -m "Remove WebView2 toast overlay window and assets"
```

---

## Self-Review

- **Spec coverage:** native window (T1) · DComp/D2D/DWrite (T1–T2) · own D3D11 device separate from capture (T1) · dedicated thread + pump (T1) · right-edge tab, no border, subtle radius, dark fill (T1) · two lines title+body (T2) · logo left centered (T3) · keycaps (T4) · slide+fade + replace + auto-hide (T5) · error accent without border (T6) · payload `{title, body, keys, kind}` (T1 Rust / T1+T7 JS) · keys from configured hotkey + i18n (T7) · remove `overlay.html`, tauri.conf window, capability, setup code, dismiss_toast JS (T1 setup + T8). All spec sections map to tasks.
- **DPI/primary monitor** handled in T1 `place`/`measure`.
- **Placeholders:** none — each drawing task states exact rects, colors, sizes; pure functions have full code + tests.
- **Type consistency:** `Toast`/`ToastData`/`ToastKind` names and the `{title, body, keys, kind}` payload are consistent across T1 (Rust + JS) and reused in T4/T6/T7; `keycap_layout`/`ease_out` are the only pure helpers and are defined where first used.
- **Known reality:** native window + GPU drawing are gated by `cargo build` + manual observation (documented up top); genuine unit tests cover `ToastKind::from_str`, `keycap_layout`, `ease_out`.
