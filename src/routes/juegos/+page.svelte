<script lang="ts">
  // Datos de ejemplo: aún no hay backend de lista de juegos (detect_game solo devuelve
  // el juego activo). Pendiente: detección del sistema o DB local de juegos registrados.
  let capturing = $state({
    name: 'Minecraft',
    initials: 'MC',
    note: 'Capturando tus mejores jugadas.. ¿o no?',
    on: true
  });

  let detected = $state([
    {
      name: 'Counter-Strike 2',
      initials: 'CS',
      lastSeen: 'Última vez hace 18 horas',
      exe: 'C:\\SteamLibrary\\steamapps\\common\\Counter-Strike Global Offensive\\game\\bin\\win64\\cs2.exe',
      on: true
    },
    {
      name: 'League of Legends',
      initials: 'LoL',
      lastSeen: 'Última vez hace 2 días',
      exe: 'C:\\Riot Games\\League of Legends\\Game\\League of Legends.exe',
      on: false
    }
  ]);
</script>

<div class="games">
  <header class="head">
    <div class="left">
      <h1>Juegos detectados</h1>
    </div>
    <div class="right">
      <span class="add-game-prompt">¿No ves tu juego? <button class="add-game-link">¡Añádelo!</button></span>
    </div>
  </header>

  <section class="game-group">
    <div class="game-group-head">
      <span class="label">Capturando</span>
      <span class="dash"></span>
    </div>
    <div class="game-list">
      <div class="game-row running">
        <span class="game-ico mono">{capturing.initials}</span>
        <div class="game-info">
          <span class="game-name">{capturing.name}</span>
          <span class="game-path mono">{capturing.note}</span>
        </div>
        <div class="cap-toggle">
          <span class="cap-toggle-label">Capturar</span>
          <button
            class="switch"
            class:on={capturing.on}
            onclick={() => (capturing.on = !capturing.on)}
            role="switch"
            aria-checked={capturing.on}
            aria-label={`Capturar ${capturing.name}`}
          >
            <span class="knob"></span>
          </button>
        </div>
      </div>
    </div>
  </section>

  <section class="game-group">
    <div class="game-group-head">
      <span class="label">Detectados en el sistema</span>
      <span class="dash"></span>
    </div>
    <div class="game-list">
      {#each detected as g (g.name)}
        <div class="game-row">
          <span class="game-ico mono">{g.initials}</span>
          <div class="game-info">
            <span class="game-name">{g.name}</span>
            <span class="game-sub">
              <span class="game-path mono sub-lastseen">{g.lastSeen}</span>
              <span class="game-path mono sub-exe">{g.exe}</span>
            </span>
          </div>
          <div class="cap-toggle">
            <span class="cap-toggle-label">Capturar</span>
            <button
              class="switch"
              class:on={g.on}
              onclick={() => (g.on = !g.on)}
              role="switch"
              aria-checked={g.on}
              aria-label={`Capturar ${g.name}`}
            >
              <span class="knob"></span>
            </button>
          </div>
        </div>
      {/each}
    </div>
  </section>
</div>

<style>
  .games {
    padding: 22px 26px 40px;
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 18px;
    margin-bottom: 26px;
    flex-wrap: wrap;
  }
  .left {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  h1 {
    font-size: 22px;
    font-weight: 650;
    letter-spacing: -0.01em;
  }
  .add-game-prompt {
    font-size: 13px;
    color: var(--text-2);
  }
  .add-game-link {
    font-size: 13px;
    font-weight: 600;
    color: var(--accent-soft);
    padding: 0;
    transition: color 0.15s ease;
  }
  .add-game-link:hover {
    color: var(--accent);
    text-decoration: underline;
    text-underline-offset: 3px;
  }

  .game-group {
    margin-bottom: 28px;
  }
  .game-group-head {
    display: flex;
    align-items: center;
    gap: 11px;
    margin-bottom: 13px;
  }
  .dash {
    flex: 1;
    height: 1px;
    background: var(--line);
  }

  .game-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .game-row {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 13px 16px;
    background: var(--bg-1);
    border: 1px solid var(--line);
    border-radius: var(--r-md);
    transition: border-color 0.15s ease, background 0.15s ease;
  }
  .game-row:hover {
    border-color: var(--line-strong);
  }
  .game-row.running {
    background: var(--bg-2);
  }
  .game-ico {
    display: grid;
    place-items: center;
    width: 46px;
    height: 46px;
    flex-shrink: 0;
    font-size: 14px;
    font-weight: 600;
    letter-spacing: 0.02em;
    color: var(--text-1);
    background: var(--bg-3);
    border: 1px solid var(--line);
    border-radius: 10px;
  }
  .game-row.running .game-ico {
    color: var(--text-0);
  }
  .game-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .game-name {
    font-size: 14.5px;
    font-weight: 560;
    color: var(--text-0);
    line-height: 1.1;
  }
  .game-path {
    font-size: 11.5px;
    color: var(--text-3);
    line-height: 1.2;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Cross-fade: "última vez" sube y se desvanece; la ruta entra desde abajo al hacer hover. */
  .game-sub {
    position: relative;
    display: block;
    height: 1.2em;
    overflow: hidden;
  }
  .game-sub .game-path {
    position: absolute;
    left: 0;
    top: 0;
    width: 100%;
    transition: transform 0.34s cubic-bezier(0.4, 0, 0.2, 1), opacity 0.28s ease;
  }
  .game-sub .sub-lastseen {
    transform: translateY(0);
    opacity: 1;
  }
  .game-sub .sub-exe {
    transform: translateY(105%);
    opacity: 0;
    color: var(--text-2);
  }
  .game-row:hover .game-sub .sub-lastseen {
    transform: translateY(-105%);
    opacity: 0;
  }
  .game-row:hover .game-sub .sub-exe {
    transform: translateY(0);
    opacity: 1;
  }
  @media (prefers-reduced-motion: reduce) {
    .game-sub .game-path {
      transition: none;
    }
  }

  .cap-toggle {
    display: flex;
    align-items: center;
    gap: 9px;
    flex-shrink: 0;
    padding-left: 16px;
    margin-left: 2px;
    border-left: 1px solid var(--line);
  }
  .cap-toggle-label {
    font-size: 11px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-2);
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
    background: var(--bright);
    border-color: transparent;
  }
  .switch.on .knob {
    transform: translateX(19px);
    background: var(--bg-1);
  }
</style>
