import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { api, isDemo } from './lib/api';
import type { AndroidDevice, LogEntry, Profile, ProjectPhase, ProjectStatus, QuitSummary, Settings } from './lib/types';
import { Icon } from './components/Icons';
import { ProjectCover } from './components/ProjectCover';
import { ReadinessPanel } from './components/ReadinessPanel';
import { TechnologyChips } from './components/TechnologyChips';
import { ProjectSetup } from './components/ProjectSetup';
import type { ValidationResult } from './components/ProjectSetup';

type View = 'all' | 'favorites' | 'recent';
type Action = 'start' | 'stop' | 'open' | 'install' | 'restart';
type Toast = { id: number; message: string; error: boolean };
const phaseLabels: Record<ProjectPhase, string> = { setup: 'Требуется настройка', stopped: 'Не запущен', starting: 'Запускается', ready: 'Работает', external: 'Уже работает', stopping: 'Останавливается', error: 'Нужна проверка', unavailable: 'Недоступен' };
const defaultSettings: Settings = { autoOpenBrowser: true, closeToTray: true, theme: 'light', liveActionsEnabled: false };
const kindLabels = { process: 'Веб-проект', powershell: 'PowerShell', docker: 'Docker', android: 'Android' };
const blurbs = {
  android: 'Ваше приложение на телефоне или в эмуляторе. Без лишних шагов.',
  process: 'Рабочее пространство для вашего локального веб-проекта.',
  powershell: 'Локальный сайт с понятным запуском и проверкой готовности.',
  docker: 'Подготовленный Docker-сервис, всегда под рукой.',
};
const errorText = (error: unknown) => error instanceof Error ? error.message : String(error);
const isRunning = (s: ProjectStatus) => s.phase === 'ready' || s.phase === 'external';
function readRecents(): Record<string, string> {
  try { return JSON.parse(localStorage.getItem('shelf-recents') || '{}'); } catch { return {}; }
}
function StatusPill({ status }: { status: ProjectStatus }) {
  return <span className={`status-pill status-${status.phase}`}><span className="status-dot" />{phaseLabels[status.phase]}</span>;
}

function Modal({ children, title, onClose, className = '' }: { children: ReactNode; title: string; onClose: () => void; className?: string }) {
  const dialog = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const first = dialog.current?.querySelector<HTMLElement>('[data-autofocus]') || dialog.current?.querySelector<HTMLElement>('button, input, select');
    first?.focus();
    const keydown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') { event.preventDefault(); onClose(); }
      if (event.key === 'Tab') {
        const elements = Array.from(dialog.current?.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), [tabindex="0"]') || []);
        const firstItem = elements[0], lastItem = elements[elements.length - 1];
        if (event.shiftKey && document.activeElement === firstItem) { event.preventDefault(); lastItem?.focus(); }
        else if (!event.shiftKey && document.activeElement === lastItem) { event.preventDefault(); firstItem?.focus(); }
      }
    };
    document.addEventListener('keydown', keydown);
    return () => { document.removeEventListener('keydown', keydown); previous?.focus(); };
  }, [onClose]);
  return <div className="modal-backdrop" onMouseDown={e => { if (e.target === e.currentTarget) onClose(); }}>
    <div ref={dialog} className={`modal ${className}`} role="dialog" aria-modal="true" aria-label={title}>
      <div className="modal-heading"><h2>{title}</h2><button className="icon-button" aria-label="Закрыть диалог" onClick={onClose}><Icon name="close" /></button></div>
      {children}
    </div>
  </div>;
}

export default function App() {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [validations, setValidations] = useState<Record<string, ValidationResult>>({});
  const [setupId, setSetupId] = useState<string | null>(null);
  const [statuses, setStatuses] = useState<ProjectStatus[]>([]);
  const [settings, setSettings] = useState<Settings>(defaultSettings);
  const [view, setView] = useState<View>('all');
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState<string | null>(null);
  const [busy, setBusy] = useState<Record<string, Action>>({});
  const [favoriteSaving, setFavoriteSaving] = useState<Record<string, boolean>>({});
  const [recents, setRecents] = useState<Record<string, string>>(readRecents);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState('');
  const [connectionError, setConnectionError] = useState('');
  const [toast, setToast] = useState<Toast | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [closeIntent, setCloseIntent] = useState<'close' | 'quit' | null>(null);
  const [exitBusy, setExitBusy] = useState(false);
  const [quitResult, setQuitResult] = useState<QuitSummary | null>(null);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [logsBusy, setLogsBusy] = useState(false);
  const [logError, setLogError] = useState('');
  const [devices, setDevices] = useState<AndroidDevice[]>([]);
  const [deviceBusy, setDeviceBusy] = useState(false);
  const [deviceError, setDeviceError] = useState('');
  const [serial, setSerial] = useState('');
  const [installTarget, setInstallTarget] = useState<string | null>(null);
  const [deviceSaving, setDeviceSaving] = useState(false);
  const [settingsBusy, setSettingsBusy] = useState(false);
  const searchInput = useRef<HTMLInputElement>(null);
  const statusRequest = useRef(0);
  const statusInFlight = useRef(false);
  const logRequest = useRef(0);
  const deviceRequest = useRef(0);
  const profilesRef = useRef(profiles);
  profilesRef.current = profiles;
  const actionLocks = useRef(new Set<string>());
  const favoriteLocks = useRef(new Set<string>());

  const notify = useCallback((message: string, error = false) => setToast({ id: Date.now(), message, error }), []);
  useEffect(() => { if (!toast) return; const timer = window.setTimeout(() => setToast(null), toast.error ? 8500 : 4000); return () => clearTimeout(timer); }, [toast]);

  const refreshStatuses = useCallback(async () => {
    if (statusInFlight.current) return;
    statusInFlight.current = true;
    const request = ++statusRequest.current;
    try {
      const next = await api.getStatuses();
      if (request !== statusRequest.current) return;
      setStatuses(next); setConnectionError('');
      setRecents(previous => {
        const nextRecents = { ...previous }; let changed = false;
        next.forEach(s => { if (s.startedAt && s.startedAt !== nextRecents[s.id]) { nextRecents[s.id] = s.startedAt; changed = true; } });
        if (changed) { try { localStorage.setItem('shelf-recents', JSON.stringify(nextRecents)); } catch { /* unavailable storage is non-fatal */ } }
        return changed ? nextRecents : previous;
      });
    } catch (error) { if (request === statusRequest.current) setConnectionError(errorText(error)); }
    finally { statusInFlight.current = false; }
  }, []);

  const load = useCallback(async () => {
    setLoading(true); setLoadError('');
    const [profileResult, settingsResult] = await Promise.allSettled([api.listProfiles(), api.getSettings()]);
    if (profileResult.status === 'fulfilled') {
      setProfiles(profileResult.value);
      const checks = await Promise.all(profileResult.value.map(async p => {
        try { return [p.id, await api.profileValidation(p)] as const; }
        catch (error) { return [p.id, { complete: false, issues: [errorText(error)] }] as const; }
      }));
      setValidations(Object.fromEntries(checks));
    }
    else setLoadError(errorText(profileResult.reason));
    if (settingsResult.status === 'fulfilled') setSettings(settingsResult.value);
    await refreshStatuses(); setLoading(false);
  }, [refreshStatuses]);
  useEffect(() => { void load(); }, [load]);
  useEffect(() => { const timer = window.setInterval(() => void refreshStatuses(), 1800); return () => clearInterval(timer); }, [refreshStatuses]);
  useEffect(() => {
    const keydown = (e: KeyboardEvent) => { if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); if (document.querySelector('[role="dialog"][aria-modal="true"]')) return; setSelected(null); window.requestAnimationFrame(() => searchInput.current?.focus()); } };
    const close = () => { setSetupId(null); setSettingsOpen(false); setInstallTarget(null); setQuitResult(null); setCloseIntent('close'); };
    const quit = () => { setSetupId(null); setSettingsOpen(false); setInstallTarget(null); setQuitResult(null); setCloseIntent('quit'); };
    window.addEventListener('keydown', keydown); window.addEventListener('shelf:close-requested', close);
    window.addEventListener('shelf:quit-requested', quit);
    return () => { window.removeEventListener('keydown', keydown); window.removeEventListener('shelf:close-requested', close); window.removeEventListener('shelf:quit-requested', quit); };
  }, []);

  const statusFor = useCallback((id: string): ProjectStatus => {
    const status = statuses.find(s => s.id === id) || { id, phase: 'unavailable' as const, owned: false, message: 'Состояние пока не получено.' };
    if (!validations[id]?.complete) return { ...status, phase: 'setup', message: validations[id]?.issues.join(' ') || 'Настройте профиль перед первым запуском.' };
    return (busy[id] === 'start' || busy[id] === 'restart') ? { ...status, phase: 'starting' } : busy[id] === 'stop' ? { ...status, phase: 'stopping' } : status;
  }, [statuses, busy, validations]);

  function runtimeLocked(id: string) {
    const s = statuses.find(item => item.id === id);
    return busy[id] === 'start' || busy[id] === 'stop' || busy[id] === 'restart' || !!(s?.owned && ['starting', 'ready', 'stopping'].includes(s.phase));
  }

  const profile = profiles.find(p => p.id === selected);
  const currentStatus = profile ? statusFor(profile.id) : undefined;
  const refreshLogs = useCallback(async (id: string) => {
    const request = ++logRequest.current; setLogsBusy(true); setLogError('');
    try { const next = await api.projectLogs(id); if (request === logRequest.current) setLogs(next); }
    catch (error) { if (request === logRequest.current) setLogError(errorText(error)); }
    finally { if (request === logRequest.current) setLogsBusy(false); }
  }, []);
  const refreshDevices = useCallback(async () => {
    const request = ++deviceRequest.current;
    setDeviceBusy(true); setDeviceError('');
    try { const next = await api.androidDevices(); if (request === deviceRequest.current) setDevices(next); }
    catch (error) { if (request === deviceRequest.current) setDeviceError(errorText(error)); }
    finally { if (request === deviceRequest.current) setDeviceBusy(false); }
  }, []);
  useEffect(() => {
    setLogs([]); setLogError(''); setInstallTarget(null);
    if (!profile) { ++logRequest.current; return; }
    setSerial(typeof profile.config.serial === 'string' ? profile.config.serial : '');
    void refreshLogs(profile.id);
    if (profile.kind === 'android' && validations[profile.id]?.complete) void refreshDevices();
    const id = profile.id;
    const timer = window.setInterval(() => void refreshLogs(id), 4000);
    return () => { ++logRequest.current; clearInterval(timer); };
  }, [selected, validations[profile?.id || '']?.complete, refreshLogs, refreshDevices]);
  useEffect(() => {
    if (profile?.kind === 'android' && validations[profile.id]?.complete && currentStatus?.serial && isRunning(currentStatus)) void refreshDevices();
  }, [profile?.id, currentStatus?.serial, currentStatus?.phase, refreshDevices]);

  async function selectDevice(p: Profile, nextSerial: string) {
    if (runtimeLocked(p.id)) { notify('Сначала остановите управляемый запуск, чтобы сменить устройство.', true); return; }
    if (actionLocks.current.has(p.id) || favoriteLocks.current.has(p.id)) return;
    const previousSerial = serial;
    actionLocks.current.add(p.id); setDeviceSaving(true); setSerial(nextSerial);
    const latest = profilesRef.current.find(item => item.id === p.id) || p;
    const config = { ...latest.config };
    if (nextSerial) config.serial = nextSerial;
    else delete config.serial;
    try {
      const validation = await api.profileValidation({ ...latest, config });
      await api.saveProfile({ ...latest, config });
      setProfiles(items => items.map(item => item.id === p.id ? { ...item, config } : item));
      setValidations(previous => ({ ...previous, [p.id]: validation }));
      await refreshStatuses();
    } catch (error) { setSerial(previousSerial); notify(`Не удалось выбрать устройство: ${errorText(error)}`, true); }
    finally { actionLocks.current.delete(p.id); setDeviceSaving(false); }
  }

  async function toggleFavorite(p: Profile) {
    if (favoriteLocks.current.has(p.id) || deviceSaving) return;
    favoriteLocks.current.add(p.id); setFavoriteSaving(previous => ({ ...previous, [p.id]: true }));
    const latest = profilesRef.current.find(item => item.id === p.id) || p;
    const updated = { ...latest, favorite: !latest.favorite };
    setProfiles(items => items.map(item => item.id === p.id ? { ...item, favorite: updated.favorite } : item));
    try { await api.saveProfile(updated); }
    catch (error) { setProfiles(items => items.map(item => item.id === p.id && item.favorite === updated.favorite ? { ...item, favorite: latest.favorite } : item)); notify(`Не удалось сохранить избранное: ${errorText(error)}`, true); }
    finally { favoriteLocks.current.delete(p.id); setFavoriteSaving(previous => ({ ...previous, [p.id]: false })); }
  }

  async function runAction(p: Profile, action: Action, targetSerial?: string) {
    if (actionLocks.current.has(p.id) || favoriteLocks.current.has(p.id)) return;
    if (action !== 'stop' && !validations[p.id]?.complete) { setSetupId(p.id); return; }
    if (action === 'install' && !targetSerial) { notify('Для установки APK нужно готовое конкретное устройство.', true); return; }
    actionLocks.current.add(p.id); setBusy(previous => ({ ...previous, [p.id]: action }));
    try {
      if (action === 'start') await api.startProject(p.id);
      if (action === 'restart') await api.restartProject(p.id);
      if (action === 'stop') await api.stopProject(p.id);
      if (action === 'open') await api.openProject(p.id);
      if (action === 'install') { await api.installAndroidApk(p.id, targetSerial!); notify(isDemo ? 'Демо: установка APK показана без изменений на устройстве.' : 'APK установлен. Данные приложения сохранены.'); }
      if (action === 'start' || action === 'restart') {
        const stamp = new Date().toISOString();
        setRecents(previous => { const next = { ...previous, [p.id]: stamp }; try { localStorage.setItem('shelf-recents', JSON.stringify(next)); } catch { /* no storage */ } return next; });
      }
      await refreshStatuses(); if (selected === p.id) { await refreshLogs(p.id); if (p.kind === 'android') await refreshDevices(); }
    } catch (error) { notify(errorText(error), true); await refreshStatuses(); }
    finally { actionLocks.current.delete(p.id); setBusy(previous => { const next = { ...previous }; delete next[p.id]; return next; }); }
  }

  function primaryLabel(p: Profile, s: ProjectStatus) {
    if (s.phase === 'setup') return 'Настроить';
    if (s.phase === 'starting') return 'Показать запуск';
    if (s.phase === 'stopping') return 'Останавливается…';
    if (s.phase === 'unavailable') return 'Проверить';
    if (isRunning(s)) return p.kind === 'android' ? 'Открыть приложение' : 'Открыть';
    if (s.phase === 'error') return 'Повторить запуск';
    return p.kind === 'android' ? 'Запустить приложение' : 'Запустить';
  }
  function primaryAction(p: Profile, s: ProjectStatus) {
    if (s.phase === 'setup') { setSetupId(p.id); return; }
    if (s.phase === 'starting' || s.phase === 'stopping' || s.phase === 'unavailable') { setSelected(p.id); if (s.phase === 'unavailable') { void refreshStatuses(); void refreshLogs(p.id); if (p.kind === 'android') void refreshDevices(); } return; }
    void runAction(p, isRunning(s) ? 'open' : 'start');
  }
  async function saveSetting(key: 'autoOpenBrowser' | 'closeToTray', value: boolean) {
    if (settingsBusy) return;
    const previous = settings; const next = { ...settings, [key]: value };
    setSettings(next); setSettingsBusy(true);
    try { await api.saveSettings(next); }
    catch (error) { setSettings(previous); notify(errorText(error), true); }
    finally { setSettingsBusy(false); }
  }
  async function leave(stopOwned: boolean) {
    setExitBusy(true);
    try {
      if (closeIntent === 'close' && settings.closeToTray && !stopOwned && !quitResult) { await api.hideWindow(); setCloseIntent(null); }
      else { const result = await api.quitApp(stopOwned); if (result && (result.retained.length || result.failed.length)) setQuitResult(result); else { setCloseIntent(null); if (isDemo) notify('Демо: запрос на выход обработан. Предпросмотр остаётся открыт.'); } }
    } catch (error) { notify(errorText(error), true); }
    finally { setExitBusy(false); }
  }
  const closeModal = useCallback(() => { if (!exitBusy) { setCloseIntent(null); setQuitResult(null); } }, [exitBusy]);
  const closeSettings = useCallback(() => setSettingsOpen(false), []);
  const closeInstall = useCallback(() => setInstallTarget(null), []);
  const closeSetup = useCallback(() => setSetupId(null), []);
  const projectSaved = useCallback((saved: Profile, validation: ValidationResult) => {
    setProfiles(items => items.map(item => item.id === saved.id ? { ...saved, favorite: item.favorite } : item));
    setValidations(previous => ({ ...previous, [saved.id]: validation }));
    if (saved.kind === 'android') {
      setSerial(typeof saved.config.serial === 'string' ? saved.config.serial : '');
      ++deviceRequest.current; setDevices([]); setDeviceError(''); setDeviceBusy(false);
      if (validation.complete) void refreshDevices();
    }
    void refreshStatuses();
    notify(validation.complete ? 'Настройки сохранены. Проект готов к запуску.' : 'Настройка сохранена. Заполните оставшиеся поля перед запуском.');
  }, [refreshStatuses, refreshDevices, notify]);

  const filtered = useMemo(() => profiles.filter(p => (view !== 'favorites' || p.favorite) && (view !== 'recent' || recents[p.id]) && `${p.name} ${p.subtitle} ${p.tags.join(' ')}`.toLocaleLowerCase('ru').includes(query.trim().toLocaleLowerCase('ru'))).sort((a,b) => view === 'recent' ? (recents[b.id] || '').localeCompare(recents[a.id] || '') : 0), [profiles, view, query, recents]);
  const runningCount = profiles.filter(p => isRunning(statusFor(p.id))).length;
  const favoriteCount = profiles.filter(p => p.favorite).length;
  const recentCount = profiles.filter(p => recents[p.id]).length;
  const managed = statuses.filter(s => s.owned && (isRunning(s) || s.phase === 'starting'));
  const powershellRetained = managed.some(s => profiles.find(p => p.id === s.id)?.kind === 'powershell');
  const installSerial = serial || currentStatus?.serial || '';
  const installDeviceOnline = !!installSerial && devices.some(device => device.serial === installSerial && device.state === 'device');
  const defaultAvd = typeof profile?.config.avd === 'string' ? profile.config.avd : '';
  const setupProfile = profiles.find(item => item.id === setupId);
  const profileConfigured = !!profile && !!validations[profile.id]?.complete;
  const profileEditLocked = !!profile && runtimeLocked(profile.id);
  const hasApk = typeof profile?.config.apkPath === 'string' && !!profile.config.apkPath.trim();
  const displayPath = profile?.path || (typeof profile?.config.composeFile === 'string' ? profile.config.composeFile : typeof profile?.config.sdkRoot === 'string' ? profile.config.sdkRoot : '') || 'Пока не задано';

  return <div className="app-shell">
    <aside className="sidebar" aria-label="Основная навигация">
      <button className="brand" onClick={() => { setView('all'); setSelected(null); setQuery(''); }} aria-label="Project Shelf — все проекты"><span className="brand-mark"><Icon name="shelf" size={25} /></span><span>Project<span className="brand-second">Shelf<span className="brand-period">.</span></span></span></button>
      <span className="sidebar-caption">ВАША КОЛЛЕКЦИЯ</span>
      <nav className="navigation">
        {([{id:'all', icon:'grid', label:'Все проекты', count:profiles.length},{id:'favorites',icon:'star',label:'Избранное',count:favoriteCount},{id:'recent',icon:'clock',label:'Недавние',count:recentCount}] as const).map(item => <button key={item.id} className={`nav-item ${view === item.id && !selected ? 'active' : ''}`} aria-current={view === item.id && !selected ? 'page' : undefined} onClick={() => { setView(item.id); setSelected(null); }}><Icon name={item.icon} size={19}/><span>{item.label}</span><span className="nav-count">{item.count}</span></button>)}
      </nav>
      <div className="sidebar-note"><div className="little-shelf" aria-hidden="true"><span/><span/><span/><span/></div><p>Меньше подготовки.<br/><strong>Больше ваших идей.</strong></p><span>Всё нужное на одной полке.</span></div>
      <div className="sidebar-bottom"><button className="nav-item" onClick={() => setSettingsOpen(true)}><Icon name="settings" size={19}/><span>Настройки</span></button><div className="app-version"><span className="small-dot"/>Локально на Windows <span>v0.2</span></div></div>
    </aside>
    <div className="workspace">
      <header className="topbar"><div className="breadcrumb">Моё пространство<span>/</span><strong>{profile ? profile.name : 'Библиотека проектов'}</strong></div><div className="window-actions"><button className="quiet-button tray-button" onClick={() => { void api.hideWindow().then(() => { if (isDemo) notify('Демо: окно было бы скрыто в системный трей.'); }).catch(error => notify(errorText(error), true)); }} title="Скрыть окно в системный трей. Проекты продолжат работу."><Icon name="tray" size={16}/>В трей</button><span className="window-divider"/><button className="icon-button" title="Выход из Project Shelf" aria-label="Выйти из Project Shelf" onClick={() => { setQuitResult(null); setCloseIntent('quit'); }}><Icon name="power" size={18}/></button></div></header>
      {isDemo && <div className="demo-banner" role="note"><span className="demo-label">ДЕМО</span><span>Предпросмотр интерфейса · состояния искусственные, команды на компьютере не выполняются.</span></div>}
      <main className="main-content" id="main-content">
        {profile && currentStatus ? <>
          <button className="back-link" onClick={() => setSelected(null)}><Icon name="back" size={17}/>К коллекции</button>
          <div className="detail-heading"><div><div className="eyebrow">НА ВАШЕЙ ПОЛКЕ / {kindLabels[profile.kind].toLocaleUpperCase('ru')}</div><h1>{profile.name}</h1><p>{profile.subtitle}</p></div><div className="detail-heading-actions"><button className="button button-secondary project-settings-button" disabled={profileEditLocked || favoriteSaving[profile.id] || !!busy[profile.id]} title={profileEditLocked ? 'Настройки доступны после завершения управляемого запуска' : 'Настроить пути и параметры запуска'} onClick={() => setSetupId(profile.id)}><Icon name="settings" size={16}/>Настройки проекта</button><button className={`favorite-button detail-favorite ${profile.favorite ? 'is-favorite' : ''}`} onClick={() => void toggleFavorite(profile)} aria-label={profile.favorite ? 'Убрать из избранного' : 'Добавить в избранное'} disabled={favoriteSaving[profile.id] || deviceSaving} aria-pressed={profile.favorite}><Icon name="star" size={20}/>{profile.favorite ? 'В избранном' : 'В избранное'}</button></div></div>
          <div className="detail-grid"><div className="detail-main"><ProjectCover profile={profile} className="detail-cover"/>
            <section className="detail-panel launch-panel"><div className="section-heading"><h2>Готовность к работе</h2><StatusPill status={currentStatus}/></div><p className="status-description">{currentStatus.message || (isRunning(currentStatus) ? 'Можно открывать проект и продолжать работу.' : 'Project Shelf проверит необходимые компоненты перед запуском.')}</p>
              {currentStatus.phase === 'external' && <div className="inline-note"><Icon name="info" size={17}/><span>Проект запущен вне этой сессии Project Shelf. Его можно открыть; остановка здесь недоступна.</span></div>}
              {profile.kind === 'android' && profileConfigured && <div className="device-block"><div className="field-label-row"><label htmlFor="android-device">Устройство Android</label><button className="text-button" onClick={() => void refreshDevices()} disabled={deviceBusy}><Icon name="refresh" size={14}/>{deviceBusy ? 'Поиск…' : 'Обновить'}</button></div><select id="android-device" value={serial} onChange={e => void selectDevice(profile, e.target.value)} disabled={!!busy[profile.id] || deviceSaving || favoriteSaving[profile.id] || profileEditLocked}><option value="">{defaultAvd ? `Эмулятор ${defaultAvd} (по умолчанию)` : 'Эмулятор не настроен'}</option>{serial && !devices.some(d => d.serial === serial) && <option value={serial}>{serial} — не подключено</option>}{devices.map(d => <option value={d.serial} key={d.serial} disabled={d.state !== 'device'}>{d.model || d.avd || d.serial} · {d.serial}{d.state !== 'device' ? ` · ${d.state}` : ''}</option>)}</select>{deviceError && <p className="field-error" role="alert">{deviceError}</p>}<p className="field-help">{serial ? 'Используется выбранное устройство. Открывается уже установленное приложение.' : `Project Shelf запустит AVD ${defaultAvd}, дождётся его готовности и откроет приложение.`} Android Studio не нужна.</p>{profileEditLocked && <div className="inline-note"><Icon name="info" size={17}/><span>Чтобы изменить устройство или параметры, сначала завершите управляемый запуск. Избранное можно менять в любое время.</span></div>}{!deviceBusy && devices.length === 0 && !deviceError && <div className="inline-note"><Icon name="device" size={17}/><span>{defaultAvd && !serial ? 'Подключённых устройств пока нет. Настроенный эмулятор будет запущен автоматически.' : 'Устройство пока недоступно. Проверьте подключение и USB-отладку, затем обновите список.'}</span></div>}</div>}
              <div className="detail-actions"><button className="button button-primary" disabled={!!busy[profile.id] || currentStatus.phase === 'stopping' || deviceSaving || favoriteSaving[profile.id]} onClick={() => primaryAction(profile, currentStatus)}><Icon name={busy[profile.id] ? 'refresh' : currentStatus.phase === 'setup' ? 'settings' : isRunning(currentStatus) ? 'external' : 'play'} size={17} className={busy[profile.id] ? 'spin' : ''}/>{primaryLabel(profile,currentStatus)}</button>{profile.kind === 'powershell' && isRunning(currentStatus) && <span className="action-help">Принудительная остановка<br/>PowerShell-сайта отключена.</span>}</div>
              <ReadinessPanel key={profile.id} projectId={profile.id} status={currentStatus} busyAction={busy[profile.id]} onRestart={() => void runAction(profile, 'restart')} onStop={() => void runAction(profile, 'stop')}/>
            </section>
            {profile.kind === 'android' && <section className="detail-panel install-panel"><div><h2>Установка приложения</h2><p>{hasApk ? 'Подготовленный APK устанавливается отдельно от обычного запуска, с сохранением данных.' : 'Добавьте путь к готовому APK в настройках проекта. Для установки нужно готовое конкретное устройство.'}</p></div><button className="button button-secondary" disabled={!profileConfigured || !hasApk || !installDeviceOnline || !!busy[profile.id] || deviceSaving} onClick={() => setInstallTarget(installSerial)}><Icon name="download" size={16}/>Установить APK</button></section>}
          </div><aside className="detail-side"><section className="detail-panel"><div className="panel-icon"><Icon name="folder" size={21}/></div><h2>О проекте</h2><p className="project-description">{blurbs[profile.kind]}</p><TechnologyChips profile={profile}/><dl className="project-metadata"><dt>Расположение</dt><dd className="file-path">{displayPath}</dd>{profile.url && <><dt>Локальный адрес</dt><dd className="mono">{profile.url}</dd></>}<dt>Тип запуска</dt><dd>{{process:'Локальный сервер',android:'ADB · установленное приложение',powershell:'PowerShell · локальный сайт',docker:'Docker Compose · один сервис'}[profile.kind]}</dd><dt>Управление</dt><dd>{currentStatus.owned ? 'Запущен этой сессией Project Shelf' : isRunning(currentStatus) ? 'Внешний процесс' : 'Нет активного запуска'}</dd>{currentStatus.pid && <><dt>Процесс</dt><dd className="mono">PID {currentStatus.pid}</dd></>}{currentStatus.serial && <><dt>Устройство</dt><dd className="mono">{currentStatus.serial}</dd></>}</dl><div className="tag-list">{profile.tags.map(tag => <span key={tag}>{tag}</span>)}</div></section><div className="detail-footnote"><Icon name="info" size={17}/><p>{profile.kind === 'docker' ? 'При остановке завершается только сервис Docker-сервис. Docker Desktop продолжает работать.' : profile.kind === 'powershell' ? 'PowerShell-сайт может записывать файлы. Принудительная остановка отключена, чтобы сохранить результаты работы.' : profile.kind === 'android' ? 'Данные приложения не очищаются. При запуске не переустанавливается APK и не сбрасывается эмулятор.' : 'Останавливаются только процессы, принадлежность которых подтверждена этой сессией.'}</p></div></aside></div>
          <section className="detail-panel log-panel"><div className="section-heading"><div className="heading-with-icon"><Icon name="terminal" size={18}/><h2>Журнал проекта</h2><span className="log-count">{logs.length}</span></div><button className="text-button" disabled={logsBusy} onClick={() => void refreshLogs(profile.id)}><Icon name="refresh" size={15} className={logsBusy ? 'spin' : ''}/>Обновить</button></div>{logError ? <div className="inline-note error-note" role="alert"><Icon name="alert" size={18}/>{logError}</div> : logs.length ? <div className="log-entries" aria-label="Записи журнала">{logs.slice(-100).map((log,i) => <div className={`log-entry log-${log.level}`} key={`${log.timestamp}-${i}`}><time title={log.timestamp}>{new Date(log.timestamp).toLocaleTimeString('ru-RU',{hour:'2-digit',minute:'2-digit',second:'2-digit'})}</time><span className="log-level">{{info:'ИНФО',warn:'ВНИМ.',error:'ОШИБКА'}[log.level]}</span><span>{log.message}</span></div>)}</div> : <div className="empty-logs"><Icon name="terminal" size={24}/><p>{logsBusy ? 'Получаем журнал…' : 'Здесь появится история запуска и диагностика.'}</p></div>}</section>
        </> : <>
          <div className="collection-heading"><div><div className="eyebrow">ВАШЕ МЕСТО ДЛЯ ХОРОШИХ ИДЕЙ</div><h1>{view === 'favorites' ? 'Особенное — рядом.' : view === 'recent' ? 'Продолжим с того же места.' : 'Каждому проекту — своя полка.'}</h1><p>{view === 'favorites' ? 'Любимые инструменты, к которым хочется возвращаться.' : view === 'recent' ? 'Проекты, которые вы запускали в Project Shelf.' : 'Один клик до работы. Всё остальное мы подготовим.'}</p></div><div className="collection-monogram" aria-hidden="true"><span>ps</span><i>✳</i></div></div>
          <div className="collection-toolbar"><div className="collection-title"><h2>{view === 'favorites' ? 'Избранное' : view === 'recent' ? 'Недавние проекты' : 'Моя коллекция'}</h2><span className="collection-count">{filtered.length}</span></div><div className="search-field"><Icon name="search" size={17}/><input ref={searchInput} aria-label="Найти проект" placeholder="Найти проект…" value={query} onChange={e => setQuery(e.target.value)}/>{query ? <button className="search-clear" aria-label="Очистить поиск" onClick={() => setQuery('')}><Icon name="close" size={14}/></button> : <kbd>Ctrl K</kbd>}</div></div>
          {loadError ? <div className="empty-state" role="alert"><Icon name="alert" size={32}/><h2>Коллекция пока недоступна</h2><p>{loadError}</p><button className="button button-primary" onClick={() => void load()}>Попробовать снова</button></div> : loading ? <div className="project-grid" aria-label="Загружаем проекты" aria-busy="true">{[0,1,2,3].map(item => <div className="skeleton-card" key={item}><div/><span/><span/></div>)}</div> : filtered.length ? <div className="project-grid">{filtered.map(p => { const s=statusFor(p.id); return <article key={p.id} className={`project-card ${s.phase === 'starting' ? 'card-starting' : ''}`}><button className="cover-link" onClick={() => setSelected(p.id)} aria-label={`Открыть страницу ${p.name}`}><ProjectCover profile={p}/><span className="cover-kind">{kindLabels[p.kind]}</span><span className="cover-open"><Icon name="arrow" size={19}/></span></button><div className="card-body"><div className="card-title-row"><div><h3><button onClick={() => setSelected(p.id)}>{p.name}</button></h3><p>{p.subtitle}</p></div><button className={`favorite-button ${p.favorite ? 'is-favorite' : ''}`} aria-label={p.favorite ? `Убрать ${p.name} из избранного` : `Добавить ${p.name} в избранное`} disabled={favoriteSaving[p.id] || (p.kind === 'android' && deviceSaving)} aria-pressed={p.favorite} onClick={() => void toggleFavorite(p)}><Icon name="star" size={19}/></button></div><TechnologyChips profile={p}/><div className="card-bottom"><StatusPill status={s}/><button className={`button card-primary ${isRunning(s) ? 'button-open' : ''}`} disabled={s.phase === 'stopping' || (busy[p.id] === 'open') || favoriteSaving[p.id] || (p.kind === 'android' && deviceSaving)} onClick={() => primaryAction(p,s)}><Icon name={s.phase === 'starting' || s.phase === 'stopping' || busy[p.id] === 'open' ? 'refresh' : isRunning(s) ? 'arrow' : s.phase === 'setup' ? 'settings' : s.phase === 'unavailable' ? 'info' : 'play'} size={15} className={s.phase === 'starting' || s.phase === 'stopping' || busy[p.id] === 'open' ? 'spin' : ''}/>{primaryLabel(p,s)}</button></div></div></article>; })}</div> : <div className="empty-state"><Icon name={query ? 'search' : view === 'favorites' ? 'star' : 'clock'} size={34}/><h2>{query ? 'Такого проекта на полке нет' : view === 'favorites' ? 'Здесь будут ваши любимые проекты' : 'Всё начинается с первого запуска'}</h2><p>{query ? 'Попробуйте другое название или очистите поиск.' : view === 'favorites' ? 'Нажмите на звёздочку на карточке проекта.' : 'Запустите любой проект — и он появится в недавних.'}</p><button className="button button-secondary" onClick={() => { setQuery(''); setView('all'); }}>Все проекты<Icon name="arrow" size={16}/></button></div>}
          <footer className="collection-footer"><span><span className={`small-dot ${connectionError ? 'dot-error' : 'dot-green'}`}/>{connectionError ? 'Нет связи с диспетчером' : isDemo ? 'Демонстрационная коллекция' : 'Все проекты на вашем компьютере'}</span><span>{runningCount > 0 ? `Сейчас работают: ${runningCount}` : 'Готовы, когда вы готовы'}<span className="footer-spark">✳</span></span></footer>
        </>}
        {connectionError && <div className="connection-warning" role="status"><Icon name="alert" size={17}/><span>Не удалось обновить состояния: {connectionError}</span><button className="text-button" onClick={() => void refreshStatuses()}>Повторить</button></div>}
      </main>
    </div>
    {setupProfile && <ProjectSetup key={setupProfile.id} profile={setupProfile} locked={runtimeLocked(setupProfile.id)} onClose={closeSetup} onSaved={projectSaved}/>}
    {toast && <div key={toast.id} className={`toast ${toast.error ? 'toast-error' : ''}`} role={toast.error ? 'alert' : 'status'}><Icon name={toast.error ? 'alert' : 'check'} size={19}/><span>{toast.message}</span><button className="icon-button" aria-label="Скрыть уведомление" onClick={() => setToast(null)}><Icon name="close" size={16}/></button></div>}
    {settingsOpen && <Modal title="Немного настроек" onClose={closeSettings}><p className="modal-intro">Project Shelf подстраивается под ваш ритм работы.</p><label className="setting-row"><span><strong>Открывать сайт после запуска</strong><small>Браузер откроется, когда проект подтвердит готовность.</small></span><input type="checkbox" className="switch-input" checked={settings.autoOpenBrowser} disabled={settingsBusy} onChange={e => void saveSetting('autoOpenBrowser',e.target.checked)}/><span className="switch" aria-hidden="true"/></label><label className="setting-row"><span><strong>При закрытии скрывать в трей</strong><small>Крестик скроет окно после подтверждения. Пункт «Выйти» полностью завершает Project Shelf.</small></span><input type="checkbox" className="switch-input" checked={settings.closeToTray} disabled={settingsBusy} onChange={e => void saveSetting('closeToTray',e.target.checked)}/><span className="switch" aria-hidden="true"/></label><div className="settings-note"><Icon name="info" size={18}/><span>Запущенные проекты продолжают работать, пока вы явно не остановите их.</span></div><div className="modal-footer"><button className="button button-primary" onClick={closeSettings}>Готово<Icon name="check" size={16}/></button></div></Modal>}
    {closeIntent && <Modal title={quitResult ? 'Некоторые проекты остаются работать' : closeIntent === 'close' && settings.closeToTray ? 'Убрать Project Shelf в трей?' : 'Завершить Project Shelf?'} onClose={closeModal} className="exit-modal"><p className="modal-intro">{quitResult ? 'Безопасная остановка завершена не для всех проектов. Выберите, что делать дальше.' : closeIntent === 'close' && settings.closeToTray ? 'Окно скроется. Вы сможете вернуться к нему через значок рядом с часами.' : 'Приложение закроется полностью. Запущенные проекты можно оставить работать.'}</p>{quitResult ? <div className="exit-results">{quitResult.retained.length > 0 && <div><strong>Продолжают работать</strong>{quitResult.retained.map(item => <p key={item}>{profiles.find(p=>p.id===item)?.name || item}</p>)}</div>}{quitResult.failed.length > 0 && <div className="field-error"><strong>Не удалось остановить</strong>{quitResult.failed.map(item => <p key={item}>{item}</p>)}</div>}</div> : <><div className="exit-status"><span className="status-dot"/><span>{managed.length ? `Управляемых запусков: ${managed.length}` : 'Нет активных управляемых запусков'}</span></div>{powershellRetained && <div className="inline-note"><Icon name="info" size={18}/><span>PowerShell-сайт продолжит работу: безопасная остановка для него недоступна. После попытки остановить остальные проекты мы покажем результат.</span></div>}</>}<div className="exit-options"><button className="exit-option recommended" data-autofocus onClick={() => void leave(false)} disabled={exitBusy}><span className="option-icon"><Icon name={closeIntent === 'close' && settings.closeToTray && !quitResult ? 'tray' : 'power'} size={21}/></span><span><strong>{quitResult ? 'Оставить остальные и выйти' : 'Оставить проекты работать'}</strong><small>{closeIntent === 'close' && settings.closeToTray && !quitResult ? 'Скрыть окно в системный трей' : 'Полностью выйти из Project Shelf'}</small></span><Icon name="arrow" size={18}/></button>{!quitResult && <button className="exit-option" onClick={() => void leave(true)} disabled={exitBusy}><span className="option-icon"><Icon name="stop" size={20}/></span><span><strong>Остановить управляемые и выйти</strong><small>{powershellRetained ? 'PowerShell-сайт потребует отдельного решения' : 'Только запуски этой сессии Project Shelf'}</small></span><Icon name="arrow" size={18}/></button>}</div><div className="modal-footer"><button className="button button-secondary" disabled={exitBusy} onClick={closeModal}>Отмена</button>{exitBusy && <span className="muted working-label"><Icon name="refresh" size={16} className="spin"/>Подождите…</span>}</div></Modal>}
    {installTarget && profile && <Modal title="Установить APK приложения?" onClose={closeInstall}><p className="modal-intro">Подготовленный APK будет установлен на <strong>{installTarget}</strong>. Установщик использует обновление приложения с сохранением данных: без удаления приложения и очистки хранилища.</p><div className="inline-note"><Icon name="info" size={18}/><span>Сборка не запускается. Если подпись APK отличается от установленной версии, операция завершится ошибкой — данные не удаляются для её обхода.</span></div><div className="modal-footer"><button className="button button-secondary" onClick={closeInstall}>Отмена</button><button className="button button-primary" onClick={() => { setInstallTarget(null); void runAction(profile,'install',installTarget); }}><Icon name="download" size={16}/>Установить с сохранением данных</button></div></Modal>}
  </div>;
}

