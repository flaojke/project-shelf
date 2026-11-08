import type { CSSProperties, ReactNode } from 'react';

export type IconName = 'shelf' | 'grid' | 'star' | 'clock' | 'search' | 'arrow' | 'back' | 'close' | 'settings' | 'play' | 'stop' | 'refresh' | 'external' | 'chevron' | 'folder' | 'check' | 'alert' | 'device' | 'download' | 'tray' | 'power' | 'terminal' | 'info';

export function Icon({ name, size = 20, className = '', style }: { name: IconName; size?: number; className?: string; style?: CSSProperties }) {
  const paths: Record<IconName, ReactNode> = {
    shelf: <><path d="M4 3.5h4v15H4zm6 3h4v12h-4z"/><path d="m16 4 3.8-.7 2.7 14.8-3.8.7zM2 21h21"/></>,
    grid: <><rect x="3" y="3" width="7" height="7" rx="1.7"/><rect x="14" y="3" width="7" height="7" rx="1.7"/><rect x="3" y="14" width="7" height="7" rx="1.7"/><rect x="14" y="14" width="7" height="7" rx="1.7"/></>,
    star: <path d="m12 3 2.8 5.7 6.3.9-4.5 4.4 1.1 6.2-5.7-3-5.7 3 1.1-6.2L3.2 9.6l6.3-.9z"/>,
    clock: <><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/></>,
    search: <><circle cx="10.8" cy="10.8" r="6.8"/><path d="m16 16 4.5 4.5"/></>,
    arrow: <path d="M4 12h15m-6-6 6 6-6 6"/>,
    back: <path d="M20 12H5m6-6-6 6 6 6"/>,
    close: <path d="m6 6 12 12M18 6 6 18"/>,
    settings: <><path d="m9.5 3-.7 2.3-2.2 1-2.2-.6-2 3.5L4 10.9v2.3l-1.6 1.6 2 3.5 2.2-.5 2.1 1L9.5 21h5l.7-2.3 2.2-1 2.2.6 2-3.5-1.6-1.7v-2.3l1.6-1.6-2-3.5-2.2.5-2.1-1L14.5 3z"/><circle cx="12" cy="12" r="3"/></>,
    play: <path d="m8 5 11 7-11 7z"/>,
    stop: <rect x="6" y="6" width="12" height="12" rx="2"/>,
    refresh: <><path d="M20 7v5h-5M4 17v-5h5"/><path d="M6.1 6.1a8.3 8.3 0 0 1 13.8 4.8M17.9 17.9a8.3 8.3 0 0 1-13.8-4.8"/></>,
    external: <><path d="M14 3h7v7m-1-6L10 14"/><path d="M10 4H5a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2h13a2 2 0 0 0 2-2v-5"/></>,
    chevron: <path d="m9 5 7 7-7 7"/>,
    folder: <path d="M3 7V5a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>,
    check: <path d="m5 12 4.5 4.5L19 7"/>,
    alert: <><path d="m12 3 10 18H2zM12 9v5"/><path d="M12 17h.01"/></>,
    device: <><rect x="6" y="2" width="12" height="20" rx="3"/><path d="M10 5h4m-3 14h2"/></>,
    download: <><path d="M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5"/></>,
    tray: <><path d="M3 13v7h18v-7M12 3v11m-5-5 5 5 5-5"/></>,
    power: <><path d="M12 2v10M6 5a9 9 0 1 0 12 0"/></>,
    terminal: <><rect x="2" y="4" width="20" height="16" rx="3"/><path d="m6 9 3 3-3 3m7 0h4"/></>,
    info: <><circle cx="12" cy="12" r="9"/><path d="M12 11v6m0-10h.01"/></>,
  };
  return <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" className={`icon ${className}`} style={style} fill="none" stroke="currentColor" strokeWidth="1.65" strokeLinecap="round" strokeLinejoin="round">{paths[name]}</svg>;
}
