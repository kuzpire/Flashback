<script lang="ts">
  import '@fontsource-variable/geist';
  import '../app.css';
  import { page } from '$app/state';
  import Icon from '$lib/components/Icon.svelte';
  import WindowControls from '$lib/components/WindowControls.svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { register, unregister, isRegistered } from '@tauri-apps/plugin-global-shortcut';

  let { children } = $props();

  const RECORD_HOTKEY = 'Alt+F9';

  const nav = [
    { href: '/', icon: 'clips', label: 'Clips' },
    { href: '/favoritos', icon: 'bookmark', label: 'Favoritos' }
  ];

  const isActive = (href: string) =>
    href === '/' ? page.url.pathname === '/' : page.url.pathname.startsWith(href);

  const session = { buffer: '01:00', quality: 'Alto', res: '1080p', fps: '60' };

  type Monitor = {
    id: string;
    label: string;
    width: number;
    height: number;
    primary: boolean;
    thumb: string | null;
  };
  type CapStatus = { running: boolean; frames: number; width: number; height: number; seconds: number };
  type AudioInput = { id: string; name: string };

  let monitors = $state<Monitor[]>([]);
  let selectedMonitor = $state<string | null>(null);
  let pickerOpen = $state(false);
  let recording = $state(false);
  let micOn = $state(false);
  let audioInputs = $state<AudioInput[]>([]);
  let micInput = $state('');

  let cap = $state<CapStatus | null>(null);
  let fps = $state(0);
  let capPoll: ReturnType<typeof setInterval> | null = null;
  let lastFrames = 0;
  let lastTime = 0;

  const activeMonitor = $derived(monitors.find((m) => m.id === selectedMonitor) ?? null);

  async function loadMonitors() {
    try {
      monitors = await invoke<Monitor[]>('list_monitors');
    } catch {
      // fuera de Tauri (preview en navegador)
    }
  }

  async function loadAudioInputs() {
    try {
      audioInputs = await invoke<AudioInput[]>('list_audio_inputs');
      if (!audioInputs.some((d) => d.id === micInput)) {
        micInput = audioInputs[0]?.id ?? '';
      }
    } catch {
      // fuera de Tauri (preview en navegador)
    }
  }

  function togglePicker(e: MouseEvent) {
    e.stopPropagation();
    pickerOpen = !pickerOpen;
    if (pickerOpen) {
      loadMonitors();
      loadAudioInputs();
    }
  }

  function stopPolling() {
    if (capPoll) {
      clearInterval(capPoll);
      capPoll = null;
    }
    cap = null;
    fps = 0;
    lastFrames = 0;
    lastTime = 0;
  }

  // FPS instantáneo: frames entregados entre dos sondeos (WGC no manda frames
  // duplicados, así que en pantalla quieta baja de los 60 y es lo correcto).
  async function pollStatus() {
    try {
      const s = await invoke<CapStatus>('capture_status');
      const now = performance.now();
      if (lastTime) {
        const dt = (now - lastTime) / 1000;
        if (dt > 0) fps = (s.frames - lastFrames) / dt;
      }
      lastFrames = s.frames;
      lastTime = now;
      cap = s;
    } catch {
      // fuera de Tauri (preview en navegador)
    }
  }

  // Elegir pantalla solo fija el objetivo; grabar es aparte (atajo / botón).
  async function selectMonitor(id: string) {
    pickerOpen = false;
    if (id === selectedMonitor) return;
    if (recording) await stopRecording();
    selectedMonitor = id;
  }

  async function backToApp() {
    pickerOpen = false;
    if (recording) await stopRecording();
    selectedMonitor = null;
  }

  async function startRecording() {
    if (recording || !selectedMonitor) return;
    try {
      await invoke('start_capture', { monitorId: selectedMonitor });
      recording = true;
      lastFrames = 0;
      lastTime = performance.now();
      capPoll = setInterval(pollStatus, 1000);
    } catch {
      // fuera de Tauri (preview en navegador)
    }
  }

  async function stopRecording() {
    if (!recording) return;
    stopPolling();
    recording = false;
    try {
      await invoke('stop_capture');
    } catch {
      // fuera de Tauri (preview en navegador)
    }
  }

  function toggleRecording() {
    if (recording) stopRecording();
    else startRecording();
  }

  type Detected = { name: string; steam_appid: number | null };

  let game = $state('');
  let frame = $state('');
  let frameKey = '';

  async function refresh() {
    try {
      const detected = (await invoke<Detected | null>('detect_game')) ?? null;
      game = detected?.name ?? '';
      if (!detected) {
        frame = '';
        frameKey = '';
        return;
      }
      const key = detected.steam_appid ? `steam:${detected.steam_appid}` : `name:${detected.name}`;
      if (key !== frameKey || !frame) {
        const url = await invoke<string | null>('game_hero', {
          name: detected.name,
          steamAppid: detected.steam_appid
        });
        if (url) {
          frame = url;
          frameKey = key;
        }
      }
    } catch {
      // fuera de Tauri (preview en navegador)
    }
  }

  $effect(() => {
    refresh();
    loadMonitors();
    loadAudioInputs();
    const id = setInterval(refresh, 5000);

    let registered = false;
    (async () => {
      try {
        if (!(await isRegistered(RECORD_HOTKEY))) {
          await register(RECORD_HOTKEY, (e) => {
            if (e.state === 'Pressed') toggleRecording();
          });
        }
        registered = true;
      } catch {
        // fuera de Tauri (preview en navegador)
      }
    })();

    return () => {
      clearInterval(id);
      if (capPoll) clearInterval(capPoll);
      if (registered) unregister(RECORD_HOTKEY).catch(() => {});
    };
  });
</script>

<svelte:window onclick={() => (pickerOpen = false)} />

<div class="app">
  <aside class="sidebar" data-tauri-drag-region>
    <a class="logo" href="/" aria-label="Flashback">
      <img src="/favicon.png" alt="Flashback" />
    </a>

    <nav>
      {#each nav as item (item.href)}
        <a
          class="nav-item"
          class:active={isActive(item.href)}
          href={item.href}
          aria-label={item.label}
        >
          <Icon name={item.icon} size={24} />
        </a>
      {/each}
    </nav>

    <a
      class="nav-item settings-tab"
      class:active={isActive('/settings')}
      href="/settings"
      aria-label="Ajustes"
    >
      <Icon name="settings" size={24} />
    </a>
  </aside>

  <div class="main">
    <header class="topbar" data-tauri-drag-region>
      <div class="capture-target">
        <button
          class="capturing"
          class:idle={!selectedMonitor && !game}
          class:screen={!!selectedMonitor}
          class:rec={recording}
          class:open={pickerOpen}
          onclick={togglePicker}
          aria-haspopup="menu"
          aria-expanded={pickerOpen}
        >
          {#if selectedMonitor}
            <span class="cap-icon">
              {#if recording}<span class="rec-dot"></span>{:else}<Icon name="monitor" size={20} />{/if}
            </span>
          {:else}
            <span class="cap-frame" style:background-image={frame ? `url(${frame})` : 'none'}></span>
          {/if}
          <span class="cap-text">
            <span class="cap-label">
              {#if selectedMonitor}
                {recording ? 'Grabando pantalla' : 'Pantalla lista'}
              {:else}
                {game ? 'Capturando clips' : 'En espera'}
              {/if}
            </span>
            <span class="cap-proc">
              {selectedMonitor ? (activeMonitor?.label ?? 'Pantalla') : game || 'Sin juego'}
              <Icon name="chevron-down" size={14} sw={2} />
            </span>
          </span>
        </button>

        {#if pickerOpen}
          <div class="cap-menu" role="menu">
            <button class="cap-opt" class:on={!selectedMonitor} role="menuitem" onclick={backToApp}>
              <span class="opt-ico"><Icon name="app" size={16} /></span>
              <span class="opt-text">
                <span class="opt-title">Aplicación</span>
                <span class="opt-sub">{game || 'Sin juego'}</span>
              </span>
              {#if !selectedMonitor}<span class="opt-check"><Icon name="check" size={15} sw={2.2} /></span>{/if}
            </button>

            <button
              class="cap-opt mic-opt"
              class:on={micOn}
              role="menuitemcheckbox"
              aria-checked={micOn}
              onclick={(e) => {
                e.stopPropagation();
                micOn = !micOn;
              }}
            >
              <span class="opt-ico"><Icon name="mic" size={16} /></span>
              <span class="mic-label">
                Capturar audio del micrófono
                <span class="help" aria-label="Qué hace esta opción">
                  ?
                  <span class="help-tip" role="tooltip">
                    Si está activo, graba también tu voz desde el micrófono elegido, mezclada con
                    el audio del clip.
                  </span>
                </span>
              </span>
              <span class="mic-switch"><span class="mic-knob"></span></span>
            </button>

            <div class="mic-input">
              <select
                class="mic-select"
                bind:value={micInput}
                onclick={(e) => e.stopPropagation()}
                aria-label="Entrada de micrófono"
              >
                {#each audioInputs as inp (inp.id)}
                  <option value={inp.id}>{inp.name}</option>
                {/each}
                {#if audioInputs.length === 0}
                  <option value="" disabled selected>Sin micrófonos detectados</option>
                {/if}
              </select>
            </div>

            <div class="cap-sep"></div>
            <span class="cap-group">Pantallas</span>

            <div class="screen-grid">
              {#each monitors as m (m.id)}
                <button
                  class="screen-card"
                  class:on={selectedMonitor === m.id}
                  role="menuitem"
                  onclick={() => selectMonitor(m.id)}
                >
                  <span class="screen-thumb" style:background-image={m.thumb ? `url(${m.thumb})` : 'none'}>
                    {#if !m.thumb}<Icon name="monitor" size={22} />{/if}
                    {#if selectedMonitor === m.id}
                      <span class="screen-check"><Icon name="check" size={14} sw={2.4} /></span>
                    {/if}
                  </span>
                </button>
              {/each}
            </div>
          </div>
        {/if}
      </div>

      <div class="quick">
        {#if recording && cap?.running}
          <span class="pill mono live">{fps.toFixed(0)} fps · {cap.width}×{cap.height}</span>
        {/if}
        <span class="pill mono">{session.buffer}</span>
        <span class="pill mono">{session.quality}</span>
        <span class="pill mono">{session.res}</span>
        <button class="pill mono">{session.fps} FPS <Icon name="chevron-down" size={12} sw={2} /></button>
        <span class="hotkey mono"><kbd>Alt</kbd><kbd>F9</kbd> grabar</span>
        <button class="gear" aria-label="Ajustes de captura"><Icon name="settings" size={17} /></button>
      </div>

      <div class="winctl"><WindowControls /></div>
    </header>

    <div class="content">
      {@render children()}
    </div>
  </div>
</div>

<style>
  .app {
    position: relative;
    z-index: 1;
    display: flex;
    height: 100vh;
  }

  .sidebar {
    width: var(--sidebar-w);
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 0 0 14px;
    background: var(--bg-1);
  }
  .logo {
    display: grid;
    place-items: center;
    width: 100%;
    height: var(--topbar-h);
    margin-bottom: 8px;
    border-bottom: 1px solid var(--line);
    transition: background 0.16s ease;
  }
  .logo:hover {
    background: var(--bg-2);
  }
  .logo img {
    display: block;
    width: 30px;
    height: 30px;
  }

  nav {
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 100%;
    align-items: center;
  }
  .nav-item {
    width: 64px;
    height: 52px;
    display: grid;
    place-items: center;
    border-radius: var(--r-md);
    color: var(--text-2);
    position: relative;
    transition: color 0.16s ease, background 0.16s ease;
  }
  .nav-item:hover {
    color: var(--text-1);
    background: var(--bg-2);
  }
  .nav-item.active {
    color: var(--accent);
    background: var(--bg-2);
  }
  .nav-item.active::before {
    content: '';
    position: absolute;
    left: -12px;
    top: 50%;
    transform: translateY(-50%);
    width: 3px;
    height: 22px;
    border-radius: 0 3px 3px 0;
    background: var(--accent);
    box-shadow: 0 0 12px var(--accent-glow);
  }
  .settings-tab {
    margin-top: auto;
  }

  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .topbar {
    height: var(--topbar-h);
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 0 18px;
    background: var(--bg-1);
    border-bottom: 1px solid var(--line);
  }

  .capture-target {
    position: relative;
    align-self: stretch;
    display: flex;
  }
  .capturing {
    position: relative;
    align-self: stretch;
    display: flex;
    align-items: center;
    min-width: 330px;
    padding: 0 16px;
    overflow: hidden;
    text-align: left;
    border-radius: var(--r-sm);
  }
  .cap-icon {
    position: relative;
    z-index: 2;
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    margin-right: 11px;
    border-radius: var(--r-sm);
    color: var(--accent);
    background: var(--bg-3);
    flex-shrink: 0;
  }
  .capturing.rec .cap-icon {
    background: color-mix(in srgb, var(--rec) 16%, var(--bg-3));
  }
  .rec-dot {
    width: 11px;
    height: 11px;
    border-radius: 999px;
    background: var(--rec);
    box-shadow: 0 0 0 0 var(--rec-glow);
    animation: rec-pulse 1.4s ease-out infinite;
  }
  @keyframes rec-pulse {
    0% {
      box-shadow: 0 0 0 0 var(--rec-glow);
    }
    70% {
      box-shadow: 0 0 0 7px transparent;
    }
    100% {
      box-shadow: 0 0 0 0 transparent;
    }
  }
  .cap-frame {
    position: absolute;
    inset: 0;
    z-index: 0;
    background-size: cover;
    background-position: center;
    filter: blur(1px);
    -webkit-mask-image: linear-gradient(to right, transparent 0%, #000 14%, #000 72%, transparent 100%);
    mask-image: linear-gradient(to right, transparent 0%, #000 14%, #000 72%, transparent 100%);
  }
  .capturing::after {
    content: '';
    position: absolute;
    inset: 0;
    z-index: 1;
    background: linear-gradient(90deg, var(--bg-1) 0%, rgba(33, 36, 46, 0.55) 42%, rgba(33, 36, 46, 0) 72%);
  }
  .cap-text {
    position: relative;
    z-index: 2;
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .cap-label {
    font-size: 10.5px;
    line-height: 1;
    color: var(--text-2);
  }
  .cap-proc {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 14px;
    font-weight: 600;
    line-height: 1.2;
    color: var(--text-0);
  }
  .capturing.idle .cap-proc {
    color: var(--text-2);
    font-weight: 500;
  }
  .cap-proc :global(svg) {
    color: var(--text-2);
    transition: transform 0.18s ease;
  }
  .capturing.open .cap-proc :global(svg) {
    transform: rotate(180deg);
  }

  .cap-menu {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    right: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 6px;
    background: var(--bg-1);
    border: 1px solid var(--line-strong);
    border-radius: var(--r-md);
    box-shadow: 0 18px 42px -14px rgba(0, 0, 0, 0.7);
    z-index: 50;
  }
  .cap-opt {
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 9px 10px;
    border-radius: 7px;
    color: var(--text-1);
    text-align: left;
    transition: background 0.12s ease, color 0.12s ease;
  }
  .cap-opt:hover {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .cap-opt.on {
    color: var(--text-0);
  }
  .opt-ico {
    display: grid;
    place-items: center;
    width: 22px;
    color: var(--text-2);
    flex-shrink: 0;
  }
  .cap-opt.on .opt-ico {
    color: var(--accent);
  }
  .screen-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
    padding: 2px;
  }
  .screen-card {
    display: flex;
    padding: 3px;
    border-radius: 8px;
    transition: background 0.12s ease;
  }
  .screen-card:hover {
    background: color-mix(in srgb, var(--accent) 80%, var(--bg-1));
  }
  .screen-thumb {
    position: relative;
    display: grid;
    place-items: center;
    width: 100%;
    aspect-ratio: 16 / 9;
    border-radius: 6px;
    background-color: var(--bg-3);
    background-size: cover;
    background-position: center;
    color: var(--text-3);
    border: 2px solid var(--line);
    overflow: hidden;
  }
  .screen-card.on .screen-thumb {
    border-color: var(--accent);
  }
  .screen-check {
    position: absolute;
    top: 6px;
    right: 6px;
    display: grid;
    place-items: center;
    width: 23px;
    height: 23px;
    border-radius: 999px;
    color: var(--on-accent);
    background: var(--accent);
    box-shadow: 0 2px 7px rgba(0, 0, 0, 0.35);
  }
  .opt-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .opt-title {
    font-size: 13px;
    line-height: 1.1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .opt-sub {
    font-size: 11px;
    line-height: 1;
    color: var(--text-3);
  }
  .opt-check {
    display: grid;
    place-items: center;
    color: var(--accent);
    flex-shrink: 0;
  }
  .cap-sep {
    height: 1px;
    margin: 5px 6px;
    background: var(--line);
  }
  .cap-group {
    padding: 2px 10px 4px;
    font-family: var(--font-mono);
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    text-align: center;
    color: var(--text-1);
  }

  .mic-label {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    font-size: 13px;
    line-height: 1.15;
  }
  .help {
    position: relative;
    display: inline-grid;
    place-items: center;
    width: 15px;
    height: 15px;
    margin-left: 7px;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 700;
    color: var(--text-2);
    background: var(--bg-3);
    cursor: help;
    flex-shrink: 0;
  }
  .help:hover {
    color: var(--text-0);
    background: var(--accent-deep);
  }
  .help-tip {
    position: absolute;
    top: calc(100% + 7px);
    left: 50%;
    transform: translateX(-50%);
    width: 214px;
    padding: 8px 10px;
    font-size: 11.5px;
    font-weight: 400;
    line-height: 1.35;
    color: var(--text-1);
    background: var(--bg-0);
    border: 1px solid var(--line-strong);
    border-radius: 8px;
    box-shadow: 0 12px 30px -10px rgba(0, 0, 0, 0.7);
    opacity: 0;
    visibility: hidden;
    pointer-events: none;
    z-index: 60;
  }
  .help:hover .help-tip {
    opacity: 1;
    visibility: visible;
  }

  .mic-input {
    margin-top: 2px;
    padding: 0 8px 2px;
  }
  .mic-select {
    width: 100%;
    padding: 7px 9px;
    font-size: 12px;
    color: var(--text-1);
    background: var(--bg-2);
    border: 1px solid var(--line);
    border-radius: 6px;
    cursor: pointer;
    transition: border-color 0.14s ease;
  }
  .mic-select:hover {
    border-color: var(--line-strong);
  }
  .mic-select:focus {
    outline: none;
    border-color: var(--accent);
  }

  .mic-switch {
    flex-shrink: 0;
    width: 36px;
    height: 20px;
    border-radius: 999px;
    background: var(--bg-3);
    border: 1px solid var(--line);
    padding: 2px;
    transition: background 0.18s ease, border-color 0.18s ease;
  }
  .mic-knob {
    display: block;
    width: 14px;
    height: 14px;
    border-radius: 999px;
    background: var(--text-2);
    transition: transform 0.18s ease, background 0.18s ease;
  }
  .mic-opt.on .mic-switch {
    background: var(--accent-deep);
    border-color: transparent;
  }
  .mic-opt.on .mic-knob {
    transform: translateX(16px);
    background: var(--accent);
  }

  .quick {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 6px 10px;
    font-size: 11.5px;
    color: var(--text-1);
    background: var(--bg-2);
    border: 1px solid var(--line);
    border-radius: var(--r-sm);
    white-space: nowrap;
  }
  button.pill:hover {
    color: var(--text-0);
    border-color: var(--line-strong);
  }
  .pill.live {
    color: var(--rec);
    border-color: color-mix(in srgb, var(--rec) 40%, transparent);
    background: color-mix(in srgb, var(--rec) 12%, var(--bg-2));
  }
  .hotkey {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    margin-left: 4px;
    font-size: 11px;
    color: var(--text-2);
  }
  kbd {
    font-family: var(--font-mono);
    font-size: 10.5px;
    padding: 2px 6px;
    color: var(--text-1);
    background: var(--bg-3);
    border: 1px solid var(--line);
    border-bottom-width: 2px;
    border-radius: 5px;
  }
  .gear {
    width: 34px;
    height: 34px;
    display: grid;
    place-items: center;
    border-radius: var(--r-sm);
    color: var(--text-1);
    transition: background 0.14s ease, color 0.14s ease;
  }
  .gear:hover {
    background: var(--bg-2);
    color: var(--text-0);
  }

  .winctl {
    align-self: stretch;
    display: flex;
    margin-right: -18px;
  }

  .content {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    border-left: 1px solid var(--line);
  }
</style>
