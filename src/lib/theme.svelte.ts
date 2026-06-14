export type Theme = 'flashback' | 'cursor';
export type AppIcon = 'color' | 'mono';

const THEME_KEY = 'flashback.theme';
const ICON_KEY = 'flashback.icon';

const ICON_SRC: Record<AppIcon, string> = {
  color: '/flashback.svg',
  mono: '/flashback-mono.svg'
};

function loadTheme(): Theme {
  if (typeof localStorage === 'undefined') return 'flashback';
  return localStorage.getItem(THEME_KEY) === 'cursor' ? 'cursor' : 'flashback';
}

function loadIcon(): AppIcon {
  if (typeof localStorage === 'undefined') return 'color';
  return localStorage.getItem(ICON_KEY) === 'mono' ? 'mono' : 'color';
}

function apply(theme: Theme) {
  if (typeof document !== 'undefined') {
    document.documentElement.dataset.theme = theme;
  }
}

export const ui = $state<{ theme: Theme; icon: AppIcon }>({
  theme: loadTheme(),
  icon: loadIcon()
});

// Aplicar ya al cargar el módulo (antes del primer render del layout) para evitar
// el parpadeo de tema.
apply(ui.theme);

export function iconSrc(icon: AppIcon): string {
  return ICON_SRC[icon];
}

export function setTheme(theme: Theme) {
  ui.theme = theme;
  apply(theme);
  if (typeof localStorage !== 'undefined') {
    try {
      localStorage.setItem(THEME_KEY, theme);
    } catch {
      // sin persistencia disponible
    }
  }
}

export function setIcon(icon: AppIcon) {
  ui.icon = icon;
  if (typeof localStorage !== 'undefined') {
    try {
      localStorage.setItem(ICON_KEY, icon);
    } catch {
      // sin persistencia disponible
    }
  }
}
