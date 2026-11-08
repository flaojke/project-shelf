import type { Profile, ProfileValidation } from './types';

const allowed: Record<string, string> = { web: 'process', powershell: 'powershell', docker: 'docker', android: 'android' };
const configKeys: Record<string, string[]> = {
  process: ['nodePath', 'port', 'readyMarker'],
  powershell: ['port', 'service', 'allowStop'],
  docker: ['dockerPath', 'composeFile', 'service', 'healthMessage'],
  android: ['sdkRoot', 'avd', 'avdHome', 'androidUserHome', 'serial', 'packageName', 'activity', 'apkPath'],
};
const absolute = (value: string) => value.length <= 2048 && /^[A-Za-z]:[\\/]/.test(value) && !/[\p{Cc}<>"|?*]/u.test(value) && !value.slice(2).includes(':') && !value.split(/[\\/]/).some(part => part === '.' || part === '..' || /[. ]$/.test(part));
const javaName = (value: string) => /^[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z_][A-Za-z0-9_]*)*$/.test(value);

// Browser preview validation has no filesystem or system access. Native mode
// always asks the Rust validator instead; it is the execution authority.
export function validateDemoProfile(profile: Profile): ProfileValidation {
  const issues: string[] = [];
  if (allowed[profile.id] !== profile.kind) issues.push('Неизвестный тип или ID профиля.');
  const c = profile.config || {};
  if (!c || typeof c !== 'object' || Array.isArray(c)) return { complete: false, issues: ['Нужен config JSON-объект.'] };
  const text = (key: string) => typeof c[key] === 'string' ? c[key].trim() : '';
  for (const key of Object.keys(c)) if (!(configKeys[profile.kind] || []).includes(key)) issues.push('Недопустимое поле конфигурации.');
  for (const [key, value] of Object.entries(c)) {
    if (key === 'port') { if (typeof value !== 'number') issues.push('Порт должен быть числом.'); }
    else if (key === 'allowStop') { if (typeof value !== 'boolean') issues.push('allowStop должен быть логическим значением.'); }
    else if (value !== null && typeof value !== 'string') issues.push(`Поле ${key} должно быть строкой.`);
  }
  function required(key: string, label: string) { if (!text(key)) issues.push(`Укажите: ${label}.`); }
  function localPath(value: string, label: string, needed = true) {
    if (!value) { if (needed) issues.push(`Укажите: ${label}.`); }
    else if (!absolute(value)) issues.push(`${label}: нужен абсолютный локальный путь Windows без '..'.`);
  }
  function executable(value: string, name: string) {
    if (value && value !== name && value !== `${name}.exe` && (!absolute(value) || value.split(/[\\/]/).at(-1)?.toLowerCase() !== `${name}.exe`)) issues.push(`Выберите ${name}.exe или оставьте поле пустым для PATH.`);
  }
  if (profile.path && profile.kind !== 'process' && profile.kind !== 'powershell') localPath(profile.path, 'Папка проекта', false);
  if (profile.kind === 'process' || profile.kind === 'powershell') {
    localPath(profile.path, 'Папка проекта');
    const port = c.port ?? (profile.kind === 'powershell' ? 8877 : 5180);
    if (typeof port !== 'number' || !Number.isInteger(port) || port < 1024 || port > 65535) issues.push('Порт должен быть целым числом от 1024 до 65535.');
    if (profile.kind === 'process') { required('readyMarker', 'Маркер готовности в HTML'); executable(text('nodePath'), 'node'); }
    else { required('service', 'Имя сервиса health'); if (c.allowStop === true) issues.push('Принудительная остановка PowerShell-сайта отключена.'); }
  }
  if (profile.kind === 'docker') {
    localPath(text('composeFile'), 'Compose-файл'); required('service', 'Имя Docker Compose сервиса'); required('healthMessage', 'Ожидаемое сообщение health'); executable(text('dockerPath'), 'docker');
    if (text('composeFile') && !/\.ya?ml$/i.test(text('composeFile'))) issues.push('Compose-файл должен иметь расширение yaml или yml.');
    if (text('service') && (text('service').length > 120 || !/^[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(text('service')))) issues.push('Некорректное имя Docker Compose сервиса.');
    if (text('healthMessage').length > 128 || /[\x00-\x1f\x7f]/.test(text('healthMessage'))) issues.push('Некорректное сообщение health.');
  }
  if (profile.kind !== 'android') {
    try {
      const url = new URL(profile.url || '');
      if (url.protocol !== 'http:' || !['localhost', '127.0.0.1'].includes(url.hostname) || url.username || url.password || url.search || url.hash || url.pathname !== '/') throw new Error();
      if (profile.kind !== 'docker' && Number(url.port || 80) !== (c.port ?? (profile.kind === 'powershell' ? 8877 : 5180))) issues.push('Порт локального адреса должен совпадать с настройкой порта.');
    } catch { issues.push('Нужен локальный HTTP-адрес localhost или 127.0.0.1 без пароля, query и fragment.'); }
  } else {
    localPath(text('sdkRoot'), 'Android SDK'); localPath(text('avdHome'), 'Каталог AVD', false); localPath(text('androidUserHome'), 'Каталог Android user', false); localPath(text('apkPath'), 'APK', false);
    required('packageName', 'Android package'); required('activity', 'Activity');
    if (!text('serial') && !text('avd')) issues.push('Укажите AVD или явно выбранный serial устройства.');
    if (text('packageName') && (text('packageName').length > 255 || !text('packageName').includes('.') || !javaName(text('packageName')))) issues.push('Некорректное имя Android package.');
    if (text('activity') && (text('activity').length > 255 || !javaName(text('activity').replace(/^\./, '')))) issues.push('Некорректная Android Activity.');
    if (text('avd') && (text('avd').length > 128 || !/^[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(text('avd')))) issues.push('Некорректное имя AVD.');
    if (text('serial') && (text('serial').length > 128 || !/^[A-Za-z0-9][A-Za-z0-9._:-]*$/.test(text('serial')))) issues.push('Некорректный Android serial.');
    if (text('apkPath') && !/\.apk$/i.test(text('apkPath'))) issues.push('Нужен файл с расширением apk.');
  }
  return { complete: issues.length === 0, issues: [...new Set(issues)] };
}
