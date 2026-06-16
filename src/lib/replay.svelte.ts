const ENABLED_KEY = 'flashback.replay.enabled';
const SECONDS_KEY = 'flashback.replay.seconds';

export const BUFFER_OPTIONS: { label: string; seconds: number }[] = [
  { label: '30 s', seconds: 30 },
  { label: '1 min', seconds: 60 },
  { label: '3 min', seconds: 180 },
  { label: '5 min', seconds: 300 }
];

function loadEnabled(): boolean {
  if (typeof localStorage === 'undefined') return false;
  return localStorage.getItem(ENABLED_KEY) === '1';
}

function loadSeconds(): number {
  if (typeof localStorage === 'undefined') return 60;
  const n = Number(localStorage.getItem(SECONDS_KEY));
  return BUFFER_OPTIONS.some((o) => o.seconds === n) ? n : 60;
}

export const replay = $state<{ enabled: boolean; seconds: number }>({
  enabled: loadEnabled(),
  seconds: loadSeconds()
});

export function setReplayEnabled(v: boolean) {
  replay.enabled = v;
  if (typeof localStorage !== 'undefined') {
    try {
      localStorage.setItem(ENABLED_KEY, v ? '1' : '0');
    } catch {
      // sin persistencia disponible
    }
  }
}

export function setReplaySeconds(s: number) {
  replay.seconds = s;
  if (typeof localStorage !== 'undefined') {
    try {
      localStorage.setItem(SECONDS_KEY, String(s));
    } catch {
      // sin persistencia disponible
    }
  }
}
