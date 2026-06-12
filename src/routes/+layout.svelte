<script lang="ts">
  import '@fontsource-variable/geist';
  import '../app.css';
  import { page } from '$app/state';
  import Icon from '$lib/components/Icon.svelte';

  let { children } = $props();

  const nav = [
    { href: '/', icon: 'clips', label: 'Clips' },
    { href: '/favoritos', icon: 'bookmark', label: 'Favoritos' },
    { href: '/settings', icon: 'settings', label: 'Ajustes' }
  ];

  const isActive = (href: string) =>
    href === '/' ? page.url.pathname === '/' : page.url.pathname.startsWith(href);

  let autoClip = $state(false);
  const session = { process: 'Minecraft', buffer: '01:00', quality: 'Alto', res: '1080p', fps: '60' };
</script>

<div class="app">
  <aside class="sidebar">
    <a class="logo" href="/" aria-label="Flashback">
      <svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M11 5 4.5 12 11 19" stroke="#8b93a6" />
        <path d="M19 5 12.5 12 19 19" stroke="var(--accent)" />
      </svg>
    </a>

    <nav>
      {#each nav as item (item.href)}
        <a class="nav-item" class:active={isActive(item.href)} href={item.href}>
          <span class="nav-icon"><Icon name={item.icon} size={20} /></span>
          <span class="nav-label">{item.label}</span>
        </a>
      {/each}
    </nav>

    <div class="encoder" title="Encoder por hardware activo">
      <Icon name="bolt" size={13} />
      <span class="mono">NVENC</span>
    </div>
  </aside>

  <div class="main">
    <header class="topbar">
      <div class="capturing">
        <span class="dot"></span>
        <span class="cap-label">CAPTURANDO</span>
        <button class="proc mono">
          {session.process}
          <Icon name="chevron-down" size={13} sw={2} />
        </button>
      </div>

      <button class="autoclip" class:on={autoClip} onclick={() => (autoClip = !autoClip)}>
        <span class="ac-text">
          Clip automático
          <span class="ac-state mono" class:off={!autoClip}>{autoClip ? 'ACTIVO' : 'OFF'}</span>
        </span>
        <span class="switch"><span class="knob"></span></span>
      </button>

      <div class="quick">
        <span class="pill mono">{session.buffer}</span>
        <span class="pill mono">{session.quality}</span>
        <span class="pill mono">{session.res}</span>
        <button class="pill mono">{session.fps} FPS <Icon name="chevron-down" size={12} sw={2} /></button>
        <span class="hotkey mono"><kbd>Alt</kbd><kbd>`</kbd> clip</span>
        <button class="gear" aria-label="Ajustes de captura"><Icon name="settings" size={17} /></button>
      </div>
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
    padding: 16px 0 14px;
    background: var(--bg-1);
    border-right: 1px solid var(--line);
  }
  .logo {
    display: grid;
    place-items: center;
    width: 46px;
    height: 46px;
    border-radius: var(--r-md);
    margin-bottom: 14px;
    transition: background 0.16s ease;
  }
  .logo:hover {
    background: var(--bg-2);
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
    padding: 10px 0 8px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
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
  .nav-label {
    font-family: var(--font-mono);
    font-size: 10px;
    letter-spacing: 0.06em;
  }

  .encoder {
    margin-top: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    color: var(--accent-soft);
  }
  .encoder .mono {
    font-size: 9.5px;
    letter-spacing: 0.1em;
    color: var(--text-2);
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

  .capturing {
    display: flex;
    align-items: center;
    gap: 9px;
    min-width: 0;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 999px;
    background: var(--accent);
    box-shadow: 0 0 0 4px rgba(63, 109, 245, 0.16);
    animation: breathe 2s ease-in-out infinite;
  }
  @keyframes breathe {
    0%, 100% { opacity: 0.45; box-shadow: 0 0 0 2px rgba(63, 109, 245, 0.1); }
    50% { opacity: 1; box-shadow: 0 0 0 5px rgba(63, 109, 245, 0.22); }
  }
  .cap-label {
    font-family: var(--font-mono);
    font-size: 11px;
    letter-spacing: 0.14em;
    color: var(--accent);
  }
  .proc {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 10px;
    font-size: 12px;
    color: var(--text-1);
    background: var(--bg-2);
    border: 1px solid var(--line);
    border-radius: var(--r-sm);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .proc:hover {
    color: var(--text-0);
    border-color: var(--line-strong);
  }

  .autoclip {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 8px 6px 12px;
    border-radius: var(--r-sm);
    color: var(--text-1);
    transition: background 0.14s ease;
  }
  .autoclip:hover {
    background: var(--bg-2);
  }
  .ac-text {
    font-size: 13px;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .ac-state {
    font-size: 10px;
    letter-spacing: 0.1em;
    color: var(--accent);
  }
  .ac-state.off {
    color: var(--text-3);
  }
  .switch {
    width: 38px;
    height: 21px;
    border-radius: 999px;
    background: var(--bg-3);
    border: 1px solid var(--line);
    padding: 2px;
    transition: background 0.18s ease;
  }
  .knob {
    display: block;
    width: 15px;
    height: 15px;
    border-radius: 999px;
    background: var(--text-2);
    transition: transform 0.18s ease, background 0.18s ease;
  }
  .autoclip.on .switch {
    background: var(--accent-deep);
    border-color: transparent;
  }
  .autoclip.on .knob {
    transform: translateX(17px);
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

  .content {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
  }
</style>
