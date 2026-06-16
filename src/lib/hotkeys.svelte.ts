export type HotkeyAction = 'saveReplay' | 'record' | 'open';

const STORAGE_KEY = 'flashback.hotkeys';

const defaults: Record<HotkeyAction, string> = {
  saveReplay: 'Alt+F8',
  record: 'Alt+F9',
  open: 'Alt+F10'
};

function load(): Record<HotkeyAction, string> {
  if (typeof localStorage === 'undefined') return { ...defaults };
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return { ...defaults, ...JSON.parse(raw) };
  } catch {
    // localStorage corrupto o bloqueado
  }
  return { ...defaults };
}

export const hotkeys = $state<Record<HotkeyAction, string>>(load());

// Mientras se reasigna un atajo en Ajustes hay que soltar los atajos globales: si no,
// el SO se traga la combinación (RegisterHotKey la intercepta) y nunca llega al capturador.
export const capture = $state({ active: false });

export function setHotkey(action: HotkeyAction, accel: string) {
  hotkeys[action] = accel;
  if (typeof localStorage !== 'undefined') {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(hotkeys));
    } catch {
      // sin persistencia disponible
    }
  }
}

const MODS = ['Control', 'Alt', 'Shift', 'Super'];
const modLabel: Record<string, string> = {
  Control: 'CTRL',
  Alt: 'ALT',
  Shift: 'SHIFT',
  Super: 'WIN'
};

export function isModifier(token: string): boolean {
  return MODS.includes(token);
}

export function hasMainKey(tokens: string[]): boolean {
  return tokens.some((t) => !isModifier(t));
}

// Tokens canónicos (los que entiende el plugin al unir con '+') → etiquetas para la UI.
export function labelTokens(accel: string): string[] {
  return accel.split('+').map((t) => modLabel[t] ?? t.toUpperCase());
}

export function labelFor(accel: string): string {
  return labelTokens(accel).join(' + ');
}

const CODE_KEYS: Record<string, string> = {
  Backquote: '`',
  Minus: '-',
  Equal: '=',
  BracketLeft: '[',
  BracketRight: ']',
  Backslash: '\\',
  Semicolon: ';',
  Quote: "'",
  Comma: ',',
  Period: '.',
  Slash: '/',
  Space: 'Space',
  Enter: 'Enter',
  Tab: 'Tab',
  Escape: 'Escape',
  Backspace: 'Backspace',
  Delete: 'Delete',
  ArrowUp: 'Up',
  ArrowDown: 'Down',
  ArrowLeft: 'Left',
  ArrowRight: 'Right'
};

function isModifierCode(code: string): boolean {
  return /^(Control|Shift|Alt|Meta)(Left|Right)$/.test(code);
}

function codeToToken(code: string): string | null {
  let m: RegExpMatchArray | null;
  if ((m = code.match(/^Key([A-Z])$/))) return m[1];
  if ((m = code.match(/^Digit(\d)$/))) return m[1];
  if ((m = code.match(/^Numpad(\d)$/))) return m[1];
  if (/^F\d{1,2}$/.test(code)) return code;
  return CODE_KEYS[code] ?? null;
}

// Combinación a partir de un evento de teclado, máximo 2 tokens (1 modificador + 1
// tecla, o una sola tecla). Devuelve [] si solo hay modificadores sin tecla principal.
export function comboFromEvent(e: KeyboardEvent): string[] {
  const mods: string[] = [];
  if (e.ctrlKey) mods.push('Control');
  if (e.altKey) mods.push('Alt');
  if (e.shiftKey) mods.push('Shift');
  if (e.metaKey) mods.push('Super');

  const main = isModifierCode(e.code) ? null : codeToToken(e.code);
  if (main) return mods.length ? [mods[0], main] : [main];
  return mods.slice(0, 2);
}
