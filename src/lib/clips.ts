import { t, localeTag } from './i18n.svelte';

export type Clip = {
  id: string;
  title: string;
  source: string;
  durationSec: number;
  sizeBytes: number;
  createdAt: Date;
  path: string;
  trimmed?: boolean;
  edited?: boolean;
  favorite?: boolean;
  previewSrc?: string;
  poster?: string;
};

const MB = 1024 * 1024;

const pad2 = (n: number) => String(n).padStart(2, '0');

export function formatDuration(sec: number): string {
  const m = Math.floor(sec / 60);
  const s = Math.floor(sec % 60);
  return `${pad2(m)}:${pad2(s)}`;
}

export function formatDurationMs(ms: number): string {
  const totalSec = Math.floor(ms / 1000);
  const m = Math.floor(totalSec / 60);
  const s = totalSec % 60;
  const millis = ms % 1000;
  return `${pad2(m)}:${pad2(s)}.${String(millis).padStart(3, '0')}`;
}

export function formatSize(bytes: number): string {
  const mb = bytes / MB;
  if (mb >= 1000) return `${(mb / 1024).toFixed(1)} GB`;
  return `${mb.toFixed(1)} MB`;
}

export function formatRelative(date: Date): string {
  const min = Math.round((Date.now() - date.getTime()) / 60_000);
  if (min < 1) return t('time.now');
  if (min < 60) return t('time.minAgo', { n: min });
  const h = Math.round(min / 60);
  if (h < 24) return t(h === 1 ? 'time.hourAgo' : 'time.hoursAgo', { n: h });
  const d = Math.round(h / 24);
  if (d === 1) return t('time.yesterday');
  return t('time.daysAgo', { n: d });
}

function startOfDay(d: Date): number {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

export function dayLabel(date: Date): string {
  const today = startOfDay(new Date());
  const day = startOfDay(date);
  const diff = Math.round((today - day) / 86_400_000);
  if (diff === 0) return t('time.today');
  if (diff === 1) return t('time.yesterday');
  return date.toLocaleDateString(localeTag(), { day: 'numeric', month: 'short', year: 'numeric' });
}

// Las capturas de pantalla guardan el origen como "Pantalla N"; cualquier otro origen es un
// juego. Es la misma convención con la que el backend rellena el `source` al capturar.
export function isScreenSource(source: string): boolean {
  return /^(?:pantalla|screen)\b/i.test(source.trim());
}

// El `source` de las pantallas se persiste canónico ("Pantalla N", el label del backend), pero se
// muestra en el idioma activo. Localiza cada tramo (group.source puede unir varios con " · ");
// los orígenes de juego pasan tal cual.
export function displaySource(source: string): string {
  return source
    .split(' · ')
    .map((part) => {
      const m = part.trim().match(/^(?:pantalla|screen)\s+(\d+)$/i);
      return m ? `${t('cap.screen')} ${m[1]}` : part;
    })
    .join(' · ');
}

export type LibraryFilter = { kind: 'edited' } | { kind: 'source'; value: string };

export function sameFilter(a: LibraryFilter, b: LibraryFilter): boolean {
  if (a.kind !== b.kind) return false;
  return a.kind === 'source' ? a.value === (b as { kind: 'source'; value: string }).value : true;
}

// Sin filtros seleccionados se muestran todos; con varios, basta con que el clip cumpla uno (OR).
export function clipMatchesFilters(clip: Clip, selected: LibraryFilter[]): boolean {
  if (selected.length === 0) return true;
  return selected.some((f) => (f.kind === 'edited' ? !!clip.edited : clip.source === f.value));
}

export type ClipGroup = { label: string; source: string; clips: Clip[] };

export function groupClips(list: Clip[], sortAsc = false): ClipGroup[] {
  const sorted = [...list].sort((a, b) =>
    sortAsc
      ? a.createdAt.getTime() - b.createdAt.getTime()
      : b.createdAt.getTime() - a.createdAt.getTime()
  );
  const groups: ClipGroup[] = [];
  for (const clip of sorted) {
    const label = dayLabel(clip.createdAt);
    const last = groups[groups.length - 1];
    if (last && last.label === label) {
      last.clips.push(clip);
      if (!last.source.includes(clip.source)) last.source = `${last.source} · ${clip.source}`;
    } else {
      groups.push({ label, source: clip.source, clips: [clip] });
    }
  }
  return groups;
}
