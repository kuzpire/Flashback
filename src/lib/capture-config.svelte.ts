const FPS_KEY = 'flashback.capture.fps';
const QUALITY_KEY = 'flashback.capture.quality';

export const FPS_OPTIONS = [20, 30, 60, 120, 144];

export type QualityKey = 'low' | 'normal' | 'high' | 'ultra';

export const QUALITY_OPTIONS: { key: QualityKey; label: string }[] = [
  { key: 'low', label: 'Bajo' },
  { key: 'normal', label: 'Medio' },
  { key: 'high', label: 'Alto' },
  { key: 'ultra', label: 'Ultra' }
];

function loadFps(): number {
  if (typeof localStorage === 'undefined') return 60;
  const n = Number(localStorage.getItem(FPS_KEY));
  return FPS_OPTIONS.includes(n) ? n : 60;
}

function loadQuality(): QualityKey {
  if (typeof localStorage === 'undefined') return 'high';
  const q = localStorage.getItem(QUALITY_KEY);
  return QUALITY_OPTIONS.some((o) => o.key === q) ? (q as QualityKey) : 'high';
}

// Config de captura compartida por la barra superior y los ajustes. Alimenta el backend
// al iniciar grabación/replay (fps + calidad). La resolución aún no se reescala.
export const captureConfig = $state<{ fps: number; quality: QualityKey }>({
  fps: loadFps(),
  quality: loadQuality()
});

export function qualityLabel(key: QualityKey): string {
  return QUALITY_OPTIONS.find((o) => o.key === key)?.label ?? key;
}

export function setFps(fps: number) {
  captureConfig.fps = fps;
  persist(FPS_KEY, String(fps));
}

export function setQuality(quality: QualityKey) {
  captureConfig.quality = quality;
  persist(QUALITY_KEY, quality);
}

function persist(key: string, value: string) {
  if (typeof localStorage === 'undefined') return;
  try {
    localStorage.setItem(key, value);
  } catch {
    // sin persistencia disponible
  }
}
