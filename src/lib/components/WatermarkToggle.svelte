<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import { t } from '$lib/i18n.svelte';

  let enabled = $state(false);
  let corner = $state('br');
  let hovering = $state(false);

  const corners = ['tl', 'tr', 'bl', 'br'];

  onMount(async () => {
    try {
      enabled = await invoke<boolean>('get_watermark');
      corner = await invoke<string>('get_watermark_corner');
    } catch {
      // fuera de Tauri (preview en navegador)
    }
  });

  async function toggle(e: MouseEvent) {
    e.stopPropagation();
    enabled = !enabled;
    try {
      await invoke('set_watermark', { on: enabled });
    } catch {}
  }

  async function pick(e: MouseEvent, c: string) {
    e.stopPropagation();
    corner = c;
    try {
      await invoke('set_watermark_corner', { corner: c });
    } catch {}
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="wm"
  onmouseenter={() => (hovering = true)}
  onmouseleave={() => (hovering = false)}
>
  {#if enabled && hovering}
    <div class="wm-pop">
      <div class="wm-pop-inner" role="radiogroup" aria-label={t('ed.watermarkPos')}>
        {#each corners as c (c)}
          <button
            class="wm-screen"
            class:sel={corner === c}
            role="radio"
            aria-checked={corner === c}
            aria-label={c}
            onclick={(e) => pick(e, c)}
          >
            <span
              class="wm-dot"
              style="{c[0] === 't' ? 'top' : 'bottom'}: 3px; {c[1] === 'l' ? 'left' : 'right'}: 3px;"
            ></span>
          </button>
        {/each}
      </div>
    </div>
  {/if}

  <button
    class="wm-toggle"
    class:on={enabled}
    aria-pressed={enabled}
    aria-label={t('ed.watermark')}
    title={t('ed.watermarkOn')}
    onclick={toggle}
  >
    <span class="wm-logo"></span>
  </button>
</div>

<style>
  .wm {
    position: relative;
    display: inline-flex;
  }

  .wm-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    font-size: 12px;
    color: var(--text-1);
    background: var(--bg-2);
    border: 1px solid var(--line);
    border-radius: var(--r-sm);
    white-space: nowrap;
    transition: background 0.14s ease, color 0.14s ease, border-color 0.14s ease;
  }
  .wm-toggle:hover {
    background: var(--bg-hover);
    color: var(--text-0);
  }
  /* Activo: el botón entero es el interruptor, se colorea de blanco (como Exportar). */
  .wm-toggle.on {
    background: var(--bright);
    color: var(--bg-0);
    border-color: transparent;
    font-weight: 600;
  }
  .wm-toggle.on:hover {
    background: var(--bright);
    color: var(--bg-0);
    opacity: 0.9;
  }
  /* Logo de Flashback (isotipo mono) como máscara: se recolorea con currentColor, así sigue el
     color del botón (claro inactivo, negro cuando está activo sobre el fondo blanco). */
  .wm-logo {
    display: block;
    width: 16px;
    height: 16px;
    background-color: currentColor;
    -webkit-mask: url('/flashback-mono.svg') center / contain no-repeat;
    mask: url('/flashback-mono.svg') center / contain no-repeat;
  }

  /* Popover: 4 mini-pantallas con la esquina resaltada, encima del interruptor.
     .wm-pop es un puente transparente que llega hasta el interruptor (padding), para que al
     mover el ratón hacia las opciones desde un lado no se cruce un hueco sin hover y se cierre. */
  .wm-pop {
    position: absolute;
    bottom: 100%;
    left: 50%;
    transform: translateX(-50%);
    padding: 0 12px 9px;
    z-index: 30;
  }
  .wm-pop-inner {
    position: relative;
    display: grid;
    grid-template-columns: repeat(2, auto);
    gap: 6px;
    padding: 8px;
    background: var(--bg-1);
    border: 1px solid var(--line);
    border-radius: var(--r-sm);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
  }
  .wm-pop-inner::after {
    content: '';
    position: absolute;
    top: 100%;
    left: 50%;
    transform: translateX(-50%);
    border: 6px solid transparent;
    border-top-color: var(--bg-1);
  }
  .wm-screen {
    position: relative;
    width: 44px;
    height: 26px;
    border-radius: 4px;
    background: var(--bg-3);
    border: 1px solid var(--line);
    transition: border-color 0.14s ease, background 0.14s ease;
  }
  .wm-screen:hover {
    background: var(--bg-hover);
  }
  .wm-screen.sel {
    border-color: var(--bright);
    background: var(--bg-hover);
  }
  .wm-dot {
    position: absolute;
    width: 9px;
    height: 6px;
    border-radius: 2px;
    background: var(--text-2);
  }
  .wm-screen.sel .wm-dot {
    background: var(--bright);
  }
</style>
