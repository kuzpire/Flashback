<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import ClipCard from '$lib/components/ClipCard.svelte';
  import { clips, groupClips } from '$lib/clips';

  let query = $state('');
  let view = $state<'grid' | 'list'>('grid');

  const filtered = $derived(
    clips.filter((c) => {
      const q = query.trim().toLowerCase();
      return !q || c.title.toLowerCase().includes(q) || c.source.toLowerCase().includes(q);
    })
  );
  const groups = $derived(groupClips(filtered));
</script>

<div class="clips">
  <header class="head">
    <div class="left">
      <h1>Todos los clips</h1>
      <span class="count mono">{filtered.length}</span>
      <button class="montage">
        <Icon name="plus" size={15} sw={2} />
        Crear montaje
      </button>
    </div>

    <div class="right">
      <label class="search">
        <Icon name="search" size={15} />
        <input placeholder="Buscar clips" bind:value={query} />
      </label>
      <button class="ctrl">
        <Icon name="filter" size={14} />
        Filtro
      </button>
      <button class="ctrl">
        Más reciente
        <Icon name="chevron-down" size={13} sw={2} />
      </button>
      <div class="viewtoggle">
        <button class:on={view === 'grid'} aria-label="Cuadrícula" onclick={() => (view = 'grid')}>
          <Icon name="clips" size={16} />
        </button>
        <button class:on={view === 'list'} aria-label="Lista" onclick={() => (view = 'list')}>
          <Icon name="list" size={16} />
        </button>
      </div>
    </div>
  </header>

  {#if filtered.length === 0}
    <div class="empty">
      <Icon name="chevrons" size={56} sw={1.2} />
      <p>Sin resultados para “{query}”.</p>
    </div>
  {:else}
    {#each groups as group (group.label)}
      <section class="group">
        <div class="group-head">
          <span class="label">{group.label}</span>
          <span class="dash"></span>
          <span class="label src">{group.source}</span>
        </div>
        <div class="grid" class:list={view === 'list'}>
          {#each group.clips as clip (clip.id)}
            <ClipCard {clip} />
          {/each}
        </div>
      </section>
    {/each}
  {/if}
</div>

<style>
  .clips {
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
  .count {
    font-size: 12px;
    padding: 3px 9px;
    color: var(--accent);
    background: rgba(63, 109, 245, 0.08);
    border: 1px solid rgba(63, 109, 245, 0.18);
    border-radius: 999px;
  }
  .montage {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    margin-left: 4px;
    padding: 8px 14px;
    font-size: 13px;
    font-weight: 560;
    color: var(--accent);
    background: rgba(63, 109, 245, 0.07);
    border: 1px solid rgba(63, 109, 245, 0.22);
    border-radius: var(--r-sm);
    transition: background 0.15s ease, box-shadow 0.15s ease;
  }
  .montage:hover {
    background: rgba(63, 109, 245, 0.13);
    box-shadow: 0 0 0 3px rgba(63, 109, 245, 0.08);
  }

  .right {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 12px;
    height: 36px;
    width: 220px;
    color: var(--text-2);
    background: var(--bg-1);
    border: 1px solid var(--line);
    border-radius: var(--r-sm);
    transition: border-color 0.15s ease;
  }
  .search:focus-within {
    border-color: var(--line-strong);
  }
  .search input {
    flex: 1;
    min-width: 0;
    background: none;
    border: none;
    outline: none;
    font-size: 13px;
    color: var(--text-0);
  }
  .search input::placeholder {
    color: var(--text-3);
  }
  .ctrl {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 36px;
    padding: 0 12px;
    font-size: 13px;
    color: var(--text-1);
    background: var(--bg-1);
    border: 1px solid var(--line);
    border-radius: var(--r-sm);
    transition: color 0.15s ease, border-color 0.15s ease;
  }
  .ctrl:hover {
    color: var(--text-0);
    border-color: var(--line-strong);
  }
  .viewtoggle {
    display: flex;
    padding: 3px;
    gap: 2px;
    background: var(--bg-1);
    border: 1px solid var(--line);
    border-radius: var(--r-sm);
  }
  .viewtoggle button {
    width: 30px;
    height: 28px;
    display: grid;
    place-items: center;
    border-radius: 5px;
    color: var(--text-2);
    transition: color 0.14s ease, background 0.14s ease;
  }
  .viewtoggle button:hover {
    color: var(--text-1);
  }
  .viewtoggle button.on {
    color: var(--accent);
    background: var(--bg-3);
  }

  .group {
    margin-bottom: 30px;
  }
  .group-head {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 14px;
  }
  .src {
    color: var(--text-3);
  }
  .dash {
    flex: 1;
    height: 1px;
    background: var(--line);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 20px;
  }
  @media (min-width: 1280px) {
    .grid:not(.list) {
      grid-template-columns: repeat(3, 1fr);
    }
  }
  .grid.list {
    grid-template-columns: minmax(0, 760px);
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 14px;
    padding: 90px 0;
    color: var(--text-3);
  }
  .empty p {
    font-size: 14px;
    color: var(--text-2);
  }
</style>
