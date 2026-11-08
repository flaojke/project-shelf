import type { AndroidDevice, LogEntry, Profile, ProjectDiagnostics, ProfileValidation, ProjectStatus, QuitSummary, Settings } from './types';
import { validateDemoProfile } from './validation.ts';

type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
declare global {
  interface Window {
    __TAURI_INTERNALS__?: { invoke: Invoke };
    __TAURI__?: { event: { listen: (name: string, callback: () => void) => Promise<() => void> } };
  }
}
export const isDemo = !window.__TAURI_INTERNALS__;
const invoke: Invoke = (command, args) => window.__TAURI_INTERNALS__
  ? window.__TAURI_INTERNALS__.invoke(command, args)
  : Promise.reject(new Error('Нативный модуль недоступен.'));

const demoProfiles: Profile[] = [
  { id: 'web', name: 'Веб-проект', subtitle: 'Vite-приложение с управляемым запуском', kind: 'process', path: '', url: 'http://127.0.0.1:5180/', accent: '#477561', cover: 'blueprint', favorite: true, tags: ['Vite', 'Web'], config: { nodePath: '', port: 5180, readyMarker: '' } },
  { id: 'powershell', name: 'PowerShell-сайт', subtitle: 'Локальный сайт с проверкой health identity', kind: 'powershell', path: '', url: 'http://127.0.0.1:8877/', accent: '#8a709b', cover: 'powershell', favorite: false, tags: ['PowerShell', 'HTTP'], config: { port: 8877, service: '', allowStop: false } },
  { id: 'docker', name: 'Docker-сервис', subtitle: 'Один сервис из вашего Compose-файла', kind: 'docker', path: '', url: 'http://127.0.0.1:8080/', accent: '#567b96', cover: 'docker', favorite: false, tags: ['Docker', 'Compose'], config: { dockerPath: '', composeFile: '', service: '', healthMessage: 'app is alive' } },
  { id: 'android', name: 'Android-приложение', subtitle: 'Эмулятор или явно выбранное устройство', kind: 'android', path: '', accent: '#ce6a40', cover: 'android', favorite: true, tags: ['Android', 'ADB'], config: { sdkRoot: '', avd: '', avdHome: '', androidUserHome: '', serial: null, packageName: '', activity: '', apkPath: '' } },
];
const statuses = new Map<string, ProjectStatus>(demoProfiles.map(p => [p.id, { id: p.id, phase: 'setup', owned: false, message: 'Сначала настройте пути и параметры профиля.' }]));
const logs = new Map<string, LogEntry[]>();
let demoSettings: Settings = { closeToTray: false, autoOpenBrowser: true, theme: 'light', liveActionsEnabled: false };
try {
  const saved = JSON.parse(localStorage.getItem('project-shelf-demo-favorites') || '{}') as Record<string, boolean>;
  for (const p of demoProfiles) if (typeof saved[p.id] === 'boolean') p.favorite = saved[p.id];
} catch { /* Preview preferences are optional. */ }
const clone = <T,>(value: T): T => structuredClone(value);
const log = (id: string, message: string) => {
  const current = logs.get(id) || [];
  current.push({ timestamp: new Date().toISOString(), level: 'info', message });
  logs.set(id, current.slice(-300));
};
const profile = (id: string) => {
  const p = demoProfiles.find(p => p.id === id);
  if (!p) throw new Error('Неизвестный профиль.');
  return p;
};
const requireConfigured = (id: string) => {
  const validation = validateDemoProfile(profile(id));
  if (!validation.complete) throw new Error(`Сначала настройте профиль: ${validation.issues.join(' ')}`);
};

export const api = {
  projectDiagnostics: async (id: string): Promise<ProjectDiagnostics> => {
    if (!isDemo) return invoke('project_diagnostics', { id });
    const p = demoProfiles.find(p => p.id === id);
    if (!p) throw new Error('Неизвестный профиль.');
    return {
      id, checkedAt: new Date().toISOString(), technologies: [], canStop: false, canRestart: false,
      stopReason: 'В браузерном предпросмотре нет управления системными процессами.',
      restartReason: 'Перезапуск доступен только для управляемого ресурса в Windows-приложении.',
      checks: (['dependencies', 'process', 'http', 'port'] as const).map((key) => ({
        id: key, label: { dependencies: 'Зависимости', process: 'Процесс', http: 'HTTP', port: 'Порт' }[key],
        state: p.kind === 'android' && (key === 'http' || key === 'port') ? 'notApplicable' : 'unknown',
        message: p.kind === 'android' && (key === 'http' || key === 'port')
          ? 'Android проверяется через ADB.' : 'Предпросмотр: реальные системные проверки не выполнялись.',
      })),
    };
  },
  restartProject: async (id: string): Promise<ProjectStatus> => {
    if (!isDemo) return invoke('restart_project', { id });
    throw new Error('Перезапуск системного процесса недоступен в браузерном предпросмотре.');
  },
  listProfiles: (): Promise<Profile[]> => isDemo ? Promise.resolve(clone(demoProfiles)) : invoke('list_profiles'),
  getStatuses: (): Promise<ProjectStatus[]> => isDemo ? Promise.resolve(clone([...statuses.values()])) : invoke('get_statuses'),
  profileValidation: (value: Profile): Promise<ProfileValidation> => isDemo ? Promise.resolve(validateDemoProfile(value)) : invoke('profile_validation', { profile: value }),
  startProject: async (id: string): Promise<ProjectStatus> => {
    if (!isDemo) return invoke('start_project', { id });
    requireConfigured(id);
    const current = statuses.get(id)!;
    if (current.phase === 'starting' || current.phase === 'ready') return clone(current);
    const next: ProjectStatus = { id, phase: 'starting', owned: profile(id).kind !== 'powershell', message: 'Демонстрация запуска, без команд ОС.' };
    statuses.set(id, next); log(id, 'Предпросмотр: начало демонстрации. Процессы не запускаются.');
    window.setTimeout(() => {
      const owned = profile(id).kind !== 'powershell';
      statuses.set(id, { id, phase: owned ? 'ready' : 'external', owned, startedAt: new Date().toISOString(), ...(id === 'android' ? { serial: String(profile(id).config.serial || 'emulator-5554') } : {}) });
      log(id, 'Предпросмотр: демонстрационная готовность.');
    }, 1000);
    return clone(next);
  },
  stopProject: async (id: string): Promise<ProjectStatus> => {
    if (!isDemo) return invoke('stop_project', { id });
    if (profile(id).kind === 'powershell') throw new Error('Безопасная остановка PowerShell-сайта не поддерживается.');
    if (!statuses.get(id)?.owned) throw new Error('Проект запущен вне текущего сеанса.');
    const next: ProjectStatus = { id, phase: 'stopped', owned: false };
    statuses.set(id, next); log(id, 'Предпросмотр: демонстрация остановлена.'); return clone(next);
  },
  openProject: async (id: string): Promise<void> => { if (!isDemo) return invoke('open_project', { id }); requireConfigured(id); log(id, 'Предпросмотр: открытие доступно в нативном приложении.'); },
  projectLogs: (id: string): Promise<LogEntry[]> => isDemo ? Promise.resolve(clone(logs.get(id) || [])) : invoke('project_logs', { id }),
  saveProfile: async (value: Profile): Promise<void> => {
    if (!isDemo) return invoke('save_profile', { profile: value });
    const current = profile(value.id);
    if (current.kind !== value.kind) throw new Error('Тип профиля изменять нельзя.');
    const withoutFavorite = (p: Profile) => ({ ...p, favorite: false });
    if (statuses.get(value.id)?.owned && JSON.stringify(withoutFavorite(current)) !== JSON.stringify(withoutFavorite(value))) throw new Error('Сначала остановите управляемый проект.');
    const i = demoProfiles.findIndex(p => p.id === value.id); demoProfiles[i] = clone(value);
    if (!statuses.get(value.id)?.owned) {
      const validation = validateDemoProfile(value);
      statuses.set(value.id, { id: value.id, phase: validation.complete ? 'stopped' : 'setup', owned: false, message: validation.complete ? 'Профиль настроен.' : validation.issues.join(' ') });
    }
    localStorage.setItem('project-shelf-demo-favorites', JSON.stringify(Object.fromEntries(demoProfiles.map(p => [p.id, p.favorite]))));
  },
  getSettings: (): Promise<Settings> => isDemo ? Promise.resolve(clone(demoSettings)) : invoke('get_settings'),
  saveSettings: async (settings: Settings): Promise<void> => { if (!isDemo) return invoke('save_settings', { settings }); demoSettings = clone(settings); },
  androidDevices: (): Promise<AndroidDevice[]> => {
    if (!isDemo) return invoke('android_devices');
    requireConfigured('android'); return Promise.resolve([{ serial: 'emulator-5554', state: 'device', model: 'Example emulator' }]);
  },
  installAndroidApk: async (id: string, serial: string): Promise<void> => {
    if (!isDemo) return invoke('install_android_apk', { id, serial });
    requireConfigured(id); if (!serial || !profile(id).config.apkPath) throw new Error('Укажите APK и конкретное готовое устройство.');
    log(id, 'Предпросмотр: установка APK не выполнялась.');
  },
  quitApp: async (stopOwned: boolean): Promise<QuitSummary> => {
    if (!isDemo) return invoke('quit_app', { stopOwned });
    const managed = [...statuses.values()].filter(s => s.owned && s.phase === 'ready');
    if (stopOwned) for (const state of managed) await api.stopProject(state.id);
    return { managed, retained: [], failed: [] };
  },
  hideWindow: async (): Promise<void> => { if (!isDemo) return invoke('hide_window'); },
};

if (!isDemo && window.__TAURI__?.event) {
  void window.__TAURI__.event.listen('request-close', () => window.dispatchEvent(new CustomEvent('shelf:close-requested')));
  void window.__TAURI__.event.listen('request-quit', () => window.dispatchEvent(new CustomEvent('shelf:quit-requested')));
}
