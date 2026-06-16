export type AppIcon = 'color' | 'mono';

const ICON_KEY = 'flashback.icon';

const ICON_SRC: Record<AppIcon, string> = {
  color: '/flashback.svg',
  mono: '/flashback-mono.svg'
};

function loadIcon(): AppIcon {
  if (typeof localStorage === 'undefined') return 'color';
  return localStorage.getItem(ICON_KEY) === 'mono' ? 'mono' : 'color';
}

export const ui = $state<{ icon: AppIcon }>({
  icon: loadIcon()
});

export function iconSrc(icon: AppIcon): string {
  return ICON_SRC[icon];
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
