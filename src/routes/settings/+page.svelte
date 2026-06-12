<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';

  let res = $state('1080p');
  let fps = $state('60');
  let quality = $state('Alto');
  let encoder = $state('Automático');
  let buffer = $state('1 min');
  let replayOn = $state(true);
  let autoDelete = $state(true);

  const folder = 'C:\\Users\\joshiny\\Videos\\Flashback';

  const hotkeys = [
    { action: 'Guardar replay', keys: ['Alt', '`'] },
    { action: 'Grabar / detener', keys: ['Alt', 'F9'] },
    { action: 'Abrir Flashback', keys: ['Alt', 'F10'] }
  ];
</script>

<div class="settings">
  <header><h1>Ajustes</h1></header>

  <section class="panel">
    <span class="label panel-title">Captura</span>

    <div class="setting">
      <div class="info"><h3>Resolución</h3><p>Resolución de salida de los clips.</p></div>
      <div class="seg">
        {#each ['720p', '1080p', '1440p', '2160p'] as o (o)}
          <button class:on={res === o} onclick={() => (res = o)}>{o}</button>
        {/each}
      </div>
    </div>

    <div class="setting">
      <div class="info"><h3>Frecuencia</h3><p>Fotogramas por segundo.</p></div>
      <div class="seg">
        {#each ['30', '60', '120', '144'] as o (o)}
          <button class:on={fps === o} onclick={() => (fps = o)}>{o}</button>
        {/each}
      </div>
    </div>

    <div class="setting">
      <div class="info"><h3>Calidad</h3><p>Más calidad = archivos más pesados.</p></div>
      <div class="seg">
        {#each ['Bajo', 'Medio', 'Alto', 'Ultra'] as o (o)}
          <button class:on={quality === o} onclick={() => (quality = o)}>{o}</button>
        {/each}
      </div>
    </div>
  </section>

  <section class="panel">
    <span class="label panel-title">Codificación</span>
    <div class="setting">
      <div class="info">
        <h3>Encoder</h3>
        <p>Automático elige el mejor por hardware disponible. <span class="hw mono">NVENC detectado</span></p>
      </div>
      <div class="seg">
        {#each ['Automático', 'NVENC', 'AMF', 'Quick Sync', 'Software'] as o (o)}
          <button class:on={encoder === o} onclick={() => (encoder = o)}>{o}</button>
        {/each}
      </div>
    </div>
  </section>

  <section class="panel">
    <span class="label panel-title">Instant Replay</span>

    <div class="setting">
      <div class="info"><h3>Replay en segundo plano</h3><p>Mantén un buffer listo para guardar.</p></div>
      <button class="switch" class:on={replayOn} onclick={() => (replayOn = !replayOn)} role="switch" aria-checked={replayOn} aria-label="Replay en segundo plano">
        <span class="knob"></span>
      </button>
    </div>

    <div class="setting" class:disabled={!replayOn}>
      <div class="info"><h3>Duración del buffer</h3><p>Cuántos segundos/minutos se guardan al pulsar el atajo.</p></div>
      <div class="seg">
        {#each ['30 s', '1 min', '3 min', '5 min'] as o (o)}
          <button class:on={buffer === o} onclick={() => (buffer = o)}>{o}</button>
        {/each}
      </div>
    </div>
  </section>

  <section class="panel">
    <span class="label panel-title">Almacenamiento</span>
    <div class="setting">
      <div class="info"><h3>Carpeta de clips</h3><p class="mono path">{folder}</p></div>
      <button class="btn"><Icon name="folder" size={15} /> Cambiar</button>
    </div>
    <div class="setting">
      <div class="info"><h3>Borrado automático</h3><p>Elimina los clips no marcados como favoritos al llenarse.</p></div>
      <button class="switch" class:on={autoDelete} onclick={() => (autoDelete = !autoDelete)} role="switch" aria-checked={autoDelete} aria-label="Borrado automático">
        <span class="knob"></span>
      </button>
    </div>
  </section>

  <section class="panel">
    <span class="label panel-title">Atajos</span>
    {#each hotkeys as h (h.action)}
      <div class="setting hk">
        <h3>{h.action}</h3>
        <span class="keys mono">{#each h.keys as k (k)}<kbd>{k}</kbd>{/each}</span>
      </div>
    {/each}
  </section>
</div>

<style>
  .settings {
    padding: 22px 26px 48px;
    max-width: 860px;
  }
  header {
    margin-bottom: 22px;
  }
  h1 {
    font-size: 22px;
    font-weight: 650;
    letter-spacing: -0.01em;
  }

  .panel {
    background: var(--bg-1);
    border: 1px solid var(--line);
    border-radius: var(--r-lg);
    padding: 8px 20px;
    margin-bottom: 18px;
  }
  .panel-title {
    display: block;
    padding: 14px 0 6px;
    color: var(--accent-soft);
  }

  .setting {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    padding: 16px 0;
    border-top: 1px solid var(--line);
  }
  .panel-title + .setting {
    border-top: none;
  }
  .setting.disabled {
    opacity: 0.45;
    pointer-events: none;
  }
  .info h3 {
    font-size: 14.5px;
    font-weight: 560;
    margin-bottom: 3px;
  }
  .info p {
    font-size: 12.5px;
    color: var(--text-2);
  }
  .path {
    font-size: 11.5px;
    color: var(--text-1);
  }
  .hw {
    color: var(--accent);
    font-size: 11px;
    margin-left: 4px;
  }

  .seg {
    display: flex;
    flex-shrink: 0;
    padding: 3px;
    gap: 2px;
    background: var(--bg-0);
    border: 1px solid var(--line);
    border-radius: var(--r-sm);
  }
  .seg button {
    padding: 7px 13px;
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--text-2);
    border-radius: 5px;
    transition: color 0.14s ease, background 0.14s ease;
  }
  .seg button:hover {
    color: var(--text-0);
  }
  .seg button.on {
    color: var(--on-accent);
    background: var(--accent);
    font-weight: 600;
  }

  .switch {
    flex-shrink: 0;
    width: 44px;
    height: 25px;
    border-radius: 999px;
    background: var(--bg-3);
    border: 1px solid var(--line);
    padding: 2px;
    transition: background 0.18s ease, border-color 0.18s ease;
  }
  .switch .knob {
    display: block;
    width: 19px;
    height: 19px;
    border-radius: 999px;
    background: var(--text-2);
    transition: transform 0.18s ease, background 0.18s ease;
  }
  .switch.on {
    background: var(--accent-deep);
    border-color: transparent;
  }
  .switch.on .knob {
    transform: translateX(19px);
    background: var(--accent);
  }

  .btn {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
    padding: 9px 15px;
    font-size: 13px;
    color: var(--text-1);
    background: var(--bg-2);
    border: 1px solid var(--line-strong);
    border-radius: var(--r-sm);
    transition: background 0.15s ease, color 0.15s ease;
  }
  .btn:hover {
    background: var(--bg-3);
    color: var(--text-0);
  }

  .hk {
    padding: 13px 0;
  }
  .hk h3 {
    font-size: 13.5px;
    font-weight: 500;
  }
  .keys {
    display: flex;
    gap: 5px;
  }
  kbd {
    font-family: var(--font-mono);
    font-size: 11px;
    padding: 3px 8px;
    color: var(--text-1);
    background: var(--bg-3);
    border: 1px solid var(--line);
    border-bottom-width: 2px;
    border-radius: 5px;
  }
</style>
