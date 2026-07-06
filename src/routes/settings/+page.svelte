<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import Icon from '$lib/components/Icon.svelte';
  import Dropdown from '$lib/components/Dropdown.svelte';
  import { t, getLocale, setLocale, LOCALES, type Locale } from '$lib/i18n.svelte';
  import { refreshLibrary } from '$lib/library.svelte';
  import {
    hotkeys,
    capture,
    setHotkey,
    labelFor,
    comboFromEvent,
    hasMainKey,
    eventHasUnsupportedKey,
    type HotkeyAction
  } from '$lib/hotkeys.svelte';

  import { replay, setReplayEnabled, setReplaySeconds, BUFFER_OPTIONS } from '$lib/replay.svelte';
  import {
    replaySound,
    setReplaySoundLevel,
    playReplaySound,
    soundLabel,
    SOUND_OPTIONS
  } from '$lib/replay-sound.svelte';
  import {
    captureConfig,
    setFps,
    setQuality,
    setResolution,
    qualityLabel,
    FPS_OPTIONS,
    QUALITY_OPTIONS,
    RES_OPTIONS
  } from '$lib/capture-config.svelte';

  const ENCODER_OPTIONS = ['Auto', 'NVENC', 'AMF', 'Quick Sync', 'Software'] as const;
  type EncoderOption = typeof ENCODER_OPTIONS[number];

  const resOptions = RES_OPTIONS.map((o) => ({ label: o.label, value: o.height }));
  const fpsOptions = FPS_OPTIONS.map((o) => ({ label: `${o} fps`, value: o }));
  const qualityOptions = $derived(QUALITY_OPTIONS.map((o) => ({ label: qualityLabel(o.key), value: o.key })));
  const encoderOptions = ENCODER_OPTIONS.map((o) => ({ label: o, value: o }));
  const bufferOptions = BUFFER_OPTIONS.map((o) => ({ label: o.label, value: o.seconds }));
  const soundOptions = $derived(SOUND_OPTIONS.map((o) => ({ label: soundLabel(o.key), value: o.key })));
  const languageOptions = LOCALES.map((l) => ({ label: l.label, value: l.value }));

  let encoder = $state<EncoderOption>('Auto');
  let autoDelete = $state(true);
  let discordRpc = $state(false);

  let folder = $state('');
  let changingFolder = $state(false);
  $effect(() => {
    invoke<string>('clips_dir').then((d) => (folder = d)).catch(() => {});
    invoke<string>('get_encoder').then((e) => {
      if (ENCODER_OPTIONS.includes(e as EncoderOption)) encoder = e as EncoderOption;
    }).catch(() => {});
    invoke<boolean>('get_discord_rpc').then((v) => (discordRpc = v)).catch(() => {});
  });

  function setDiscordRpc(on: boolean) {
    discordRpc = on;
    invoke('set_discord_rpc', { enabled: on }).catch(() => {});
  }

  function setEncoder(opt: EncoderOption) {
    encoder = opt;
    invoke('set_encoder', { enc: opt }).catch(() => {});
  }

  async function changeFolder() {
    if (changingFolder) return;
    changingFolder = true;
    try {
      const picked = await invoke<string | null>('pick_folder');
      if (picked) {
        await invoke('set_clips_dir', { dir: picked });
        folder = picked;
        // La biblioteca lee de la carpeta activa: recargar para reflejar el cambio.
        await refreshLibrary();
      }
    } catch (e) {
      console.error('set_clips_dir', e);
    } finally {
      changingFolder = false;
    }
  }

  const shortcutRows: { key: HotkeyAction; labelKey: string }[] = [
    { key: 'saveReplay', labelKey: 'settings.hk.saveReplay' },
    { key: 'record', labelKey: 'settings.hk.record' },
    { key: 'open', labelKey: 'settings.hk.open' }
  ];

  let rebinding = $state<HotkeyAction | null>(null);
  let liveTokens = $state<string[]>([]);
  let badKey = $state(false);
  let canSave = $derived(liveTokens.length > 0 && hasMainKey(liveTokens));

  function onKeyDown(e: KeyboardEvent) {
    if (!rebinding) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.code === 'Escape') {
      endCapture(false);
      return;
    }
    const combo = comboFromEvent(e);
    if (combo.length && hasMainKey(combo)) {
      liveTokens = combo;
      badKey = false;
    } else if (eventHasUnsupportedKey(e)) {
      badKey = true;
    }
  }

  function startCapture(action: HotkeyAction) {
    rebinding = action;
    liveTokens = [];
    badKey = false;
    // Soltar los atajos globales mientras se escucha, o el SO se traga la combinación.
    capture.active = true;
    window.addEventListener('keydown', onKeyDown, true);
  }

  function endCapture(save: boolean) {
    window.removeEventListener('keydown', onKeyDown, true);
    if (save && rebinding && liveTokens.length && hasMainKey(liveTokens)) {
      setHotkey(rebinding, liveTokens.join('+'));
    }
    rebinding = null;
    liveTokens = [];
    badKey = false;
    capture.active = false;
  }

  // Tocar el atajo inicia la captura; se guarda con el botón ✓ y ESC cancela. Tocar
  // otra fila cancela la reasignación en curso sin guardar.
  function startRebind(action: HotkeyAction) {
    if (rebinding === action) return;
    if (rebinding) endCapture(false);
    startCapture(action);
  }

  $effect(() => {
    return () => endCapture(false);
  });
</script>

<div class="settings">
  <header><h1>{t('settings.title')}</h1></header>

  <section class="panel">
    <span class="label panel-title">{t('settings.section.language')}</span>
    <div class="setting">
      <div class="info"><h3>{t('settings.language')}</h3><p>{t('settings.language.desc')}</p></div>
      <Dropdown value={getLocale()} options={languageOptions} onchange={(v) => setLocale(v as Locale)} ariaLabel={t('settings.language')} />
    </div>
  </section>

  <section class="panel">
    <span class="label panel-title">{t('settings.section.capture')}</span>

    <div class="setting">
      <div class="info">
        <h3>{t('settings.resolution')}</h3>
        <p>{t('settings.resolution.desc')}</p>
      </div>
      <Dropdown value={captureConfig.resolution} options={resOptions} onchange={setResolution} ariaLabel={t('settings.resolution')} />
    </div>

    <div class="setting">
      <div class="info"><h3>{t('settings.fps')}</h3><p>{t('settings.fps.desc')}</p></div>
      <Dropdown value={captureConfig.fps} options={fpsOptions} onchange={setFps} ariaLabel={t('settings.fps')} />
    </div>

    <div class="setting">
      <div class="info"><h3>{t('settings.quality')}</h3><p>{t('settings.quality.desc')}</p></div>
      <Dropdown value={captureConfig.quality} options={qualityOptions} onchange={setQuality} ariaLabel={t('settings.quality')} />
    </div>

    <div class="setting">
      <div class="info"><h3>{t('settings.replayBg')}</h3><p>{t('settings.replayBg.desc')}</p></div>
      <button class="switch" class:on={replay.enabled} onclick={() => setReplayEnabled(!replay.enabled)} role="switch" aria-checked={replay.enabled} aria-label={t('settings.replayBg')}>
        <span class="knob"></span>
      </button>
    </div>

    <div class="setting" class:disabled={!replay.enabled}>
      <div class="info"><h3>{t('settings.bufferLen')}</h3><p>{t('settings.bufferLen.desc')}</p></div>
      <Dropdown value={replay.seconds} options={bufferOptions} onchange={setReplaySeconds} ariaLabel={t('settings.bufferLen')} />
    </div>
  </section>

  <section class="panel">
    <span class="label panel-title">{t('settings.section.encoding')}</span>
    <div class="setting">
      <div class="info">
        <h3>{t('settings.encoder')}</h3>
        <p>{t('settings.encoder.desc')}</p>
      </div>
      <Dropdown value={encoder} options={encoderOptions} onchange={setEncoder} ariaLabel={t('settings.encoder')} />
    </div>
  </section>

  <section class="panel">
    <span class="label panel-title">{t('settings.section.sound')}</span>
    <div class="setting">
      <div class="info"><h3>{t('settings.saveSound')}</h3><p>{t('settings.saveSound.desc')}</p></div>
      <div class="sound-row">
        <Dropdown value={replaySound.level} options={soundOptions} onchange={setReplaySoundLevel} ariaLabel={t('settings.soundVolume')} />
        <button class="play-btn" aria-label={t('settings.testSound')} onclick={() => playReplaySound()}>
          <Icon name="play" size={18} />
        </button>
      </div>
    </div>
  </section>

  <section class="panel">
    <span class="label panel-title">{t('settings.section.storage')}</span>
    <div class="setting">
      <div class="info"><h3>{t('settings.clipsFolder')}</h3><p class="mono path">{folder}</p><p>{t('settings.clipsFolder.desc')}</p></div>
      <button class="btn" onclick={changeFolder} disabled={changingFolder}><Icon name="folder" size={15} /> {t('settings.change')}</button>
    </div>
    <div class="setting">
      <div class="info"><h3>{t('settings.autoDelete')}</h3><p>{t('settings.autoDelete.desc')}</p></div>
      <button class="switch" class:on={autoDelete} onclick={() => (autoDelete = !autoDelete)} role="switch" aria-checked={autoDelete} aria-label={t('settings.autoDelete')}>
        <span class="knob"></span>
      </button>
    </div>
  </section>

  <section class="panel" id="atajos">
    <span class="label panel-title">{t('settings.section.shortcuts')}</span>
    <p class="hk-hint">{@html t('settings.shortcuts.hint')}</p>
    {#each shortcutRows as row (row.key)}
      <div class="setting">
        <div class="info"><h3>{t(row.labelKey)}</h3></div>
        <div class="hk-edit">
          <button
            type="button"
            class="combo mono"
            class:rec={rebinding === row.key}
            class:bad={rebinding === row.key && badKey && liveTokens.length === 0}
            onclick={() => startRebind(row.key)}
            aria-label={t('settings.hk.changeAria', { label: t(row.labelKey) })}
          >
            {#if rebinding === row.key}
              {#if liveTokens.length}
                {labelFor(liveTokens.join('+'))}
              {:else if badKey}
                {t('settings.hk.badKey')}
              {:else}
                {t('settings.hk.pressKey')}
              {/if}
            {:else}
              {labelFor(hotkeys[row.key])}
            {/if}
          </button>
          {#if rebinding === row.key}
            <button
              type="button"
              class="combo-save"
              disabled={!canSave}
              onclick={() => endCapture(true)}
              aria-label={t('settings.hk.saveAria')}
            >
              <Icon name="check" size={12} sw={3} />
            </button>
          {/if}
        </div>
      </div>
    {/each}
  </section>

  <section class="panel">
    <span class="label panel-title">{t('settings.section.integrations')}</span>
    <div class="setting">
      <div class="info">
        <h3>{t('settings.discordRpc')}</h3>
        <p>{t('settings.discordRpc.desc')}</p>
      </div>
      <button class="switch" class:on={discordRpc} onclick={() => setDiscordRpc(!discordRpc)} role="switch" aria-checked={discordRpc} aria-label={t('settings.discordRpc')}>
        <span class="knob"></span>
      </button>
    </div>
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
    background: var(--on-accent);
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

  .hk-hint {
    font-size: 12px;
    color: var(--text-2);
    padding: 2px 0 10px;
  }
  .hk-hint :global(strong) {
    color: var(--text-1);
    font-weight: 600;
  }

  .hk-edit {
    position: relative;
    display: flex;
    align-items: center;
    flex-shrink: 0;
  }
  .combo {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 132px;
    height: 34px;
    padding: 0 14px;
    font-size: 12px;
    letter-spacing: 0.04em;
    color: var(--text-1);
    background: var(--bg-0);
    border: 1px solid var(--line);
    border-radius: var(--r-sm);
    transition: background 0.15s ease, color 0.15s ease, border-color 0.15s ease;
  }
  .combo:hover {
    color: var(--text-0);
    border-color: var(--line-strong);
  }
  .combo.rec {
    color: var(--accent);
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, var(--bg-0));
  }
  .combo.bad {
    color: var(--rec);
    border-color: var(--rec);
    background: color-mix(in srgb, var(--rec) 12%, var(--bg-0));
  }
  .combo-save {
    position: absolute;
    top: -8px;
    right: -8px;
    z-index: 5;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    color: #000;
    background: #fff;
    border-radius: 999px;
    box-shadow: 0 3px 10px -2px rgba(0, 0, 0, 0.6);
    transition: transform 0.12s ease, opacity 0.12s ease;
  }
  .combo-save:hover:not(:disabled) {
    transform: scale(1.08);
  }
  .combo-save:active:not(:disabled) {
    transform: scale(0.94);
  }
  .combo-save:disabled {
    cursor: default;
  }

  .sound-row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-shrink: 0;
  }
  .play-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    width: 44px;
    height: 34px;
    color: #000;
    background: var(--accent);
    border: 1px solid transparent;
    border-radius: var(--r-sm);
    transition: background 0.15s ease, transform 0.1s ease;
  }
  .play-btn:hover {
    background: var(--accent-deep);
  }
  .play-btn:active {
    transform: scale(0.96);
  }
</style>
