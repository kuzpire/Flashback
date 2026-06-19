<script lang="ts">
  import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';

  const appWindow = getCurrentWindow();
  let maximized = $state(false);

  $effect(() => {
    appWindow.setMinSize(new LogicalSize(1200, 675));
    appWindow.isMaximized().then((m) => (maximized = m));
    const unlisten = appWindow.onResized(async () => {
      let { width, height } = await appWindow.outerSize();
      if (width < 1200 || height < 675) {
        await appWindow.setSize(new LogicalSize(1200, 675));
      }
      maximized = await appWindow.isMaximized();
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  });
</script>

<div class="controls">
  <button class="ctl" aria-label="Minimizar" onclick={() => appWindow.minimize()}>
    <svg viewBox="0 0 10 10" width="10" height="10"><path d="M1 5h8" /></svg>
  </button>

  <button
    class="ctl"
    aria-label={maximized ? 'Restaurar' : 'Maximizar'}
    onclick={() => appWindow.toggleMaximize()}
  >
    {#if maximized}
      <svg viewBox="0 0 10 10" width="10" height="10">
        <rect x="1" y="2.5" width="5.5" height="5.5" />
        <path d="M3 2.5V1h6v6H7.5" />
      </svg>
    {:else}
      <svg viewBox="0 0 10 10" width="10" height="10"><rect x="1" y="1" width="8" height="8" /></svg>
    {/if}
  </button>

  <button class="ctl close" aria-label="Cerrar" onclick={() => appWindow.close()}>
    <svg viewBox="0 0 10 10" width="10" height="10"><path d="M1 1l8 8M9 1l-8 8" /></svg>
  </button>
</div>

<style>
  .controls {
    display: flex;
    align-self: stretch;
  }
  .ctl {
    width: 46px;
    display: grid;
    place-items: center;
    color: var(--text-2);
    transition: background 0.14s ease, color 0.14s ease;
  }
  .ctl svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 1;
  }
  .ctl:hover {
    background: var(--bg-2);
    color: var(--text-0);
  }
  .close:hover {
    background: var(--rec);
    color: #fff;
  }
</style>
