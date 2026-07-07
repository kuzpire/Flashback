# Auto-updater Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add signed auto-updates to Flashback with a non-intrusive centered modal and a notification dot on the app icon.

**Architecture:** Uses the official Tauri 2 `updater` plugin (+ `process` to relaunch). The frontend runs `check()` ~4s after mount; a Svelte store holds update state; the layout renders the dot on the sidebar logo and a centered modal. A `release.sh` script builds with signing enabled and publishes `latest.json` to the GitHub release.

**Tech Stack:** Tauri 2, Rust, Svelte 5 (runes), TypeScript, Bash + Node for the release script, `gh` CLI.

## Global Constraints

- Communication/UI copy for user-facing strings goes through i18n (`en` + `es`), keys identical in both dicts.
- No decorative comments; only comment non-obvious *why*.
- Commits in English; never add Claude as author/co-author.
- Windows x64 only; do not add cross-platform branches.
- Do not touch the capture/encode hot path. Update check/download is JS/IPC async only.
- Private signing key and its password live under `src-tauri/.tauri/` and MUST be gitignored.
- Repo owner/name for URLs: `kuzpire/Flashback`.

---

### Task 1: Rust plugins + capabilities

**Files:**
- Modify: `src-tauri/Cargo.toml` (dependencies section, after line 24)
- Modify: `src-tauri/src/lib.rs:400` (plugin chain)
- Modify: `src-tauri/capabilities/default.json` (permissions array)

**Interfaces:**
- Produces: the `updater` and `process` Tauri plugins available to the JS side (`@tauri-apps/plugin-updater` `check()`, `@tauri-apps/plugin-process` `relaunch()`).

- [ ] **Step 1: Add Rust dependencies**

In `src-tauri/Cargo.toml`, after the line `tauri-plugin-single-instance = "2"`, add:

```toml
tauri-plugin-updater = "2"
tauri-plugin-process = "2"
```

- [ ] **Step 2: Register the plugins**

In `src-tauri/src/lib.rs`, find (around line 400):

```rust
    builder
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
```

Replace with:

```rust
    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
```

- [ ] **Step 3: Add capability permissions**

In `src-tauri/capabilities/default.json`, in the `permissions` array, after `"opener:allow-reveal-item-in-dir",` add:

```json
    "updater:default",
    "process:allow-restart",
```

- [ ] **Step 4: Verify it compiles**

Run: `cd src-tauri && cargo check`
Expected: finishes with no errors (warnings ok). The two new crates download and compile.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/lib.rs src-tauri/capabilities/default.json
git commit -m "Register updater and process plugins"
```

---

### Task 2: Signing keys + updater config

**Files:**
- Create: `src-tauri/.tauri/flashback.key` (gitignored, generated)
- Create: `src-tauri/.tauri/flashback.key.pub` (generated, holds pubkey)
- Create: `src-tauri/.tauri/flashback.key.pass` (gitignored, generated)
- Modify: `.gitignore`
- Modify: `src-tauri/tauri.conf.json` (bundle + new plugins block)

**Interfaces:**
- Produces: `bundle.createUpdaterArtifacts` (build emits `-setup.exe.sig`), `plugins.updater.pubkey` and `plugins.updater.endpoints` (runtime knows where `latest.json` lives). Consumed by Task 6's release script (reads the key + pass files).

- [ ] **Step 1: Gitignore the key dir first**

In `.gitignore`, after the line `/src-tauri/gen/`, add:

```
/src-tauri/.tauri/
```

- [ ] **Step 2: Generate a 32-char password and the keypair**

Run (Git Bash):

```bash
mkdir -p src-tauri/.tauri
PW=$(openssl rand -base64 24)
printf '%s' "$PW" > src-tauri/.tauri/flashback.key.pass
pnpm tauri signer generate -w src-tauri/.tauri/flashback.key -p "$PW" --ci --force
```

Expected: prints a public key and writes `flashback.key` + `flashback.key.pub`.

- [ ] **Step 3: Confirm the key is NOT tracked by git**

Run: `git status --porcelain src-tauri/.tauri`
Expected: **no output** (the whole dir is ignored). If any `.tauri` path shows up, fix `.gitignore` before continuing.

- [ ] **Step 4: Read the public key**

Run: `cat src-tauri/.tauri/flashback.key.pub`
Expected: one base64 line (this is the value for the next step).

- [ ] **Step 5: Add updater config to `tauri.conf.json`**

In `src-tauri/tauri.conf.json`, change the `bundle` object opening from:

```json
  "bundle": {
    "active": true,
    "targets": "all",
```

to:

```json
  "bundle": {
    "active": true,
    "targets": "all",
    "createUpdaterArtifacts": true,
```

Then add a `plugins` block after the closing of `bundle` (i.e. change the final `}` of the file). Find:

```json
    "windows": {
      "nsis": {
        "installerHooks": "installer.nsh"
      },
      "wix": {
        "fragmentPaths": ["autostart.wxs"],
        "componentRefs": ["FlashbackAutostart"]
      }
    }
  }
}
```

Replace with (paste the pubkey from Step 4 in place of `PASTE_PUBKEY_HERE`):

```json
    "windows": {
      "nsis": {
        "installerHooks": "installer.nsh"
      },
      "wix": {
        "fragmentPaths": ["autostart.wxs"],
        "componentRefs": ["FlashbackAutostart"]
      }
    }
  },
  "plugins": {
    "updater": {
      "pubkey": "PASTE_PUBKEY_HERE",
      "endpoints": [
        "https://github.com/kuzpire/Flashback/releases/latest/download/latest.json"
      ]
    }
  }
}
```

- [ ] **Step 6: Verify the config parses**

Run: `node -e "JSON.parse(require('fs').readFileSync('src-tauri/tauri.conf.json','utf8')); console.log('ok')"`
Expected: prints `ok`.

- [ ] **Step 7: Commit (config only — keys stay local)**

```bash
git add .gitignore src-tauri/tauri.conf.json
git commit -m "Configure updater endpoint, pubkey and updater artifacts"
```

---

### Task 3: Frontend updater store

**Files:**
- Create: `src/lib/updater.svelte.ts`
- Modify: `package.json` (dependencies)

**Interfaces:**
- Consumes: `check()` from `@tauri-apps/plugin-updater`, `relaunch()` from `@tauri-apps/plugin-process`, `invoke('stop_replay')` / `invoke('stop_capture')` (existing Rust commands).
- Produces (imported by Task 5):
  - `updater` — reactive object `{ available: boolean; info: { version: string; notes: string } | null; popupOpen: boolean; installing: boolean; progress: number }`
  - `checkForUpdate(): Promise<void>`
  - `maybeAutoShow(): Promise<void>`
  - `openUpdatePopup(): void`
  - `closeUpdatePopup(): void`
  - `installUpdate(): Promise<void>`

- [ ] **Step 1: Add JS dependencies**

Run: `pnpm add @tauri-apps/plugin-updater @tauri-apps/plugin-process`
Expected: both appear under `dependencies` in `package.json`.

- [ ] **Step 2: Write the store**

Create `src/lib/updater.svelte.ts`:

```ts
import { check, type Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';

type UpdateInfo = { version: string; notes: string };

export const updater = $state<{
  available: boolean;
  info: UpdateInfo | null;
  popupOpen: boolean;
  installing: boolean;
  progress: number;
}>({ available: false, info: null, popupOpen: false, installing: false, progress: 0 });

let pending: Update | null = null;
// Solo se auto-muestra el modal la primera vez que la ventana está visible; después
// del "Cancelar" solo se reabre desde la bolita del logo.
let autoShown = false;

export async function checkForUpdate() {
  try {
    const u = await check();
    if (!u) return;
    pending = u;
    updater.info = { version: u.version, notes: u.body ?? '' };
    updater.available = true;
    await maybeAutoShow();
  } catch (e) {
    // Fuera de Tauri (preview en navegador) o sin red: se ignora en silencio.
    console.error('update check', e);
  }
}

export async function maybeAutoShow() {
  if (!updater.available || autoShown || updater.popupOpen) return;
  try {
    if (await getCurrentWindow().isVisible()) {
      autoShown = true;
      updater.popupOpen = true;
    }
  } catch {
    // preview navegador
  }
}

export function openUpdatePopup() {
  if (updater.available) updater.popupOpen = true;
}

export function closeUpdatePopup() {
  if (updater.installing) return;
  updater.popupOpen = false;
}

export async function installUpdate() {
  if (!pending || updater.installing) return;
  updater.installing = true;
  updater.progress = 0;
  // El instalador cierra la app: se detiene la captura para no dejar el pipeline a medias.
  await invoke('stop_replay').catch(() => {});
  await invoke('stop_capture').catch(() => {});
  try {
    let total = 0;
    let got = 0;
    await pending.downloadAndInstall((e) => {
      if (e.event === 'Started') total = e.data.contentLength ?? 0;
      else if (e.event === 'Progress') {
        got += e.data.chunkLength;
        updater.progress = total ? Math.round((got / total) * 100) : 0;
      } else if (e.event === 'Finished') updater.progress = 100;
    });
    await relaunch();
  } catch (e) {
    console.error('install update', e);
    updater.installing = false;
  }
}
```

- [ ] **Step 3: Verify types**

Run: `pnpm check`
Expected: 0 errors. (Warnings pre-existing elsewhere are fine; the new file must contribute none.)

- [ ] **Step 4: Commit**

```bash
git add package.json pnpm-lock.yaml src/lib/updater.svelte.ts
git commit -m "Add updater store"
```

---

### Task 4: i18n strings

**Files:**
- Modify: `src/lib/i18n.svelte.ts` (end of `en` dict ~line 266, end of `es` dict ~line 473)

**Interfaces:**
- Produces keys used in Task 5: `upd.title`, `upd.version` (param `{v}`), `upd.update`, `upd.cancel`, `upd.installing`, `upd.badgeLabel`.

- [ ] **Step 1: Add English keys**

In `src/lib/i18n.svelte.ts`, find the end of the `en` dict:

```ts
  'time.yearAgo': '1 year ago',
  'time.yearsAgo': '{n} years ago'
};
```

Replace with:

```ts
  'time.yearAgo': '1 year ago',
  'time.yearsAgo': '{n} years ago',
  'upd.title': 'Update available',
  'upd.version': 'Version {v} is ready to install.',
  'upd.update': 'Update',
  'upd.cancel': 'Cancel',
  'upd.installing': 'Downloading and installing…',
  'upd.badgeLabel': 'Update available'
};
```

- [ ] **Step 2: Add Spanish keys**

Find the end of the `es` dict:

```ts
  'time.yearAgo': 'Hace 1 año',
  'time.yearsAgo': 'Hace {n} años'
};
```

Replace with:

```ts
  'time.yearAgo': 'Hace 1 año',
  'time.yearsAgo': 'Hace {n} años',
  'upd.title': 'Actualización disponible',
  'upd.version': 'La versión {v} está lista para instalar.',
  'upd.update': 'Actualizar',
  'upd.cancel': 'Cancelar',
  'upd.installing': 'Descargando e instalando…',
  'upd.badgeLabel': 'Actualización disponible'
};
```

- [ ] **Step 3: Verify both dicts have the same new keys**

Run: `grep -c "upd\." src/lib/i18n.svelte.ts`
Expected: `12` (6 keys × 2 dicts).

- [ ] **Step 4: Verify types**

Run: `pnpm check`
Expected: 0 errors.

- [ ] **Step 5: Commit**

```bash
git add src/lib/i18n.svelte.ts
git commit -m "Add updater i18n strings"
```

---

### Task 5: UI — notification dot + modal

**Files:**
- Modify: `src/routes/+layout.svelte` (imports, mount effect, `.logo` markup, modal markup, `<style>`)

**Interfaces:**
- Consumes from Task 3: `updater`, `checkForUpdate`, `maybeAutoShow`, `openUpdatePopup`, `closeUpdatePopup`, `installUpdate`.
- Consumes from Task 4: the `upd.*` i18n keys.

- [ ] **Step 1: Import the store**

In `src/routes/+layout.svelte`, after the line `import { t, initLocale } from '$lib/i18n.svelte';` add:

```ts
  import {
    updater,
    checkForUpdate,
    maybeAutoShow,
    openUpdatePopup,
    closeUpdatePopup,
    installUpdate
  } from '$lib/updater.svelte';
```

- [ ] **Step 2: Add the check-on-mount effect**

In the `<script>`, immediately before the last closing of the script logic (after the instant-replay `$effect` that ends at line ~431, before `</script>`), add:

```ts
  // Chequeo de actualización ~4s tras montar. El popup solo se auto-muestra si la ventana
  // está visible (arranque en bandeja: solo bolita, y al enfocar la ventana se muestra).
  $effect(() => {
    const timer = setTimeout(checkForUpdate, 4000);
    const unlisten = getCurrentWindow().onFocusChanged(({ payload }) => {
      if (payload) maybeAutoShow();
    });
    return () => {
      clearTimeout(timer);
      unlisten.then((u) => u());
    };
  });
```

(`getCurrentWindow` is already imported at line 10.)

- [ ] **Step 3: Make the logo show the dot**

Replace the `.logo` block:

```svelte
    <div class="logo" data-tauri-drag-region>
      <img src="/flashback-mono.svg" alt="Flashback" />
    </div>
```

with:

```svelte
    <div class="logo" data-tauri-drag-region>
      {#if updater.available}
        <button
          class="logo-btn"
          aria-label={t('upd.badgeLabel')}
          onclick={(e) => {
            e.stopPropagation();
            openUpdatePopup();
          }}
        >
          <img src="/flashback-mono.svg" alt="Flashback" />
          <span class="upd-dot"></span>
        </button>
      {:else}
        <img src="/flashback-mono.svg" alt="Flashback" />
      {/if}
    </div>
```

- [ ] **Step 4: Add the modal markup**

At the very end of the markup, after the editor block:

```svelte
{#if editorState.clip}
  {#key editorState.clip.id}
    <Editor />
  {/key}
{/if}
```

add:

```svelte
{#if updater.popupOpen && updater.info}
  <div class="upd-overlay" role="presentation" onclick={closeUpdatePopup}>
    <div
      class="upd-modal"
      role="dialog"
      aria-modal="true"
      onclick={(e) => e.stopPropagation()}
    >
      <h2 class="upd-title">{t('upd.title')}</h2>
      <p class="upd-ver">{t('upd.version', { v: updater.info.version })}</p>
      {#if updater.info.notes}<p class="upd-notes">{updater.info.notes}</p>{/if}
      {#if updater.installing}
        <div class="upd-track"><div class="upd-fill" style:width={`${updater.progress}%`}></div></div>
        <p class="upd-status">{t('upd.installing')}</p>
      {:else}
        <div class="upd-actions">
          <button class="upd-btn ghost" onclick={closeUpdatePopup}>{t('upd.cancel')}</button>
          <button class="upd-btn primary" onclick={installUpdate}>{t('upd.update')}</button>
        </div>
      {/if}
    </div>
  </div>
{/if}
```

- [ ] **Step 5: Add styles**

At the end of the `<style>` block (before the closing `</style>`), add:

```css
  .logo-btn {
    position: relative;
    display: grid;
    place-items: center;
    background: none;
    border: 0;
    padding: 0;
    cursor: pointer;
  }
  .upd-dot {
    position: absolute;
    top: -2px;
    right: -2px;
    width: 9px;
    height: 9px;
    border-radius: 999px;
    background: var(--accent);
    box-shadow: 0 0 0 2px #080808;
  }

  .upd-overlay {
    position: fixed;
    inset: 0;
    z-index: 200;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.6);
  }
  .upd-modal {
    width: 380px;
    max-width: calc(100vw - 40px);
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 22px;
    background: var(--bg-1);
    border: 1px solid var(--line-strong);
    border-radius: var(--r-md);
    box-shadow: 0 24px 60px -18px rgba(0, 0, 0, 0.8);
  }
  .upd-title {
    font-size: 17px;
    font-weight: 640;
    color: var(--text-0);
  }
  .upd-ver {
    font-size: 13px;
    color: var(--text-1);
  }
  .upd-notes {
    max-height: 160px;
    overflow-y: auto;
    font-size: 12.5px;
    line-height: 1.4;
    color: var(--text-2);
    white-space: pre-wrap;
  }
  .upd-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
  }
  .upd-btn {
    padding: 9px 16px;
    font-size: 13px;
    border-radius: var(--r-sm);
    cursor: pointer;
    transition: background 0.15s ease, color 0.15s ease, border-color 0.15s ease;
  }
  .upd-btn.ghost {
    color: var(--text-1);
    background: transparent;
    border: 1px solid var(--line);
  }
  .upd-btn.ghost:hover {
    color: var(--text-0);
    border-color: var(--line-strong);
  }
  .upd-btn.primary {
    color: var(--on-accent);
    background: var(--accent);
    border: 1px solid transparent;
    font-weight: 560;
  }
  .upd-btn.primary:hover {
    background: var(--accent-deep);
  }
  .upd-track {
    height: 7px;
    margin-top: 6px;
    border-radius: 999px;
    background: var(--bg-3);
    overflow: hidden;
  }
  .upd-fill {
    height: 100%;
    background: var(--accent);
    transition: width 0.15s ease;
  }
  .upd-status {
    font-size: 12px;
    color: var(--text-2);
  }
```

- [ ] **Step 6: Verify types**

Run: `pnpm check`
Expected: 0 errors.

- [ ] **Step 7: Manual smoke (UI only)**

Run: `pnpm tauri dev`
Then temporarily force the modal by editing the store's `updater` initial `available`/`popupOpen`/`info` in devtools console is not possible; instead verify visually that:
- App launches, no dot, no modal (no update published yet → `check()` returns null).
- No console errors from `update check` other than expected network/no-update.
Stop the dev app. (Full end-to-end update is verified in Task 6 once a release exists.)

- [ ] **Step 8: Commit**

```bash
git add src/routes/+layout.svelte
git commit -m "Add update notification dot and modal"
```

---

### Task 6: Release script + latest.json

**Files:**
- Create: `scripts/gen-latest-json.mjs`
- Create: `scripts/release.sh`

**Interfaces:**
- Consumes: the key files from Task 2, the built artifacts from `pnpm tauri build`, `gh` CLI.
- `gen-latest-json.mjs` CLI: `node scripts/gen-latest-json.mjs <version> <sigPath>` → prints the `latest.json` manifest to stdout (reads `RELEASE_NOTES` env for notes).

- [ ] **Step 1: Write the failing test for the manifest generator**

Create `scripts/gen-latest-json.test.mjs`:

```js
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import assert from 'node:assert/strict';

const dir = mkdtempSync(join(tmpdir(), 'flb-'));
const sig = join(dir, 'setup.sig');
writeFileSync(sig, 'SIGNATURE_CONTENT\n');

const out = execFileSync('node', ['scripts/gen-latest-json.mjs', '1.4.0', sig], {
  env: { ...process.env, RELEASE_NOTES: 'Hello notes' },
  encoding: 'utf8'
});
const m = JSON.parse(out);

assert.equal(m.version, '1.4.0');
assert.equal(m.notes, 'Hello notes');
assert.equal(m.platforms['windows-x86_64'].signature, 'SIGNATURE_CONTENT');
assert.equal(
  m.platforms['windows-x86_64'].url,
  'https://github.com/kuzpire/Flashback/releases/download/v1.4.0/Flashback_1.4.0_x64-setup.exe'
);
console.log('gen-latest-json: ok');
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `node scripts/gen-latest-json.test.mjs`
Expected: FAIL — `Cannot find module '.../scripts/gen-latest-json.mjs'`.

- [ ] **Step 3: Write the generator**

Create `scripts/gen-latest-json.mjs`:

```js
import { readFileSync } from 'node:fs';

const [version, sigPath] = process.argv.slice(2);
if (!version || !sigPath) {
  console.error('usage: gen-latest-json.mjs <version> <sigPath>');
  process.exit(1);
}

const signature = readFileSync(sigPath, 'utf8').trim();
const url = `https://github.com/kuzpire/Flashback/releases/download/v${version}/Flashback_${version}_x64-setup.exe`;

const manifest = {
  version,
  notes: process.env.RELEASE_NOTES ?? '',
  pub_date: new Date().toISOString(),
  platforms: {
    'windows-x86_64': { signature, url }
  }
};

process.stdout.write(JSON.stringify(manifest, null, 2));
```

- [ ] **Step 4: Run the test to confirm it passes**

Run: `node scripts/gen-latest-json.test.mjs`
Expected: prints `gen-latest-json: ok`.

- [ ] **Step 5: Write the release script**

Create `scripts/release.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

NOTES_FILE="${1:-}"

VERSION=$(node -p "require('./package.json').version")
TAG="v${VERSION}"

KEY_FILE="src-tauri/.tauri/flashback.key"
PASS_FILE="src-tauri/.tauri/flashback.key.pass"
[ -f "$KEY_FILE" ] || { echo "missing $KEY_FILE"; exit 1; }
[ -f "$PASS_FILE" ] || { echo "missing $PASS_FILE"; exit 1; }

export TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY_FILE")"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(cat "$PASS_FILE")"

pnpm tauri build

SETUP="src-tauri/target/release/bundle/nsis/Flashback_${VERSION}_x64-setup.exe"
MSI="src-tauri/target/release/bundle/msi/Flashback_${VERSION}_x64_en-US.msi"
SIG="${SETUP}.sig"
[ -f "$SIG" ] || { echo "missing signature $SIG (createUpdaterArtifacts?)"; exit 1; }

if [ -n "$NOTES_FILE" ] && [ -f "$NOTES_FILE" ]; then
  export RELEASE_NOTES="$(cat "$NOTES_FILE")"
fi

node scripts/gen-latest-json.mjs "$VERSION" "$SIG" > latest.json

if [ -n "$NOTES_FILE" ] && [ -f "$NOTES_FILE" ]; then
  gh release create "$TAG" --title "Flashback ${VERSION}" --notes-file "$NOTES_FILE" \
    "$SETUP" "$MSI" latest.json
else
  gh release create "$TAG" --title "Flashback ${VERSION}" --generate-notes \
    "$SETUP" "$MSI" latest.json
fi

echo "Published $TAG"
```

- [ ] **Step 6: Make it executable and sanity-check syntax**

Run: `chmod +x scripts/release.sh && bash -n scripts/release.sh && echo ok`
Expected: prints `ok`.

- [ ] **Step 7: Commit**

```bash
git add scripts/gen-latest-json.mjs scripts/gen-latest-json.test.mjs scripts/release.sh
git commit -m "Add signed release script and latest.json generator"
```

- [ ] **Step 8: End-to-end publish (real update path)**

This is the integration verification for the whole feature. Bump the version (patch) in `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` (e.g. to `1.4.0`), commit `Bump version to 1.4.0`, then:

Run: `git push origin main && bash scripts/release.sh`
Expected: build succeeds, `latest.json` uploaded, release `v1.4.0` created with `.exe`, `.msi`, `latest.json`.

Then install the **previous** version and confirm: ~4s after opening, the modal appears; "Cancelar" closes it and leaves the dot; clicking the logo reopens it; "Actualizar" downloads, installs and relaunches into the new version.

---

## Notes for the implementer

- Run each task's verification before committing; never commit red.
- The private key and password files must never be staged (Task 2 Step 3 guards this).
- `--accent`, `--accent-deep`, `--on-accent`, `--bg-1/3`, `--line/line-strong`, `--text-0..3`, `--r-sm/md` are existing CSS variables in `src/app.css`; reuse them, don't hardcode colors.
