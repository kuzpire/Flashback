<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import ClipCard from '$lib/components/ClipCard.svelte';
  import { clips, groupClips } from '$lib/clips';

  const favorites = clips.filter((c) => c.favorite);
  const groups = groupClips(favorites);
</script>

<div class="favs">
  <header class="head">
    <span class="ico"><Icon name="bookmark" size={17} /></span>
    <h1>Favoritos</h1>
    <span class="count mono">{favorites.length}</span>
  </header>

  {#if favorites.length === 0}
    <div class="empty">
      <Icon name="bookmark" size={50} sw={1.3} />
      <p>Aún no tienes clips guardados como favoritos.</p>
      <span class="hint mono">Marca la estrella de un clip para guardarlo aquí.</span>
    </div>
  {:else}
    {#each groups as group (group.label)}
      <section class="group">
        <div class="group-head">
          <span class="label">{group.label}</span>
          <span class="dash"></span>
          <span class="label src">{group.source}</span>
        </div>
        <div class="grid">
          {#each group.clips as clip (clip.id)}
            <ClipCard {clip} />
          {/each}
        </div>
      </section>
    {/each}
  {/if}
</div>

<style>
  .favs {
    padding: 22px 26px 40px;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 13px;
    margin-bottom: 26px;
  }
  .ico {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border-radius: var(--r-sm);
    color: var(--accent);
    background: rgba(63, 109, 245, 0.10);
    border: 1px solid rgba(63, 109, 245, 0.20);
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
    .grid {
      grid-template-columns: repeat(3, 1fr);
    }
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    padding: 90px 0;
    color: var(--text-3);
  }
  .empty p {
    font-size: 14px;
    color: var(--text-1);
  }
  .hint {
    font-size: 11.5px;
    color: var(--text-3);
  }
</style>
