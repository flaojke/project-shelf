import { useEffect, useRef, useState } from 'react';
import type { FormEvent, ReactNode } from 'react';
import { api } from '../lib/api';
import type { Profile } from '../lib/types';
import { Icon } from './Icons';

export interface ValidationResult { complete: boolean; issues: string[] }
interface Props { profile: Profile; locked: boolean; onClose: () => void; onSaved: (profile: Profile, validation: ValidationResult) => void }
type Fields = Record<string, string>;
const titles = { process: 'Настройка веб-проекта', powershell: 'Настройка PowerShell-сайта', docker: 'Настройка Docker-сервиса', android: 'Настройка Android-приложения' };
const read = (value: unknown) => typeof value === 'string' || typeof value === 'number' ? String(value) : '';

function initialFields(profile: Profile): Fields {
  const c = profile.config;
  return { path: profile.path, nodePath: read(c.nodePath), port: read(c.port) || (profile.kind === 'powershell' ? '8877' : '5180'), readyMarker: read(c.readyMarker), service: read(c.service), composeFile: read(c.composeFile), dockerPath: read(c.dockerPath), healthMessage: read(c.healthMessage), url: profile.url || 'http://127.0.0.1:8080/', sdkRoot: read(c.sdkRoot), avd: read(c.avd), avdHome: read(c.avdHome), androidUserHome: read(c.androidUserHome), serial: read(c.serial), packageName: read(c.packageName), activity: read(c.activity), apkPath: read(c.apkPath) };
}

function draftProfile(profile: Profile, fields: Fields): Profile {
  const s = (name: string) => (fields[name] || '').trim();
  const config: Record<string, unknown> = {};
  let path = '', url: string | undefined;
  const optional = (name: string) => { if (s(name)) config[name] = s(name); };
  if (profile.kind === 'process' || profile.kind === 'powershell') {
    path = s('path');
    const port = Number(s('port') || (profile.kind === 'powershell' ? '8877' : '5180'));
    config.port = port;
    url = `http://127.0.0.1:${port}/`;
    if (profile.kind === 'process') { optional('nodePath'); config.readyMarker = s('readyMarker'); }
    else { config.service = s('service'); config.allowStop = false; }
  } else if (profile.kind === 'docker') {
    config.composeFile = s('composeFile'); config.service = s('service'); config.healthMessage = s('healthMessage'); optional('dockerPath'); url = s('url');
  } else {
    config.sdkRoot = s('sdkRoot'); config.avd = s('avd'); config.packageName = s('packageName'); config.activity = s('activity');
    ['avdHome', 'androidUserHome', 'serial', 'apkPath'].forEach(optional);
  }
  // Runtime configuration is rebuilt from these structured fields. No hidden command or arguments are forwarded.
  return { ...profile, path, url, config };
}

function localIssues(profile: Profile, fields: Fields): string[] {
  const missing: string[] = [];
  const needs = (name: string, label: string) => { if (!fields[name]?.trim()) missing.push(`Укажите ${label}.`); };
  if (profile.kind === 'process' || profile.kind === 'powershell') {
    needs('path', 'папку проекта');
    const n = Number(fields.port || (profile.kind === 'powershell' ? 8877 : 5180));
    if (!Number.isInteger(n) || n < 1024 || n > 65535) missing.push('Порт должен быть целым числом от 1024 до 65535.');
    if (profile.kind === 'process') needs('readyMarker', 'маркер готовности');
    else needs('service', 'идентификатор health-сервиса');
  } else if (profile.kind === 'docker') {
    needs('composeFile', 'compose-файл'); needs('service', 'имя сервиса'); needs('healthMessage', 'сообщение готовности'); needs('url', 'локальный адрес');
  } else {
    needs('sdkRoot', 'папку Android SDK'); needs('packageName', 'package name'); needs('activity', 'activity');
    if (!fields.serial?.trim()) needs('avd', 'имя эмулятора AVD или конкретный serial устройства');
  }
  return missing;
}

function Field({ label, help, optional, children }: { label: string; help?: string; optional?: boolean; children: ReactNode }) {
  return <label className="setup-field"><span className="setup-label">{label}{optional && <span className="optional-label">необязательно</span>}</span>{children}{help && <span className="setup-field-help">{help}</span>}</label>;
}

export function ProjectSetup({ profile, locked, onClose, onSaved }: Props) {
  const [fields, setFields] = useState<Fields>(() => initialFields(profile));
  const [validation, setValidation] = useState<ValidationResult>(() => ({ complete: false, issues: localIssues(profile, initialFields(profile)) }));
  const [validating, setValidating] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [saved, setSaved] = useState(false);
  const dialog = useRef<HTMLDivElement>(null);
  const request = useRef(0);
  const dismiss = useRef(onClose); dismiss.current = onClose;
  const busy = useRef(false); busy.current = saving;
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    dialog.current?.querySelector<HTMLInputElement>('input')?.focus();
    const key = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !busy.current) { event.preventDefault(); dismiss.current(); }
      if (event.key === 'Tab') {
        const items = Array.from(dialog.current?.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), [tabindex="0"]') || []);
        if (event.shiftKey && document.activeElement === items[0]) { event.preventDefault(); items[items.length - 1]?.focus(); }
        else if (!event.shiftKey && document.activeElement === items[items.length - 1]) { event.preventDefault(); items[0]?.focus(); }
      }
    };
    document.addEventListener('keydown', key);
    return () => { document.removeEventListener('keydown', key); previous?.focus(); };
  }, []);
  useEffect(() => {
    const current = ++request.current;
    const issues = localIssues(profile, fields);
    setValidation({ complete: false, issues }); setValidating(true);
    const timer = window.setTimeout(() => {
      void api.profileValidation(draftProfile(profile, fields)).then(result => { if (current === request.current) setValidation(result); }).catch(reason => { if (current === request.current) setValidation({ complete: false, issues: [...issues, reason instanceof Error ? reason.message : String(reason)] }); }).finally(() => { if (current === request.current) setValidating(false); });
    }, 240);
    return () => clearTimeout(timer);
  }, [fields, profile]);

  function input(name: string, placeholder: string, type = 'text') {
    return <input name={name} type={type} autoComplete="off" spellCheck={false} value={fields[name] || ''} placeholder={placeholder} min={type === 'number' ? 1024 : undefined} max={type === 'number' ? 65535 : undefined} step={type === 'number' ? 1 : undefined} onChange={event => { setFields(previous => ({ ...previous, [name]: event.target.value })); setSaved(false); setError(''); }} />;
  }
  async function save(event: FormEvent) {
    event.preventDefault(); if (locked || saving) return;
    setSaving(true); setError('');
    try {
      const draft = draftProfile(profile, fields);
      const result = await api.profileValidation(draft);
      setValidation(result);
      await api.saveProfile(draft);
      setSaved(true); onSaved(draft, result);
      if (result.complete) dismiss.current();
    } catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)); }
    finally { setSaving(false); }
  }
  return <div className="modal-backdrop setup-backdrop" onMouseDown={event => { if (event.target === event.currentTarget && !saving) onClose(); }}>
    <div ref={dialog} className="modal setup-modal" role="dialog" aria-modal="true" aria-label={titles[profile.kind]}>
      <div className="modal-heading"><div><div className="eyebrow">ВАШ ПРОЕКТ · ВАШИ НАСТРОЙКИ</div><h2>{titles[profile.kind]}</h2></div><button className="icon-button" aria-label="Закрыть настройки проекта" disabled={saving} onClick={onClose}><Icon name="close"/></button></div>
      <p className="modal-intro">Заполните данные для запуска. Можно сохранить часть настроек и вернуться позже — до полной настройки Project Shelf ничего не запускает.</p>
      <form noValidate onSubmit={event => void save(event)}>
        <fieldset className="setup-fields" disabled={saving || locked}>
          {profile.kind === 'process' && <>
            <Field label="Папка веб-проекта" help="Проект на Vite с уже установленными зависимостями. Сборка и установка пакетов не выполняются.">{input('path', 'Путь к папке проекта')}</Field>
            <Field label="Node.js" optional help="Оставьте пустым, чтобы использовать node.exe из PATH.">{input('nodePath', 'Путь к node.exe или PATH')}</Field>
            <div className="setup-two-columns"><Field label="Локальный порт">{input('port', '5180', 'number')}</Field><Field label="Маркер готовности" help="Уникальный текст в HTML главной страницы нужного сайта.">{input('readyMarker', 'Например: web-project')}</Field></div>
            <div className="setup-contract"><Icon name="info" size={17}/><span>Сайт будет доступен только на 127.0.0.1. Перед открытием проверяются маркер готовности, папка проекта и порт.</span></div>
          </>}
          {profile.kind === 'powershell' && <>
            <Field label="Папка сайта" help="В этой папке должен находиться start-site.ps1. Произвольные команды не выполняются.">{input('path', 'Путь к папке со start-site.ps1')}</Field>
            <div className="setup-two-columns"><Field label="Локальный порт">{input('port', '8877', 'number')}</Field><Field label="Идентификатор сервиса" help="Значение service в /api/health.">{input('service', 'Например: local-site')}</Field></div>
            <div className="setup-contract"><Icon name="info" size={17}/><span>Нужен совместимый /api/health: service, root, port, ready, pid. Принудительная остановка сайта отключена, чтобы не прервать запись файлов.</span></div>
          </>}
          {profile.kind === 'docker' && <>
            <Field label="Docker Compose файл">{input('composeFile', 'Полный путь к compose.yaml')}</Field>
            <div className="setup-two-columns"><Field label="Имя сервиса" help="Ключ сервиса из services в compose-файле.">{input('service', 'Например: web')}</Field><Field label="Сообщение готовности" help="Значение message в JSON на /health.">{input('healthMessage', 'Например: app is alive')}</Field></div>
            <Field label="Локальный адрес" help="Только HTTP на localhost или 127.0.0.1, без пути, параметров и пароля.">{input('url', 'http://127.0.0.1:8080/')}</Field>
            <Field label="Docker CLI" optional help="Оставьте пустым, чтобы использовать docker.exe из PATH.">{input('dockerPath', 'Путь к docker.exe или PATH')}</Field>
            <div className="setup-contract"><Icon name="info" size={17}/><span>Запускается только указанный сервис из подготовленной конфигурации. Образы не скачиваются, сборка и удаление томов не выполняются.</span></div>
          </>}
          {profile.kind === 'android' && <>
            <Field label="Android SDK" help="Папка с platform-tools и emulator.">{input('sdkRoot', 'Путь к Android SDK')}</Field>
            <div className="setup-two-columns"><Field label="Имя эмулятора AVD" help="Нужно для автоматического запуска, если serial не указан.">{input('avd', 'Имя подготовленного AVD')}</Field><Field label="Serial устройства" optional help="Для конкретного телефона или уже работающего эмулятора.">{input('serial', 'Точный serial из ADB')}</Field></div>
            <div className="setup-two-columns"><Field label="Package name">{input('packageName', 'com.example.app')}</Field><Field label="Activity">{input('activity', '.MainActivity')}</Field></div>
            <div className="setup-two-columns"><Field label="Папка AVD" optional help="ANDROID_AVD_HOME, если отличается от стандартной.">{input('avdHome', 'Путь к каталогу AVD')}</Field><Field label="Домашняя папка Android" optional help="ANDROID_USER_HOME, если отличается от стандартной.">{input('androidUserHome', 'Путь к Android user home')}</Field></div>
            <Field label="Готовый APK" optional help="Нужен только для отдельного действия «Установить APK».">{input('apkPath', 'Полный путь к APK')}</Field>
            <div className="setup-contract"><Icon name="info" size={17}/><span>Обычный запуск открывает установленное приложение. Если выбран AVD, Project Shelf дождётся его загрузки. Сборка, удаление приложения и очистка данных не выполняются.</span></div>
          </>}
        </fieldset>
        {locked && <div className="inline-note" role="status"><Icon name="info" size={17}/><span>Параметры запуска нельзя менять, пока этот проект запускается, работает или останавливается под управлением Project Shelf. Избранное остаётся доступно.</span></div>}
        <div className={`setup-validation ${validation.complete ? 'validation-complete' : ''}`} aria-live="polite"><div className="setup-validation-heading"><Icon name={validating ? 'refresh' : validation.complete ? 'check' : 'info'} size={16} className={validating ? 'spin' : ''}/><strong>{validating ? 'Проверяем настройки…' : validation.complete ? 'Настройки готовы к запуску' : 'Пока требуется настройка'}</strong></div>{!validation.complete && validation.issues.length > 0 && <ul>{validation.issues.map((issue, index) => <li key={`${index}-${issue}`}>{issue}</li>)}</ul>}</div>
        {saved && <div className="setup-saved" role="status"><Icon name="check" size={17}/><span>Настройка сохранена. Заполните оставшиеся поля, когда будете готовы; проект пока не запускается.</span></div>}
        {error && <div className="inline-note error-note" role="alert"><Icon name="alert" size={17}/><span>{error}</span></div>}
        <div className="modal-footer"><span className="setup-save-hint">Все настройки хранятся на этом компьютере.</span><button type="button" className="button button-secondary" disabled={saving} onClick={onClose}>{saved ? 'Закрыть' : 'Отмена'}</button><button type="submit" className="button button-primary" disabled={saving || locked}><Icon name={saving ? 'refresh' : 'check'} size={16} className={saving ? 'spin' : ''}/>{saving ? 'Сохраняем…' : 'Сохранить'}</button></div>
      </form>
    </div>
  </div>;
}
