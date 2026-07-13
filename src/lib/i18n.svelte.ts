import { invoke } from '@tauri-apps/api/core';

export type Locale = 'en' | 'es';

export const LOCALES: { value: Locale; label: string }[] = [
  { value: 'en', label: 'English' },
  { value: 'es', label: 'Español' }
];

let locale = $state<Locale>('en');

export function getLocale(): Locale {
  return locale;
}

// Etiqueta BCP-47 para APIs nativas (Intl / toLocaleDateString). Lee `locale` (rune) para
// que los formateos de fecha se recalculen al cambiar de idioma.
export function localeTag(): string {
  return locale === 'es' ? 'es-ES' : 'en-US';
}

export function initLocale() {
  invoke<string>('get_language')
    .then((l) => {
      if (l === 'en' || l === 'es') locale = l;
    })
    .catch(() => {});
}

export function setLocale(l: Locale) {
  locale = l;
  invoke('set_language', { lang: l }).catch(() => {});
}

// t(key) es reactivo: al leer `locale` (rune) dentro de un template, el componente se
// re-renderiza al cambiar de idioma. Interpola {param} con el objeto opcional.
export function t(key: string, params?: Record<string, string | number>): string {
  let s = messages[locale][key] ?? messages.en[key] ?? key;
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      s = s.replaceAll(`{${k}}`, String(v));
    }
  }
  return s;
}

const en: Record<string, string> = {
  // Navegación / ventana
  'nav.clips': 'Clips',
  'nav.library': 'Library',
  'nav.games': 'Games',
  'nav.favorites': 'Favorites',
  'nav.settings': 'Settings',

  'win.minimize': 'Minimize',
  'win.restore': 'Restore',
  'win.maximize': 'Maximize',
  'win.close': 'Close',

  // Ajustes
  'settings.title': 'Settings',
  'settings.language': 'Language',
  'settings.language.desc': 'Interface language.',
  'settings.section.language': 'Language',
  'settings.section.capture': 'Capture',
  'settings.section.encoding': 'Encoding',
  'settings.section.sound': 'Sound',
  'settings.section.storage': 'Storage',
  'settings.section.integrations': 'Integrations',
  'settings.section.shortcuts': 'Shortcuts',
  'settings.resolution': 'Resolution',
  'settings.resolution.desc': 'Output height. Scaled from the native capture, never upscaled.',
  'settings.fps': 'Frames per second',
  'settings.fps.desc': 'Frames recorded per second.',
  'settings.quality': 'Quality',
  'settings.quality.desc': 'Higher quality produces larger files.',
  'settings.replayBg': 'Background replay',
  'settings.replayBg.desc': 'Keep a buffer ready to save.',
  'settings.bufferLen': 'Buffer length',
  'settings.bufferLen.desc': 'How many seconds/minutes are saved when you press the shortcut.',
  'settings.encoder': 'Encoder',
  'settings.encoder.desc': 'Auto picks the best available hardware encoder. Software uses the CPU.',
  'settings.soundVolume': 'Sound volume',
  'settings.saveSound': 'Sound on save',
  'settings.saveSound.desc': 'A sound plays when a replay is saved.',
  'settings.testSound': 'Test sound',
  'settings.clipsFolder': 'Clips folder',
  'settings.clipsFolder.desc':
    'Changing it only affects new clips; existing ones are still shown.',
  'settings.change': 'Change',
  'settings.autoDelete': 'Auto-delete',
  'settings.autoDelete.desc': 'Deletes clips not marked as favorites when it fills up.',
  'settings.discordRpc': 'Discord Rich Presence',
  'settings.discordRpc.desc':
    'Shows on your Discord profile that you use Flashback, with the detected game.',
  'settings.shortcuts.hint':
    'Tap a shortcut, press the new combination and save it with <strong>✓</strong>. <strong>ESC</strong> cancels.',
  'settings.hk.saveReplay': 'Save replay',
  'settings.hk.record': 'Record / stop',
  'settings.hk.open': 'Open Flashback',
  'settings.hk.changeAria': 'Change shortcut for {label}',
  'settings.hk.saveAria': 'Save shortcut',
  'settings.hk.badKey': 'Unsupported key',
  'settings.hk.pressKey': 'Press a key…',

  // Calidad de captura
  'quality.low': 'Low',
  'quality.normal': 'Medium',
  'quality.high': 'High',
  'quality.veryhigh': 'Very high',
  'quality.ultra': 'Ultra',

  // Nivel de sonido
  'sound.low': 'Low',
  'sound.normal': 'Normal',
  'sound.high': 'High',

  // Barra de captura
  'cap.inEditor': 'In the editor',
  'cap.recordingScreen': 'Recording screen',
  'cap.screenReady': 'Screen ready',
  'cap.captureDisabled': 'Capture disabled',
  'cap.capturingClips': 'Capturing clips',
  'cap.idle': 'Idle',
  'cap.screen': 'Screen',
  'cap.noGame': 'No game',
  'cap.application': 'Application',
  'cap.micCapture': 'Capture microphone audio',
  'cap.whatOption': 'What this option does',
  'cap.micTip':
    'If enabled, it also records your voice from the chosen microphone, mixed with the clip audio.',
  'cap.micInput': 'Microphone input',
  'cap.noMics': 'No microphones detected',
  'cap.noMicsShort': 'No microphones',
  'cap.screens': 'Screens',
  'cap.estSize': 'Estimated clip size:',
  'cap.aboutEstSize': 'About the estimated size',
  'cap.estSizeTip':
    'The size is approximate: the bitrate is variable and the real size depends on how busy the scene is.',
  'cap.startRec': 'Start recording',
  'cap.stopRec': 'Stop recording',
  'cap.editRecHotkey': 'Edit recording shortcut',
  'cap.duration': 'Duration',
  'cap.quality': 'Quality',
  'cap.resolution': 'Resolution',
  'cap.fps': 'FPS',

  // Toasts / atajos
  'toast.selectScreen': 'Select a screen to record, or open a game for Application mode.',
  'toast.recording': 'Recording',
  'toast.startFailed': 'Could not start recording: {e}',
  'toast.clipSaved': 'Clip saved',
  'toast.recStopped': 'Recording stopped',
  'toast.stopFailed': 'Error stopping the recording: {e}',
  'toast.enableReplay': 'Enable "Background replay" in Settings to save.',
  'toast.noTargetReplay': 'No target: select a screen or open a game for the replay to record.',
  'toast.replaySaveFailed': 'Could not save the replay (the buffer has no keyframe yet).',
  'toast.replaySaveError': 'Error saving the replay: {e}',
  'toast.replayReady': 'Ready to clip',
  'toast.replayReadyHint': 'to save a clip',
  'toast.replayStartFailed': 'Could not start the replay: {e}',
  'toast.hotkeyInUse': 'Shortcut in use by another app: {failed}. Change it in Settings.',
  'hk.name.saveClip': 'save clip',
  'hk.name.recording': 'recording',
  'hk.name.openFlashback': 'open Flashback',

  // Biblioteca de clips
  'clips.title': 'All clips',
  'clips.search': 'Search clips',
  'clips.newest': 'Newest',
  'clips.oldest': 'Oldest',
  'clips.emptyNone': "You don't have any clips yet.",
  'clips.emptyNoneHint': 'Record with the button or your shortcut and they will show up here.',
  'clips.noResultsQuery': 'No results for “{query}”.',
  'clips.noResultsFilter': 'No results with this filter.',

  // Favoritos
  'favs.title': 'Your Favorites',
  'favs.empty': "You don't have any clips saved as favorites yet.",
  'favs.emptyHint': "Tap a clip's star to save it here.",

  // Filtro de biblioteca
  'filter.label': 'Filter',
  'filter.edited': 'Edited',
  'filter.count': '{n} filters',
  'filter.clear': 'Clear filter',
  'filter.all': 'All',
  'filter.games': 'Games',
  'filter.screens': 'Screens',

  // Tarjeta de clip
  'card.favOn': 'Marked as favorite, click to unmark',
  'card.favOff': 'Mark as favorite',
  'card.share': 'Share',
  'card.more': 'More options',
  'card.openEditor': 'Open in editor',
  'card.rename': 'Rename',
  'card.openLocation': 'Open location',
  'card.delete': 'Delete',

  // Juegos
  'games.title': 'Detected games',
  'games.now': 'Right now',
  'games.recent': 'Recent games',
  'games.capturingClips': 'Capturing clips',
  'games.captureDisabled': 'Capture disabled',
  'games.captureActive': 'Capture on',
  'games.capture': 'Capture',
  'games.captureAria': 'Capture {name}',
  'games.emptyTitle': "You haven't opened any game with Flashback running yet.",
  'games.emptyHint': 'Games show up here the first time we detect them.',

  // Editor
  'ed.prevClip': 'Previous clip',
  'ed.nextClip': 'Next clip',
  'ed.created': 'Created:',
  'ed.size': 'Size:',
  'ed.path': 'Path:',
  'ed.reset': 'Reset',
  'ed.closeEditor': 'Close editor',
  'ed.preparingAudio': 'Preparing audio tracks…',
  'ed.trackSplitError': 'Could not separate the tracks: {error}',
  'ed.dragResize': 'Drag to resize',
  'ed.captureFrame': 'Capture frame',
  'ed.goStart': 'Go to start',
  'ed.prevFrame': 'Previous frame',
  'ed.pause': 'Pause',
  'ed.play': 'Play',
  'ed.nextFrame': 'Next frame',
  'ed.goEnd': 'Go to end',
  'ed.fullscreen': 'Fullscreen (F)',
  'ed.remove': 'Remove',
  'ed.exporting': 'Exporting…',
  'ed.export': 'Export',
  'ed.watermark': 'Watermark',
  'ed.watermarkOn': 'Watermark on export',
  'ed.watermarkPos': 'Watermark position',
  'ed.micAudio': 'Microphone audio',
  'ed.sysAudio': 'System audio',
  'ed.audio': 'Audio',
  'ed.mute': 'Mute {label}',
  'ed.unmute': 'Unmute {label}',
  'ed.noAudio': 'No audio',
  'ed.enable': 'Enable',
  'ed.disable': 'Disable',
  'ed.deleteBlock': 'Delete',
  'ed.exportingClip': 'Exporting clip…',
  'ed.shotSaved': 'Screenshot saved',
  'ed.shotError': 'Error capturing: {e}',
  'ed.shotOpen': 'Click here to open',
  'ed.exported': 'Exported: {name}',
  'ed.exportError': 'Error exporting: {e}',

  // Tiempo relativo
  'time.now': 'Just now',
  'time.moment': 'A moment ago',
  'time.today': 'Today',
  'time.yesterday': 'Yesterday',
  'time.minAgo': '{n} min ago',
  'time.hourAgo': '1 hour ago',
  'time.hoursAgo': '{n} hours ago',
  'time.hAgo': '{n} h ago',
  'time.daysAgo': '{n} days ago',
  'time.weekAgo': '1 week ago',
  'time.weeksAgo': '{n} weeks ago',
  'time.monthAgo': '1 month ago',
  'time.monthsAgo': '{n} months ago',
  'time.yearAgo': '1 year ago',
  'time.yearsAgo': '{n} years ago',
  'upd.title': 'Update available',
  'upd.version': 'Version {v} is ready to install.',
  'upd.update': 'Update',
  'upd.cancel': 'Cancel',
  'upd.installing': 'Downloading and installing…',
  'upd.badgeLabel': 'Update available'
};

const es: Record<string, string> = {
  'nav.clips': 'Clips',
  'nav.library': 'Biblioteca',
  'nav.games': 'Juegos',
  'nav.favorites': 'Favoritos',
  'nav.settings': 'Ajustes',

  'win.minimize': 'Minimizar',
  'win.restore': 'Restaurar',
  'win.maximize': 'Maximizar',
  'win.close': 'Cerrar',

  'settings.title': 'Ajustes',
  'settings.language': 'Idioma',
  'settings.language.desc': 'Idioma de la interfaz.',
  'settings.section.language': 'Idioma',
  'settings.section.capture': 'Captura',
  'settings.section.encoding': 'Codificación',
  'settings.section.sound': 'Sonido',
  'settings.section.storage': 'Almacenamiento',
  'settings.section.integrations': 'Integraciones',
  'settings.section.shortcuts': 'Atajos',
  'settings.resolution': 'Resolución',
  'settings.resolution.desc': 'Alto de salida. Se escala desde la captura nativa, sin superarla.',
  'settings.fps': 'Fotogramas por segundo',
  'settings.fps.desc': 'Cantidad de fotogramas grabados por segundo.',
  'settings.quality': 'Calidad',
  'settings.quality.desc': 'Más calidad produce archivos más pesados.',
  'settings.replayBg': 'Replay en segundo plano',
  'settings.replayBg.desc': 'Mantén un buffer listo para guardar.',
  'settings.bufferLen': 'Duración del buffer',
  'settings.bufferLen.desc': 'Cuántos segundos/minutos se guardan al pulsar el atajo.',
  'settings.encoder': 'Encoder',
  'settings.encoder.desc': 'Auto elige el mejor encoder por hardware disponible. Software usa CPU.',
  'settings.soundVolume': 'Volumen del sonido',
  'settings.saveSound': 'Sonido al guardar',
  'settings.saveSound.desc': 'Se reproduce un aviso al guardar un replay.',
  'settings.testSound': 'Probar sonido',
  'settings.clipsFolder': 'Carpeta de clips',
  'settings.clipsFolder.desc':
    'Cambiarla solo afecta a los clips nuevos; los anteriores se siguen mostrando.',
  'settings.change': 'Cambiar',
  'settings.autoDelete': 'Borrado automático',
  'settings.autoDelete.desc': 'Elimina los clips no marcados como favoritos al llenarse.',
  'settings.discordRpc': 'Discord Rich Presence',
  'settings.discordRpc.desc':
    'Muestra en tu perfil de Discord que usas Flashback, con el juego detectado.',
  'settings.shortcuts.hint':
    'Toca un atajo, pulsa la nueva combinación y guárdala con <strong>✓</strong>. <strong>ESC</strong> cancela.',
  'settings.hk.saveReplay': 'Guardar replay',
  'settings.hk.record': 'Grabar / detener',
  'settings.hk.open': 'Abrir Flashback',
  'settings.hk.changeAria': 'Cambiar atajo de {label}',
  'settings.hk.saveAria': 'Guardar atajo',
  'settings.hk.badKey': 'Tecla no compatible',
  'settings.hk.pressKey': 'Pulsa una tecla…',

  'quality.low': 'Bajo',
  'quality.normal': 'Medio',
  'quality.high': 'Alto',
  'quality.veryhigh': 'Muy alta',
  'quality.ultra': 'Ultra',

  'sound.low': 'Bajo',
  'sound.normal': 'Normal',
  'sound.high': 'Alto',

  'cap.inEditor': 'En el editor',
  'cap.recordingScreen': 'Grabando pantalla',
  'cap.screenReady': 'Pantalla lista',
  'cap.captureDisabled': 'Captura deshabilitada',
  'cap.capturingClips': 'Capturando clips',
  'cap.idle': 'En espera',
  'cap.screen': 'Pantalla',
  'cap.noGame': 'Sin juego',
  'cap.application': 'Aplicación',
  'cap.micCapture': 'Capturar audio del micrófono',
  'cap.whatOption': 'Qué hace esta opción',
  'cap.micTip':
    'Si está activo, graba también tu voz desde el micrófono elegido, mezclada con el audio del clip.',
  'cap.micInput': 'Entrada de micrófono',
  'cap.noMics': 'Sin micrófonos detectados',
  'cap.noMicsShort': 'Sin micrófonos',
  'cap.screens': 'Pantallas',
  'cap.estSize': 'Tamaño estimado del clip:',
  'cap.aboutEstSize': 'Sobre el tamaño estimado',
  'cap.estSizeTip':
    'El tamaño es aproximado: el bitrate es variable y el peso real depende de lo movida que sea la escena.',
  'cap.startRec': 'Iniciar grabación',
  'cap.stopRec': 'Detener grabación',
  'cap.editRecHotkey': 'Editar atajo de grabación',
  'cap.duration': 'Duración',
  'cap.quality': 'Calidad',
  'cap.resolution': 'Resolución',
  'cap.fps': 'FPS',

  'toast.selectScreen': 'Selecciona una pantalla para grabar, o abre un juego para el modo Aplicación.',
  'toast.recording': 'Grabando',
  'toast.startFailed': 'No se pudo iniciar la grabación: {e}',
  'toast.clipSaved': 'Clip guardado',
  'toast.recStopped': 'Grabación detenida',
  'toast.stopFailed': 'Error al detener la grabación: {e}',
  'toast.enableReplay': 'Activa "Replay en segundo plano" en Ajustes para guardar.',
  'toast.noTargetReplay': 'Sin objetivo: selecciona una pantalla o abre un juego para que el replay grabe.',
  'toast.replaySaveFailed': 'No se pudo guardar el replay (el buffer aún no tiene un keyframe).',
  'toast.replaySaveError': 'Error al guardar el replay: {e}',
  'toast.replayReady': 'Listo para clipear',
  'toast.replayReadyHint': 'para guardar un clip',
  'toast.replayStartFailed': 'No se pudo iniciar el replay: {e}',
  'toast.hotkeyInUse': 'Atajo en uso por otra app: {failed}. Cámbialo en Ajustes.',
  'hk.name.saveClip': 'guardar clip',
  'hk.name.recording': 'grabación',
  'hk.name.openFlashback': 'abrir Flashback',

  'clips.title': 'Todos los clips',
  'clips.search': 'Buscar clips',
  'clips.newest': 'Más reciente',
  'clips.oldest': 'Más antiguo',
  'clips.emptyNone': 'Aún no tienes clips.',
  'clips.emptyNoneHint': 'Graba con el botón o tu atajo y aparecerán aquí.',
  'clips.noResultsQuery': 'Sin resultados para “{query}”.',
  'clips.noResultsFilter': 'Sin resultados con este filtro.',

  'favs.title': 'Tus Favoritos',
  'favs.empty': 'Aún no tienes clips guardados como favoritos.',
  'favs.emptyHint': 'Marca la estrella de un clip para guardarlo aquí.',

  'filter.label': 'Filtro',
  'filter.edited': 'Editados',
  'filter.count': '{n} filtros',
  'filter.clear': 'Quitar filtro',
  'filter.all': 'Todos',
  'filter.games': 'Juegos',
  'filter.screens': 'Pantallas',

  'card.favOn': 'Marcado como favorito, click para desmarcar',
  'card.favOff': 'Marcar como favorito',
  'card.share': 'Compartir',
  'card.more': 'Más opciones',
  'card.openEditor': 'Abrir en editor',
  'card.rename': 'Renombrar',
  'card.openLocation': 'Abrir ubicación',
  'card.delete': 'Borrar',

  'games.title': 'Juegos detectados',
  'games.now': 'Ahora mismo',
  'games.recent': 'Juegos Recientes',
  'games.capturingClips': 'Capturando clips',
  'games.captureDisabled': 'Captura deshabilitada',
  'games.captureActive': 'Captura activa',
  'games.capture': 'Capturar',
  'games.captureAria': 'Capturar {name}',
  'games.emptyTitle': 'Aún no has abierto ningún juego con Flashback activo.',
  'games.emptyHint': 'Los juegos aparecen aquí la primera vez que los detectamos.',

  'ed.prevClip': 'Clip anterior',
  'ed.nextClip': 'Clip siguiente',
  'ed.created': 'Creado:',
  'ed.size': 'Tamaño:',
  'ed.path': 'Ruta:',
  'ed.reset': 'Reestablecer',
  'ed.closeEditor': 'Cerrar editor',
  'ed.preparingAudio': 'Preparando pistas de audio…',
  'ed.trackSplitError': 'No se pudieron separar las pistas: {error}',
  'ed.dragResize': 'Arrastrar para redimensionar',
  'ed.captureFrame': 'Capturar fotograma',
  'ed.goStart': 'Ir al inicio',
  'ed.prevFrame': 'Fotograma anterior',
  'ed.pause': 'Pausar',
  'ed.play': 'Reproducir',
  'ed.nextFrame': 'Fotograma siguiente',
  'ed.goEnd': 'Ir al final',
  'ed.fullscreen': 'Pantalla completa (F)',
  'ed.remove': 'Quitar',
  'ed.exporting': 'Exportando…',
  'ed.export': 'Exportar',
  'ed.watermark': 'Marca de agua',
  'ed.watermarkOn': 'Marca de agua al exportar',
  'ed.watermarkPos': 'Posición de la marca',
  'ed.micAudio': 'Audio del micrófono',
  'ed.sysAudio': 'Audio del sistema',
  'ed.audio': 'Audio',
  'ed.mute': 'Silenciar {label}',
  'ed.unmute': 'Activar {label}',
  'ed.noAudio': 'Sin audio',
  'ed.enable': 'Activar',
  'ed.disable': 'Desactivar',
  'ed.deleteBlock': 'Eliminar',
  'ed.exportingClip': 'Exportando clip…',
  'ed.shotSaved': 'Captura guardada',
  'ed.shotError': 'Error al capturar: {e}',
  'ed.shotOpen': 'Clic aquí para abrir',
  'ed.exported': 'Exportado: {name}',
  'ed.exportError': 'Error al exportar: {e}',

  'time.now': 'Ahora mismo',
  'time.moment': 'Hace un momento',
  'time.today': 'Hoy',
  'time.yesterday': 'Ayer',
  'time.minAgo': 'Hace {n} min',
  'time.hourAgo': 'Hace una hora',
  'time.hoursAgo': 'Hace {n} horas',
  'time.hAgo': 'Hace {n} h',
  'time.daysAgo': 'Hace {n} días',
  'time.weekAgo': 'Hace 1 semana',
  'time.weeksAgo': 'Hace {n} semanas',
  'time.monthAgo': 'Hace 1 mes',
  'time.monthsAgo': 'Hace {n} meses',
  'time.yearAgo': 'Hace 1 año',
  'time.yearsAgo': 'Hace {n} años',
  'upd.title': 'Actualización disponible',
  'upd.version': 'La versión {v} está lista para instalar.',
  'upd.update': 'Actualizar',
  'upd.cancel': 'Cancelar',
  'upd.installing': 'Descargando e instalando…',
  'upd.badgeLabel': 'Actualización disponible'
};

const messages: Record<Locale, Record<string, string>> = { en, es };
