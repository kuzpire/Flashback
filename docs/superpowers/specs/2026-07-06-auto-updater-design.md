# Auto-updater con popup y bolita de notificación — Design

Fecha: 2026-07-06
Estado: aprobado (pendiente de plan de implementación)

## Objetivo

Añadir actualización automática a Flashback usando el plugin oficial de Tauri 2, con una
UX mínima y no intrusiva:

- Al arrancar, tras un breve retardo, se comprueba si hay una versión nueva.
- Si la hay y la ventana está visible, aparece un **modal centrado** para actualizar.
- Si la ventana no está visible (arranque en bandeja), no se interrumpe: solo aparece una
  **bolita de notificación** sobre el icono de la app; el modal se muestra al abrir la ventana.
- El modal se cierra con click fuera o "Cancelar" (la bolita persiste). "Actualizar" descarga,
  instala y relanza.
- Mientras haya una actualización pendiente, la bolita se mantiene; al clicar el icono de la
  app se reabre el modal.

Encaja con la filosofía del proyecto: nativo, ligero, opt-in, sin nube obligatoria, sin
impacto en el camino de captura.

## Enfoque y alternativas

**Elegido:** plugin oficial `tauri-plugin-updater` (+ `tauri-plugin-process` para relanzar).
Nativo, verifica firma criptográfica, sin dependencias externas pesadas.

Descartadas:
- **Velopack / Squirrel:** dependencia externa grande, contra la filosofía de ligereza.
- **Updater casero:** reimplementar descarga + verificación de firma; riesgo de seguridad
  innecesario.

## Infraestructura de firma y publicación

- Par de claves generado con `pnpm tauri signer generate`.
  - **Password:** aleatoria de 32 caracteres.
  - **Clave privada:** `src-tauri/.tauri/flashback.key` (gitignored).
  - **Password:** `src-tauri/.tauri/flashback.key.pass` (gitignored).
  - **Clave pública:** incrustada en `tauri.conf.json` (`plugins.updater.pubkey`).
- `.gitignore`: añadir `/src-tauri/.tauri/`.
- `tauri.conf.json`:
  - `bundle.createUpdaterArtifacts: true` → genera `-setup.exe` + su `.sig`.
  - `plugins.updater.endpoints: ["https://github.com/kuzpire/Flashback/releases/latest/download/latest.json"]`.
- **`latest.json`** (formato del updater de Tauri):
  ```json
  {
    "version": "1.4.0",
    "notes": "…",
    "pub_date": "2026-…T…Z",
    "platforms": {
      "windows-x86_64": {
        "signature": "<contenido de Flashback_x.y.z_x64-setup.exe.sig>",
        "url": "https://github.com/kuzpire/Flashback/releases/download/vX.Y.Z/Flashback_X.Y.Z_x64-setup.exe"
      }
    }
  }
  ```
- **`scripts/release.sh`** (un solo comando reemplaza el flujo manual):
  1. Lee la versión de `package.json`.
  2. Exporta `TAURI_SIGNING_PRIVATE_KEY` (contenido de la key) y
     `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (contenido del `.pass`).
  3. `pnpm tauri build` (genera `.exe`, `.msi`, `.sig`).
  4. Genera `latest.json` leyendo el `.sig` y componiendo la URL de la release.
  5. `gh release create vX.Y.Z` subiendo `.exe`, `.msi` y `latest.json`, con título y notas
     coherentes con las releases anteriores.
  - El endpoint apunta a `/releases/latest/download/latest.json`, que GitHub resuelve a la
    última release no-prerelease.

## Backend (Rust)

- `Cargo.toml`: `tauri-plugin-updater = "2"`, `tauri-plugin-process = "2"`.
- `lib.rs`: registrar `.plugin(tauri_plugin_updater::Builder::new().build())` y
  `.plugin(tauri_plugin_process::init())`.
- `capabilities/default.json`: añadir `updater:default` y `process:allow-restart`.
- La comprobación e instalación las orquesta el frontend con la API JS del plugin; no hace
  falta comando Rust propio.

## Frontend — estado y lógica

Nuevo módulo `src/lib/updater.svelte.ts`:

- Estado (`$state`): `available: boolean`, `info: { version, notes } | null`,
  `popupOpen: boolean`, `installing: boolean`, `progress: number`.
- `checkForUpdate()`: llama a `check()` del plugin. Si devuelve update:
  `available = true`, `info = { version, notes }`. Si la ventana está visible en ese momento,
  `popupOpen = true`.
- `openPopup()` / `closePopup()`: controlan `popupOpen`. Cerrar NO baja `available`.
- `installUpdate()`: detiene replay/grabación activa → `downloadAndInstall()` (actualiza
  `progress`) → `relaunch()`.
- Fuera de Tauri (preview navegador) el `check()` falla silenciosamente (igual que el resto
  de invokes del proyecto).

Disparo en `+layout.svelte` (al montar): `setTimeout(~4000ms)` → `checkForUpdate()`.
La comprobación de "ventana visible" usa `getCurrentWindow().isVisible()`.

## Frontend — UI

1. **Bolita de notificación.** El `.logo` del sidebar se convierte en `<button>` cuando
   `available` es true; punto de color de acento (`--accent`) anclado arriba-derecha del
   icono (30×30). Click → `openPopup()`. Cuando no hay update, el logo se mantiene como está
   (no interactivo).
2. **Modal centrado.** Overlay oscuro a pantalla completa + tarjeta centrada, con el mismo
   lenguaje visual que los menús actuales (`--bg-1`, `--line-strong`, sombras existentes).
   Contenido: título "Actualización disponible", versión nueva, notas breves, y botones
   **"Cancelar"** / **"Actualizar"**. Click en el overlay o "Cancelar" → `closePopup()`.
   "Actualizar" → `installUpdate()` con barra de progreso (deshabilita los botones mientras
   `installing`).

## Seguridad en captura

- La descarga/instalación es JS/IPC async; no toca el hilo de captura/codificación.
- Antes de instalar se detiene el replay/grabación en curso (el instalador cierra la app),
  evitando dejar el pipeline en estado inconsistente.

## i18n

Añadir claves a los diccionarios `en`/`es` de `src/lib/i18n.svelte.ts`:
título del modal, etiqueta de versión, botones "Actualizar"/"Cancelar", texto de progreso
("Descargando…", "Instalando…"), y `aria-label` de la bolita.

## Fuera de alcance

- Actualizaciones diferenciales/delta.
- Canal beta/prerelease.
- Comprobación periódica en segundo plano (solo al arrancar / al reabrir la ventana con
  update pendiente).
- Plataformas distintas de Windows x64.
