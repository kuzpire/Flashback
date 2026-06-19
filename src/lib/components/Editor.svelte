<script lang="ts">
  import Icon from './Icon.svelte';
  import {
    editorState,
    inMs,
    outMs,
    closeEditor,
    resetTrim,
    exportClip,
    markEdited,
    cutAtPlayhead,
    removeSegment,
    moveBoundary,
    persistEdit,
  } from '$lib/editor.svelte';
  import { formatDuration, formatDurationMs, formatSize } from '$lib/clips';
  import { refreshLibrary } from '$lib/library.svelte';

  let video = $state<HTMLVideoElement | null>(null);
  let sysAudio = $state<HTMLAudioElement | null>(null);
  let micAudio = $state<HTMLAudioElement | null>(null);

  let playing = $state(false);
  let currentTime = $state(0);
  let duration = $state(0);
  let raf = 0;

  let notice = $state<string | null>(null);

  const FRAME_MS = $derived(1000 / (editorState.fps || 30));

  let timelineEl = $state<HTMLDivElement | null>(null);
  let draggingPlayhead = false;
  let draggingBoundary: { index: number } | null = null;

  const hasSeparate = $derived(!!(editorState.system || editorState.mic));

  const hasTrim = $derived(
    editorState.durationMs > 0 &&
      editorState.segments.length > 0 &&
      !(
        editorState.segments.length === 1 &&
        editorState.segments[0].startMs === 0 &&
        editorState.segments[0].endMs >= editorState.durationMs - 16
      )
  );

  const playPct = $derived(duration > 0 ? (currentTime / duration) * 100 : 0);

  const segPcts = $derived(
    editorState.durationMs > 0
      ? editorState.segments.map(s => ({
          left: (s.startMs / editorState.durationMs) * 100,
          width: ((s.endMs - s.startMs) / editorState.durationMs) * 100,
        }))
      : []
  );

  const COLORS = ['#4a9eff', '#ff6b6b', '#51cf66', '#fcc419', '#cc5de8', '#ff922b'];

  function applyAudio() {
    if (video) video.muted = hasSeparate;
    if (sysAudio) sysAudio.volume = editorState.mixer.sys_muted ? 0 : editorState.mixer.sys_vol;
    if (micAudio) micAudio.volume = editorState.mixer.mic_muted ? 0 : editorState.mixer.mic_vol;
  }

  $effect(() => {
    void [
      editorState.mixer.sys_vol,
      editorState.mixer.mic_vol,
      editorState.mixer.sys_muted,
      editorState.mixer.mic_muted,
      editorState.system,
      editorState.mic,
      video,
      sysAudio,
      micAudio,
    ];
    applyAudio();
  });

  $effect(() => () => cancelAnimationFrame(raf));

  function alignAudios(t: number) {
    for (const a of [sysAudio, micAudio]) {
      if (a && Math.abs(a.currentTime - t) > 0.05) a.currentTime = t;
    }
  }

  function tick() {
    if (!video) return;
    currentTime = video.currentTime;
    if (playing) {
      const t = video.currentTime;
      for (const a of [sysAudio, micAudio]) {
        if (a && Math.abs(a.currentTime - t) > 0.12) a.currentTime = t;
      }
      raf = requestAnimationFrame(tick);
    }
  }

  async function play() {
    if (!video) return;
    try {
      const ct = video.currentTime * 1000;
      if (editorState.durationMs > 0 && (ct < inMs() || ct > outMs())) {
        video.currentTime = inMs() / 1000;
        alignAudios(inMs() / 1000);
      }
      await video.play();
      await sysAudio?.play();
      await micAudio?.play();
      playing = true;
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(tick);
    } catch (e) {
      console.error('editor play', e);
    }
  }

  function pause() {
    video?.pause();
    sysAudio?.pause();
    micAudio?.pause();
    playing = false;
    cancelAnimationFrame(raf);
  }

  function toggle() {
    if (playing) pause();
    else play();
  }

  function onLoaded() {
    duration = video?.duration || 0;
    const dMs = duration * 1000;
    editorState.durationMs = dMs;
    if (editorState.segments.length === 0) {
      editorState.segments = [{ startMs: 0, endMs: dMs }];
    }
    applyAudio();
  }

  function seekToClientX(clientX: number) {
    if (!timelineEl || editorState.durationMs <= 0) return;
    const rect = timelineEl.getBoundingClientRect();
    let pct = (clientX - rect.left) / rect.width;
    pct = Math.max(0, Math.min(1, pct));
    const ms = pct * editorState.durationMs;
    if (video) {
      video.currentTime = ms / 1000;
      currentTime = video.currentTime;
    }
    alignAudios(currentTime);
  }

  function onTimelineMouseDown(e: MouseEvent) {
    if (!timelineEl || editorState.durationMs <= 0) return;
    const rect = timelineEl.getBoundingClientRect();
    const pct = (e.clientX - rect.left) / rect.width;

    for (let i = 0; i < editorState.segments.length - 1; i++) {
      const boundaryPct = editorState.segments[i].endMs / editorState.durationMs;
      if (Math.abs(pct - boundaryPct) * rect.width < 8) {
        draggingBoundary = { index: i };
        e.preventDefault();
        return;
      }
    }

    draggingPlayhead = true;
    seekToClientX(e.clientX);
    e.preventDefault();
  }

  function onTimelineMouseMove(e: MouseEvent) {
    if (draggingPlayhead) {
      seekToClientX(e.clientX);
      return;
    }
    if (draggingBoundary && timelineEl && editorState.durationMs > 0) {
      const rect = timelineEl.getBoundingClientRect();
      let pct = (e.clientX - rect.left) / rect.width;
      pct = Math.max(0, Math.min(1, pct));
      const ms = pct * editorState.durationMs;
      moveBoundary(draggingBoundary.index, ms);
    }
  }

  function onTimelineMouseUp() {
    draggingPlayhead = false;
    draggingBoundary = null;
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') close();
    else if (e.key === 'f' || e.key === 'F') {
      e.preventDefault();
      cutAtPlayhead(currentTime * 1000);
    } else if (e.key === ' ' || e.key === 'k') {
      e.preventDefault();
      toggle();
    } else if (e.key === 'ArrowLeft') {
      e.preventDefault();
      stepFrame(-1);
    } else if (e.key === 'ArrowRight') {
      e.preventDefault();
      stepFrame(1);
    }
  }

  function stepFrame(dir: number) {
    if (!video) return;
    const t = video.currentTime + (dir * FRAME_MS) / 1000;
    video.currentTime = Math.max(0, Math.min(t, duration));
    currentTime = video.currentTime;
    alignAudios(currentTime);
  }

  function goToStart() {
    if (!video) return;
    video.currentTime = 0;
    currentTime = 0;
    alignAudios(0);
  }

  function goToEnd() {
    if (!video) return;
    video.currentTime = duration;
    currentTime = duration;
    alignAudios(duration);
  }

  async function handleExport() {
    try {
      const dst = await exportClip();
      if (dst) {
        await refreshLibrary();
        notice = `Exportado: ${dst.split(/[/\\]/).pop()}`;
        setTimeout(() => (notice = null), 4000);
      }
    } catch (e) {
      notice = `Error al exportar: ${e}`;
      console.error('export', e);
      setTimeout(() => (notice = null), 6000);
    }
  }

  async function close() {
    pause();
    await persistEdit();
    closeEditor();
  }

  function fmtDate(d?: Date): string {
    if (!d) return '';
    try {
      return d.toLocaleDateString('es-ES', { day: 'numeric', month: 'short', year: 'numeric' });
    } catch {
      return '';
    }
  }

  function shortPath(p: string): string {
    if (!p) return '';
    const parts = p.split(/[/\\]/);
    if (parts.length <= 3) return p;
    return `${parts[0]}\\...\\${parts[parts.length - 2]}\\${parts[parts.length - 1]}`;
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="overlay">
  <div class="sub-bar mono">
    <div class="sub-left">
      <div class="sub-chevrons">
        <button class="sub-btn" aria-label="Clip anterior">
          <svg width="256" height="256" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg>
        </button>
        <button class="sub-btn" aria-label="Clip siguiente">
          <svg width="256" height="256" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>
        </button>
      </div>
      <span class="sub-sep">|</span>
      <span class="sub-label">Creado:</span>
      <span class="sub-info">{fmtDate(editorState.clip?.createdAt)}</span>
      <span class="sub-sep">|</span>
      <span class="sub-label">Tamaño:</span>
      <span class="sub-info">{formatSize(editorState.clip?.sizeBytes ?? 0)}</span>
      <span class="sub-sep">|</span>
      <span class="sub-label">Ruta:</span>
      <span class="sub-path">{shortPath(editorState.clip?.path ?? '')}</span>
    </div>
    <button class="sub-close" aria-label="Cerrar editor" onclick={close}>✕</button>
  </div>

  <div class="stage">
    {#if editorState.videoSrc}
      <video
        bind:this={video}
        src={editorState.videoSrc}
        playsinline
        onloadedmetadata={onLoaded}
        onended={pause}
        onclick={toggle}
      ><track kind="captions" /></video>
    {/if}
    {#if editorState.system}
      <audio bind:this={sysAudio} src={editorState.system} preload="auto"></audio>
    {/if}
    {#if editorState.mic}
      <audio bind:this={micAudio} src={editorState.mic} preload="auto"></audio>
    {/if}

    {#if editorState.loading}
      <div class="prep mono">Preparando pistas de audio…</div>
    {:else if editorState.error}
      <div class="prep mono err">No se pudieron separar las pistas: {editorState.error}</div>
    {/if}
  </div>

  <div class="controls">
    <!-- Play controls row (above timeline) -->
    <div class="ctrl-row">
      <div class="ctrl-left">
        <button class="nav-btn" aria-label="Ir al inicio" onclick={goToStart}>
          <Icon name="skip-back" size={16} />
        </button>
        <button class="nav-btn" aria-label="Retroceder un fotograma" onclick={() => stepFrame(-1)}>
          <Icon name="step-back" size={16} />
        </button>
        <button class="play-btn" aria-label={playing ? 'Pausar' : 'Reproducir'} onclick={toggle}>
          <Icon name={playing ? 'stop' : 'play'} size={20} />
        </button>
        <button class="nav-btn" aria-label="Avanzar un fotograma" onclick={() => stepFrame(1)}>
          <Icon name="step-fwd" size={16} />
        </button>
        <button class="nav-btn" aria-label="Ir al final" onclick={goToEnd}>
          <Icon name="skip-fwd" size={16} />
        </button>
        <button class="nav-btn" aria-label="Pantalla completa">
          <Icon name="fullscreen" size={15} />
        </button>
      </div>
      <div class="ctrl-right">
        <button class="act-btn" onclick={resetTrim}>Reestablecer</button>
        <button class="act-btn export" onclick={handleExport} disabled={editorState.exporting}>
          {editorState.exporting ? 'Exportando…' : 'Exportar'}
        </button>
      </div>
    </div>

    <!-- Timeline bar -->
    {#if editorState.durationMs > 0}
      <!-- svelte-ignore a11y_role_has_required_aria_props -->
      <div
        class="timeline"
        role="slider"
        tabindex="-1"
        aria-label="Línea de tiempo"
        aria-valuenow={currentTime}
        aria-valuemin={0}
        aria-valuemax={duration}
        bind:this={timelineEl}
        onmousedown={onTimelineMouseDown}
        onmousemove={onTimelineMouseMove}
        onmouseup={onTimelineMouseUp}
        onmouseleave={onTimelineMouseUp}
      >
        {#each segPcts as seg, i}
          <div
            class="tl-segment"
            class:active={i === editorState.activeSegment}
            style="left: {seg.left}%; width: {seg.width}%; background: {COLORS[i % COLORS.length]};"
          >
            {#if editorState.segments.length > 1}
              <button
                class="seg-remove"
                aria-label="Eliminar segmento"
                onclick={(e) => { e.stopPropagation(); removeSegment(i); }}
              >✕</button>
            {/if}
          </div>
        {/each}
        {#each editorState.segments.slice(0, -1) as _, i}
          <div
            class="tl-boundary"
            style="left: {editorState.segments[i].endMs / editorState.durationMs * 100}%"
          ></div>
        {/each}
        <div class="tl-playhead" style="left: {playPct}%"></div>
      </div>
    {/if}

    <!-- Audio tracks -->
    {#if hasSeparate}
      {#if editorState.system}
        <div class="track-audio">
          <div class="track-left">
            <div class="track-head">
              <span class="track-label mono">Audio del sistema</span>
            </div>
            <div class="vol-row">
              <button
                class="spk-btn"
                class:muted={editorState.mixer.sys_muted}
                aria-label="Silenciar audio del sistema"
                onclick={() => {
                  editorState.mixer.sys_muted = !editorState.mixer.sys_muted;
                  markEdited();
                }}
              >
                <Icon name="speaker" size={14} />
              </button>
              <input
                class="vol-slider"
                type="range"
                min="0"
                max="1"
                step="0.01"
                value={editorState.mixer.sys_vol}
                oninput={(e) => {
                  editorState.mixer.sys_vol = Number((e.target as HTMLInputElement).value);
                  markEdited();
                }}
              />
            </div>
          </div>
          <div class="track-bar">
            {#if editorState.durationMs > 0}
              {#each segPcts as seg}
                <div class="track-bar-seg" style="left: {seg.left}%; width: {seg.width}%;"></div>
              {/each}
            {/if}
          </div>
        </div>
      {/if}
      {#if editorState.mic}
        <div class="track-audio">
          <div class="track-left">
            <div class="track-head">
              <span class="track-label mono">Audio del micrófono</span>
            </div>
            <div class="vol-row">
              <button
                class="spk-btn"
                class:muted={editorState.mixer.mic_muted}
                aria-label="Silenciar audio del micrófono"
                onclick={() => {
                  editorState.mixer.mic_muted = !editorState.mixer.mic_muted;
                  markEdited();
                }}
              >
                <Icon name="speaker" size={14} />
              </button>
              <input
                class="vol-slider"
                type="range"
                min="0"
                max="1"
                step="0.01"
                value={editorState.mixer.mic_vol}
                oninput={(e) => {
                  editorState.mixer.mic_vol = Number((e.target as HTMLInputElement).value);
                  markEdited();
                }}
              />
            </div>
          </div>
          <div class="track-bar">
            {#if editorState.durationMs > 0}
              {#each segPcts as seg}
                <div class="track-bar-seg" style="left: {seg.left}%; width: {seg.width}%;"></div>
              {/each}
            {/if}
          </div>
        </div>
      {/if}
    {:else if !editorState.loading && !editorState.error}
      <div class="single mono">Este clip tiene una sola pista de audio.</div>
    {/if}
  </div>

  {#if notice}
    <div class="notice mono">{notice}</div>
  {/if}
</div>

<style>
  .overlay {
    position: fixed;
    left: 0;
    right: 0;
    top: var(--topbar-h, 60px);
    bottom: 0;
    z-index: 100;
    display: flex;
    flex-direction: column;
    background: var(--bg-0);
  }

  .sub-bar {
    height: 35px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 14px;
    border-bottom: 1px solid var(--line);
    background: var(--bg-1);
    font-size: 12px;
    color: var(--text-3);
  }
  .sub-left {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .sub-chevrons {
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .sub-btn {
    width: 26px;
    height: 26px;
    display: grid;
    place-items: center;
    color: var(--text-1);
    border-radius: 4px;
    transition: color 0.14s ease, background 0.14s ease;
  }
  .sub-btn svg {
    width: 18px;
    height: 18px;
  }
  .sub-btn:hover {
    color: var(--text-0);
    background: var(--bg-2);
  }
  .sub-sep {
    color: var(--line);
    font-size: 13px;
  }
  .sub-label {
    color: var(--text-3);
    white-space: nowrap;
  }
  .sub-info {
    color: var(--text-1);
    white-space: nowrap;
  }
  .sub-path {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 260px;
    color: var(--text-2);
  }
  .sub-close {
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    font-size: 15px;
    color: var(--text-2);
    border-radius: 4px;
    flex-shrink: 0;
    transition: color 0.14s ease, background 0.14s ease;
  }
  .sub-close:hover {
    color: var(--text-0);
    background: var(--bg-2);
  }

  /* Stage (video) */
  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
    display: grid;
    place-items: start center;
    background: #000;
    overflow: hidden;
    padding: 16px 40px 0;
  }
  .stage video {
    max-width: 100%;
    max-height: 72%;
    display: block;
    cursor: pointer;
    object-fit: contain;
  }
  .prep {
    position: absolute;
    bottom: 16px;
    left: 50%;
    transform: translateX(-50%);
    padding: 7px 12px;
    font-size: 12px;
    color: var(--text-1);
    background: var(--bg-2);
    border: 1px solid var(--line);
    border-radius: 8px;
  }
  .prep.err {
    color: var(--rec);
    border-color: color-mix(in srgb, var(--rec) 50%, var(--line));
  }

  /* Controls area */
  .controls {
    flex-shrink: 0;
    border-top: 1px solid var(--line);
    background: var(--bg-1);
    padding: 8px 14px 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  /* Play controls row */
  .ctrl-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    min-height: 36px;
  }
  .ctrl-left {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .ctrl-right {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .nav-btn {
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    color: var(--text-2);
    border-radius: 6px;
    transition: color 0.14s ease, background 0.14s ease;
  }
  .nav-btn:hover {
    color: var(--text-0);
    background: var(--bg-2);
  }
  .play-btn {
    width: 34px;
    height: 34px;
    display: grid;
    place-items: center;
    color: var(--text-0);
    background: var(--bg-3);
    border-radius: 999px;
    transition: background 0.14s ease;
    flex-shrink: 0;
    margin: 0 2px;
  }
  .play-btn:hover {
    background: var(--bg-hover);
  }
  .act-btn {
    padding: 5px 10px;
    font-size: 11.5px;
    color: var(--text-1);
    background: var(--bg-2);
    border: 1px solid var(--line);
    border-radius: var(--r-sm);
    transition: background 0.14s ease, color 0.14s ease;
  }
  .act-btn:hover {
    background: var(--bg-hover);
    color: var(--text-0);
  }
  .act-btn.export {
    color: var(--bright);
    background: rgba(240, 242, 247, 0.1);
    border-color: rgba(240, 242, 247, 0.3);
  }
  .act-btn.export:hover {
    background: rgba(240, 242, 247, 0.18);
  }
  .act-btn:disabled {
    opacity: 0.5;
    pointer-events: none;
  }

  /* Timeline bar */
  .timeline {
    position: relative;
    height: 34px;
    cursor: pointer;
    user-select: none;
    border-radius: 6px;
    overflow: hidden;
    background: var(--bg-3);
  }
  .tl-segment {
    position: absolute;
    top: 2px;
    bottom: 2px;
    border-radius: 4px;
    opacity: 0.75;
    transition: opacity 0.1s ease;
    display: flex;
    align-items: center;
    padding: 0 3px;
    min-width: 6px;
    overflow: hidden;
  }
  .tl-segment.active {
    opacity: 1;
    box-shadow: inset 0 0 0 1px rgba(255,255,255,0.3);
  }
  .seg-remove {
    width: 14px;
    height: 14px;
    display: none;
    place-items: center;
    font-size: 7px;
    color: rgba(255,255,255,0.9);
    background: rgba(0,0,0,0.35);
    border-radius: 999px;
    flex-shrink: 0;
    cursor: pointer;
  }
  .tl-segment:hover .seg-remove {
    display: grid;
  }
  .tl-boundary {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 3px;
    background: rgba(255,255,255,0.55);
    cursor: ew-resize;
    z-index: 3;
    transform: translateX(-1.5px);
  }
  .tl-boundary:hover {
    background: rgba(255,255,255,0.85);
  }
  .tl-playhead {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    background: var(--bright, #fff);
    z-index: 5;
    pointer-events: none;
    transform: translateX(-1px);
  }

  /* Audio tracks */
  .track-audio {
    display: flex;
    align-items: stretch;
    gap: 10px;
    min-height: 56px;
  }
  .track-left {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 6px;
    flex-shrink: 0;
    width: 190px;
    padding: 8px 10px;
    border-radius: 8px;
    background: var(--bg-2);
    border: 1px solid var(--line);
  }
  .track-head {
    display: flex;
    align-items: center;
  }
  .track-label {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-1);
    white-space: nowrap;
  }
  .vol-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .spk-btn {
    width: 24px;
    height: 24px;
    display: grid;
    place-items: center;
    color: var(--text-1);
    border-radius: 4px;
    flex-shrink: 0;
    transition: color 0.14s ease, background 0.14s ease;
  }
  .spk-btn:hover {
    background: var(--bg-hover);
    color: var(--text-0);
  }
  .spk-btn.muted {
    color: var(--rec);
    opacity: 0.6;
  }
  .vol-slider {
    -webkit-appearance: none;
    appearance: none;
    width: 100%;
    height: 6px;
    border-radius: 3px;
    background: var(--bg-3);
    cursor: pointer;
  }
  .vol-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 18px;
    height: 14px;
    border-radius: 3px;
    background: var(--bright, #fff);
    border: none;
  }

  /* Track bars */
  .track-bar {
    flex: 1;
    position: relative;
    border-radius: 6px;
    overflow: hidden;
    min-height: 100%;
    background: var(--bg-2);
    border: 1px solid var(--line);
  }
  .track-bar-seg {
    position: absolute;
    top: 1px;
    bottom: 1px;
    border-radius: 2px;
    min-width: 2px;
    background: var(--text-3);
    opacity: 0.15;
  }

  .single {
    font-size: 12px;
    color: var(--text-3);
    padding: 8px 0;
  }

  .notice {
    position: absolute;
    bottom: 90px;
    left: 50%;
    transform: translateX(-50%);
    padding: 8px 14px;
    font-size: 11.5px;
    color: var(--text-1);
    background: var(--bg-2);
    border: 1px solid var(--line);
    border-radius: 8px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.35);
    z-index: 110;
    max-width: 70%;
    text-align: center;
    pointer-events: none;
  }
</style>
