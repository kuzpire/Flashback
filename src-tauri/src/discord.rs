// Rich Presence de Discord (opt-in, off por defecto). Un hilo en segundo plano mantiene la
// conexión IPC con el cliente de Discord y refresca la presencia según el estado (grabando /
// Instant Replay / biblioteca) y el juego detectado. Aislado del camino de captura: solo lee
// estado cada pocos segundos y reconecta solo si Discord se cierra/abre.

use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use discord_rich_presence::activity::{Activity, Assets, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};

const APP_ID: &str = "1495797767245922498";
// Asset subido en la app de Discord (Rich Presence → Art Assets) con el logo de Flashback.
const FLASHBACK_ASSET: &str = "flashback";

struct Shared {
    enabled: bool,
}

fn state() -> &'static (Mutex<Shared>, Condvar) {
    static STATE: OnceLock<(Mutex<Shared>, Condvar)> = OnceLock::new();
    STATE.get_or_init(|| (Mutex::new(Shared { enabled: false }), Condvar::new()))
}

// Arranca el gestor una sola vez con el valor persistido. Idempotente.
pub fn init(enabled: bool) {
    set_enabled(enabled);
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_ok() {
        let _ = std::thread::Builder::new()
            .name("flashback-discord".into())
            .spawn(run);
    }
}

pub fn set_enabled(enabled: bool) {
    let (m, cv) = state();
    m.lock().unwrap().enabled = enabled;
    cv.notify_all();
}

fn is_enabled() -> bool {
    state().0.lock().unwrap().enabled
}

// Espera hasta `dur` o hasta que cambie el toggle (notify), para reaccionar rápido a on/off.
fn wait(dur: Duration) {
    let (m, cv) = state();
    if let Ok(g) = m.lock() {
        let _ = cv.wait_timeout(g, dur);
    }
}

fn run() {
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut client: Option<DiscordIpcClient> = None;
    let mut last_key = String::new();

    loop {
        if !is_enabled() {
            if let Some(mut c) = client.take() {
                let _ = c.close();
            }
            last_key.clear();
            let (m, cv) = state();
            if let Ok(g) = m.lock() {
                let _ = cv.wait_timeout_while(g, Duration::from_secs(60), |s| !s.enabled);
            }
            continue;
        }

        // Asegurar conexión: si Discord no está abierto, reintentar en 15 s.
        if client.is_none() {
            if let Ok(mut c) = DiscordIpcClient::new(APP_ID) {
                if c.connect().is_ok() {
                    client = Some(c);
                    last_key.clear();
                }
            }
            if client.is_none() {
                wait(Duration::from_secs(15));
                continue;
            }
        }

        let (details, state_line, large_image, large_text) = presence_fields();
        // Solo se reenvía a Discord si algo cambió (evita spam de set_activity).
        let key = format!("{details}\u{1}{state_line}\u{1}{large_image}\u{1}{large_text}");
        if key != last_key {
            if let Some(c) = client.as_mut() {
                let mut assets = Assets::new()
                    .small_image(FLASHBACK_ASSET)
                    .small_text("Flashback");
                if !large_image.is_empty() {
                    assets = assets.large_image(&large_image);
                }
                if !large_text.is_empty() {
                    assets = assets.large_text(&large_text);
                }
                let mut act = Activity::new()
                    .assets(assets)
                    .timestamps(Timestamps::new().start(started));
                if !details.is_empty() {
                    act = act.details(&details);
                }
                if !state_line.is_empty() {
                    act = act.state(&state_line);
                }
                if c.set_activity(act).is_err() {
                    // Conexión caída (Discord cerrado): reconectar en el próximo ciclo.
                    let _ = c.close();
                    client = None;
                    last_key.clear();
                    wait(Duration::from_secs(5));
                    continue;
                }
                last_key = key;
            }
        }

        wait(Duration::from_secs(5));
    }
}

// Detalle según captura; el juego detectado va como imagen grande (arte) + línea de estado.
fn presence_fields() -> (String, String, String, String) {
    let details = if crate::capture::status().running {
        "Grabando".to_string()
    } else if crate::capture::replay_active() {
        "Instant Replay activo".to_string()
    } else {
        "En la biblioteca".to_string()
    };

    match crate::detect::current_game() {
        Some(g) => {
            // Steam: el header público del CDN sirve de imagen grande. Sin AppID no hay URL
            // pública (el arte se cachea local como data-url, que Discord no admite) → logo.
            let large_image = g
                .steam_appid
                .map(|id| {
                    format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{id}/header.jpg")
                })
                .unwrap_or_else(|| FLASHBACK_ASSET.to_string());
            (details, g.name.clone(), large_image, g.name)
        }
        None => (
            details,
            String::new(),
            FLASHBACK_ASSET.to_string(),
            "Flashback".to_string(),
        ),
    }
}
