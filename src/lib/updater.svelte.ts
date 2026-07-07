import { check, type Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';

type UpdateInfo = { version: string; notes: string };

export const updater = $state<{
  available: boolean;
  info: UpdateInfo | null;
  popupOpen: boolean;
  installing: boolean;
  progress: number;
}>({ available: false, info: null, popupOpen: false, installing: false, progress: 0 });

let pending: Update | null = null;
// Solo se auto-muestra el modal la primera vez que la ventana está visible; después
// del "Cancelar" solo se reabre desde la bolita del logo.
let autoShown = false;

export async function checkForUpdate() {
  try {
    const u = await check();
    if (!u) return;
    pending = u;
    updater.info = { version: u.version, notes: u.body ?? '' };
    updater.available = true;
    await maybeAutoShow();
  } catch (e) {
    // Fuera de Tauri (preview en navegador) o sin red: se ignora en silencio.
    console.error('update check', e);
  }
}

export async function maybeAutoShow() {
  if (!updater.available || autoShown || updater.popupOpen) return;
  try {
    if (await getCurrentWindow().isVisible()) {
      autoShown = true;
      updater.popupOpen = true;
    }
  } catch {
    // preview navegador
  }
}

export function openUpdatePopup() {
  if (updater.available) updater.popupOpen = true;
}

export function closeUpdatePopup() {
  if (updater.installing) return;
  updater.popupOpen = false;
}

export async function installUpdate() {
  if (!pending || updater.installing) return;
  updater.installing = true;
  updater.progress = 0;
  // El instalador cierra la app: se detiene la captura para no dejar el pipeline a medias.
  await invoke('stop_replay').catch(() => {});
  await invoke('stop_capture').catch(() => {});
  try {
    let total = 0;
    let got = 0;
    await pending.downloadAndInstall((e) => {
      if (e.event === 'Started') total = e.data.contentLength ?? 0;
      else if (e.event === 'Progress') {
        got += e.data.chunkLength;
        updater.progress = total ? Math.round((got / total) * 100) : 0;
      } else if (e.event === 'Finished') updater.progress = 100;
    });
    await relaunch();
  } catch (e) {
    console.error('install update', e);
    updater.installing = false;
  }
}
