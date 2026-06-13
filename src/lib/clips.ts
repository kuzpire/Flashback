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
const minutesAgo = (m: number) => new Date(Date.now() - m * 60_000);

export const clips: Clip[] = [
  { id: 'c1', title: 'Vídeo de Flashback', source: 'MINECRAFT', durationSec: 59, sizeBytes: 142 * MB, createdAt: minutesAgo(95), favorite: true },
  { id: 'c2', title: 'Clip recortado', source: 'MINECRAFT', durationSec: 18, sizeBytes: 41 * MB, createdAt: minutesAgo(120), trimmed: true },
  { id: 'c3', title: 'Clip recortado', source: 'MINECRAFT', durationSec: 19, sizeBytes: 44 * MB, createdAt: minutesAgo(64), trimmed: true },
  { id: 'c4', title: 'Vídeo de Flashback', source: 'MINECRAFT', durationSec: 18, sizeBytes: 39 * MB, createdAt: minutesAgo(180), edited: true },
  { id: 'c5', title: 'Vídeo de Flashback', source: 'MINECRAFT', durationSec: 118, sizeBytes: 274 * MB, createdAt: minutesAgo(210) },
  { id: 'c6', title: 'Ace en el sitio', source: 'VALORANT', durationSec: 119, sizeBytes: 286 * MB, createdAt: minutesAgo(60 * 26), favorite: true },
  { id: 'c7', title: 'Clip recortado', source: 'VALORANT', durationSec: 32, sizeBytes: 73 * MB, createdAt: minutesAgo(60 * 28), trimmed: true },
  { id: 'c8', title: 'Remontada 1v3', source: 'CS2', durationSec: 47, sizeBytes: 109 * MB, createdAt: minutesAgo(60 * 52) }
];

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

const THUMBS = [
  'linear-gradient(135deg, #1e2331 0%, #2a3f6e 55%, #3f6df5 135%)',
  'linear-gradient(135deg, #20242e 0%, #2b3550 60%, #47628f 125%)',
  'linear-gradient(135deg, #221f30 0%, #34305a 60%, #5a4f9a 125%)',
  'linear-gradient(135deg, #1d2530 0%, #244055 55%, #357a8a 125%)',
  'linear-gradient(135deg, #262430 0%, #3a3552 55%, #6a5f8a 125%)',
  'linear-gradient(135deg, #1e222c 0%, #2a3346 60%, #45567a 125%)'
];

export function thumbBackground(id: string): string {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return THUMBS[h % THUMBS.length];
}
