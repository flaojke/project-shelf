import test from 'node:test';
import assert from 'node:assert/strict';
const storage = new Map(), pending = [];
globalThis.localStorage = { getItem: key => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) };
globalThis.window = { setTimeout: callback => pending.push(callback), dispatchEvent: () => {} };
const { api, isDemo } = await import('../src/lib/api.ts');
const { validateDemoProfile } = await import('../src/lib/validation.ts');
const configuredWeb = async () => ({ ...(await api.listProfiles()).find(p => p.id === 'web'), path: 'C:\\Projects\\ExampleWeb', config: { nodePath: '', port: 5180, readyMarker: 'Example Web' } });

test('fresh public preview has four safe setup profiles with no personal defaults', async () => {
  assert.equal(isDemo, true);
  const profiles = await api.listProfiles();
  assert.deepEqual(profiles.map(p => p.id), ['web', 'powershell', 'docker', 'android']);
  for (const p of profiles) {
    assert.equal(p.path, '');
    assert.equal((await api.profileValidation(p)).complete, false);
  }
  assert.equal((await api.getStatuses()).every(s => s.phase === 'setup' && !s.owned), true);
  assert.equal((await api.getSettings()).liveActionsEnabled, false);
});

test('incomplete start/open/install and device lookup never schedule execution', async () => {
  await assert.rejects(api.startProject('web'), /настройте/);
  await assert.rejects(api.openProject('web'), /настройте/);
  await assert.rejects(api.installAndroidApk('android', 'emulator-5554'), /настройте/);
  await assert.rejects(async () => api.androidDevices(), /настройте/);
  assert.equal(pending.length, 0);
  assert.deepEqual(await api.projectLogs('web'), []);
});

test('safe partial configuration can be saved without launching anything', async () => {
  const p = (await api.listProfiles()).find(p => p.id === 'web');
  p.path = 'C:\\Projects\\ExampleWeb';
  await api.saveProfile(p);
  assert.equal((await api.getStatuses()).find(s => s.id === p.id).phase, 'setup');
  assert.equal(pending.length, 0);
  assert.equal((await api.profileValidation(p)).complete, false);
});

test('configured Start is idempotent and uses a clearly simulated lifecycle', async () => {
  const p = await configuredWeb(); await api.saveProfile(p);
  assert.equal((await api.profileValidation(p)).complete, true);
  assert.equal((await api.getStatuses()).find(s => s.id === p.id).phase, 'stopped');
  const first = await api.startProject(p.id), second = await api.startProject(p.id);
  assert.deepEqual(first, second); assert.equal(first.phase, 'starting'); assert.equal(pending.length, 1);
  pending.shift()();
  assert.equal((await api.getStatuses()).find(s => s.id === p.id).phase, 'ready');
});

test('owned runtime configuration is locked but favorites remain editable', async () => {
  const p = (await api.listProfiles()).find(p => p.id === 'web');
  await assert.rejects(api.saveProfile({ ...p, path: 'C:\\Projects\\AnotherWeb' }), /остановите/);
  await api.saveProfile({ ...p, favorite: !p.favorite });
  assert.equal((await api.getStatuses()).find(s => s.id === p.id).phase, 'ready');
});

test('leave retains running preview resources; stop-owned closes only owned resources', async () => {
  assert.equal((await api.quitApp(false)).managed.some(s => s.id === 'web'), true);
  assert.equal((await api.getStatuses()).find(s => s.id === 'web').phase, 'ready');
  await api.quitApp(true);
  assert.equal((await api.getStatuses()).find(s => s.id === 'web').phase, 'stopped');
  await assert.rejects(api.stopProject('web'), /вне текущего/);
  await assert.rejects(api.stopProject('powershell'), /не поддерживается/);
});

test('fixed IDs/kinds cannot be replaced with a fifth adapter', async () => {
  const p = await configuredWeb();
  await assert.rejects(api.saveProfile({ ...p, id: 'extra' }), /Неизвестный/);
  await assert.rejects(api.saveProfile({ ...p, kind: 'docker' }), /Тип профиля/);
});

test('validator rejects unsafe structured fields and nonlocal URL', async () => {
  const p = await configuredWeb();
  for (const candidate of [
    { ...p, path: '\\\\server\\share' },
    { ...p, path: 'C:\\Projects\\..\\other' },
    { ...p, url: 'https://example.invalid/' },
    { ...p, url: 'http://127.0.0.1:5180/?q=value' },
    { ...p, config: { ...p.config, shell: 'example' } },
    { ...p, config: { ...p.config, nodePath: 'cmd.exe' } },
    { ...p, config: { ...p.config, port: 70000 } },
  ]) assert.equal(validateDemoProfile(candidate).complete, false);
});

test('Android needs explicit SDK, target, package and activity; APK is separate', async () => {
  const p = (await api.listProfiles()).find(p => p.id === 'android');
  const configured = { ...p, config: { sdkRoot: 'C:\\Android\\Sdk', avd: 'ExampleDevice', packageName: 'com.example.app', activity: '.MainActivity' } };
  assert.equal(validateDemoProfile(configured).complete, true);
  await api.saveProfile(configured);
  await assert.rejects(api.installAndroidApk(p.id, 'emulator-5554'), /APK/);
  assert.equal((await api.getStatuses()).find(s => s.id === p.id).phase, 'stopped');
  assert.equal(pending.length, 0);
});

test('all four structured adapters accept neutral complete configurations', async () => {
  const profiles = await api.listProfiles();
  const ps = profiles.find(p => p.id === 'powershell');
  const docker = profiles.find(p => p.id === 'docker');
  const android = profiles.find(p => p.id === 'android');
  const candidates = [
    await configuredWeb(),
    { ...ps, path: 'C:\\Projects\\ExampleSite', config: { service: 'example-site', allowStop: false } },
    { ...docker, config: { composeFile: 'C:\\Projects\\ExampleService\\compose.yaml', service: 'web', healthMessage: 'example-ready' } },
    { ...android, config: { sdkRoot: 'C:\\Android\\Sdk', serial: 'example-device', packageName: 'com.example.app', activity: '.MainActivity' } },
  ];
  for (const candidate of candidates) assert.deepEqual(validateDemoProfile(candidate), { complete: true, issues: [] });
});

test('typed fields and Android target syntax match the native setup contract', async () => {
  const android = (await api.listProfiles()).find(p => p.id === 'android');
  const base = { ...android, config: { sdkRoot: 'C:\\Android\\Sdk', avd: 'ExampleDevice', packageName: 'com.example.app', activity: '.MainActivity' } };
  for (const changes of [{ activity: 'Bad/Activity' }, { avd: 'Bad AVD' }, { serial: '-option' }, { packageName: 'example' }, { sdkRoot: 'C:\\Android\\Sdk:stream' }, { packageName: 42 }]) {
    assert.equal(validateDemoProfile({ ...base, config: { ...base.config, ...changes } }).complete, false);
  }
  const web = await configuredWeb();
  assert.equal(validateDemoProfile({ ...web, config: { ...web.config, port: '5180' } }).complete, false);
});

test('native bridge preserves typed IPC and never falls back to demo on errors', async () => {
  const calls = [];
  window.__TAURI_INTERNALS__ = { invoke: async (command, args) => { calls.push({ command, args }); if (command === 'start_project') throw new Error('setup required'); return { complete: false, issues: ['missing configuration'] }; } };
  const native = await import('../src/lib/api.ts?native-contract');
  assert.equal(native.isDemo, false);
  const p = await configuredWeb();
  assert.equal((await native.api.profileValidation(p)).complete, false);
  await assert.rejects(native.api.startProject('web'), /setup required/);
  assert.equal(calls[0].command, 'profile_validation'); assert.equal(calls[1].command, 'start_project');
  assert.deepEqual(calls[1].args, { id: 'web' });
});
