export type Clip = {
  id: string;
  title: string;
  source: string;
  durationSec: number;
  sizeBytes: number;
  createdAt: Date;
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

export function formatSize(bytes: number): string {
  const mb = bytes / MB;
  if (mb >= 1000) return `${(mb / 1024).toFixed(1)} GB`;
  return `${mb.toFixed(1)} MB`;
}

export function formatRelative(date: Date): string {
  const min = Math.round((Date.now() - date.getTime()) / 60_000);
  if (min < 1) return 'Ahora mismo';
  if (min < 60) return `Hace ${min} min`;
  const h = Math.round(min / 60);
  if (h === 1) return 'Hace una hora';
  if (h < 24) return `Hace ${h} horas`;
  const d = Math.round(h / 24);
  if (d === 1) return 'Ayer';
  return `Hace ${d} días`;
}

function startOfDay(d: Date): number {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

const MONTHS = ['ene', 'feb', 'mar', 'abr', 'may', 'jun', 'jul', 'ago', 'sep', 'oct', 'nov', 'dic'];

export function dayLabel(date: Date): string {
  const today = startOfDay(new Date());
  const day = startOfDay(date);
  const diff = Math.round((today - day) / 86_400_000);
  if (diff === 0) return 'Hoy';
  if (diff === 1) return 'Ayer';
  return `${date.getDate()} ${MONTHS[date.getMonth()]} ${date.getFullYear()}`;
}

export type ClipGroup = { label: string; source: string; clips: Clip[] };

export function groupClips(list: Clip[]): ClipGroup[] {
  const sorted = [...list].sort((a, b) => b.createdAt.getTime() - a.createdAt.getTime());
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
