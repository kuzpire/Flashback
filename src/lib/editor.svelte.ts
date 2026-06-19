import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import type { Clip } from './clips';

type ClipAudio = { system: string | null; mic: string | null };

export type MixerState = {
  sys_vol: number;
  sys_muted: boolean;
  mic_vol: number;
  mic_muted: boolean;
};

const defaultMixer: MixerState = {
  sys_vol: 1,
  sys_muted: false,
  mic_vol: 1,
  mic_muted: false,
};

export type Segment = {
  startMs: number;
  endMs: number;
};

export const editorState = $state<{
  clip: Clip | null;
  videoSrc: string | null;
  system: string | null;
  mic: string | null;
  loading: boolean;
  error: string | null;
  segments: Segment[];
  activeSegment: number;
  durationMs: number;
  keyframes: number[];
  fps: number;
  mixer: MixerState;
  exporting: boolean;
}>({
  clip: null,
  videoSrc: null,
  system: null,
  mic: null,
  loading: false,
  error: null,
  segments: [],
  activeSegment: 0,
  durationMs: 0,
  keyframes: [],
  fps: 30,
  mixer: { ...defaultMixer },
  exporting: false,
});

function resetEditorState() {
  editorState.clip = null;
  editorState.videoSrc = null;
  editorState.system = null;
  editorState.mic = null;
  editorState.error = null;
  editorState.loading = false;
  editorState.segments = [];
  editorState.activeSegment = 0;
  editorState.durationMs = 0;
  editorState.keyframes = [];
  editorState.fps = 30;
  editorState.mixer = { ...defaultMixer };
  editorState.exporting = false;
}

export function inMs(): number {
  return editorState.segments[0]?.startMs ?? 0;
}
export function outMs(): number {
  return editorState.segments.length > 0
    ? editorState.segments[editorState.segments.length - 1].endMs
    : 0;
}

export function openEditor(clip: Clip) {
  resetEditorState();
  editorState.clip = clip;
  editorState.videoSrc = clip.previewSrc ?? null;
  editorState.loading = true;

  (async () => {
    if (!clip.path) { editorState.error = 'clip sin ruta'; editorState.loading = false; return; }

    try {
      editorState.keyframes = await invoke<number[]>('keyframe_times', { path: clip.path });
    } catch {
      editorState.keyframes = [];
    }

    try {
      const fps = await invoke<number>('clip_fps', { path: clip.path });
      if (fps > 0) editorState.fps = fps;
    } catch {
      /* fps por defecto */
    }

    try {
      const saved = await invoke<{ segments: { start_ms: number; end_ms: number }[]; mixer: MixerState }>(
        'load_clip_edit',
        { path: clip.path },
      );
      if (saved?.segments?.length) {
        editorState.segments = saved.segments.map((s) => ({ startMs: s.start_ms, endMs: s.end_ms }));
        editorState.activeSegment = 0;
      }
      if (saved?.mixer) editorState.mixer = { ...defaultMixer, ...saved.mixer };
    } catch {
      /* sin edición previa */
    }

    try {
      const res = await invoke<ClipAudio>('prepare_clip_audio', { path: clip.path });
      editorState.system = res.system ? convertFileSrc(res.system) : null;
      editorState.mic = res.mic ? convertFileSrc(res.mic) : null;
    } catch (e) {
      editorState.error = String(e);
    } finally {
      editorState.loading = false;
    }
  })();
}

export async function exportClip() {
  if (!editorState.clip?.path || editorState.segments.length === 0) return;
  editorState.exporting = true;
  try {
    const src = editorState.clip.path;
    const dot = src.lastIndexOf('.');
    const dst = dot > 0 ? `${src.slice(0, dot)}_edit.mp4` : `${src}_edit.mp4`;

    await invoke('export_clip', {
      src,
      dst,
      edit: {
        segments: editorState.segments.map(s => ({ start_ms: s.startMs, end_ms: s.endMs })),
        mixer: editorState.mixer,
      },
    });
    return dst;
  } catch (e) {
    throw e;
  } finally {
    editorState.exporting = false;
  }
}

export function resetTrim() {
  if (editorState.durationMs > 0) {
    editorState.segments = [{ startMs: 0, endMs: editorState.durationMs }];
    editorState.activeSegment = 0;
    markEdited();
  }
}

export function cutAtPlayhead(playheadMs: number) {
  if (editorState.durationMs <= 0) return;
  const idx = editorState.segments.findIndex(s => playheadMs > s.startMs && playheadMs < s.endMs);
  if (idx < 0) return;
  const seg = editorState.segments[idx];
  if (playheadMs - seg.startMs < 100 || seg.endMs - playheadMs < 100) return;
  const newSegs = [...editorState.segments];
  newSegs.splice(idx, 1,
    { startMs: seg.startMs, endMs: playheadMs },
    { startMs: playheadMs, endMs: seg.endMs },
  );
  editorState.segments = newSegs;
  editorState.activeSegment = idx + 1;
  markEdited();
}

export function removeSegment(index: number) {
  if (editorState.segments.length <= 1) return;
  const newSegs = editorState.segments.filter((_, i) => i !== index);
  editorState.segments = newSegs;
  if (editorState.activeSegment >= newSegs.length) {
    editorState.activeSegment = newSegs.length - 1;
  }
  markEdited();
}

export function moveSegment(fromIndex: number, toIndex: number) {
  if (fromIndex === toIndex) return;
  const segs = [...editorState.segments];
  const [moved] = segs.splice(fromIndex, 1);
  segs.splice(toIndex, 0, moved);
  editorState.segments = segs;
  editorState.activeSegment = toIndex;
  markEdited();
}

export function moveBoundary(index: number, newMs: number) {
  const segs = editorState.segments.map((s) => ({ ...s }));
  if (index < 0 || index >= segs.length - 1) return;
  const left = segs[index];
  const right = segs[index + 1];
  const gap = right.startMs - left.endMs;

  const minEnd = left.startMs + 16;
  const maxEnd = right.endMs - gap - 16;
  const newEnd = Math.max(minEnd, Math.min(newMs, maxEnd));

  left.endMs = newEnd;
  right.startMs = newEnd + gap;
  segs[index] = left;
  segs[index + 1] = right;
  editorState.segments = segs;
  markEdited();
}

let persistTimer: ReturnType<typeof setTimeout> | null = null;

export async function persistEdit() {
  if (!editorState.clip?.path || editorState.durationMs <= 0) return;
  try {
    await invoke('save_clip_edit', {
      path: editorState.clip.path,
      edit: {
        segments: editorState.segments.map((s) => ({ start_ms: s.startMs, end_ms: s.endMs })),
        mixer: editorState.mixer,
      },
    });
  } catch (e) {
    console.error('save_clip_edit', e);
  }
}

export function markEdited() {
  if (editorState.clip) editorState.clip.edited = true;
  if (persistTimer) clearTimeout(persistTimer);
  persistTimer = setTimeout(() => {
    persistTimer = null;
    void persistEdit();
  }, 400);
}

export function closeEditor() {
  if (persistTimer) {
    clearTimeout(persistTimer);
    persistTimer = null;
  }
  resetEditorState();
}
