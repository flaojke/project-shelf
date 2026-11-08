import test from 'node:test';
import assert from 'node:assert/strict';

const pending = [], storage = new Map();
globalThis.localStorage = { getItem: key => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) };
globalThis.window = { setTimeout: callback => pending.push(callback), dispatchEvent() {} };
const { api, isDemo } = await import('../src/lib/api.ts');

test('preview diagnostics never invent measured success or expose destructive capabilities', async () => {
  assert.equal(isDemo, true);
  for (const profile of await api.listProfiles()) {
    const report = await api.projectDiagnostics(profile.id);
    assert.equal(report.id, profile.id);
    assert.equal(report.canStop, false);
    assert.equal(report.canRestart, false);
    assert.deepEqual(report.checks.map(c => c.id), ['dependencies', 'process', 'http', 'port']);
    for (const check of report.checks) {
      assert.equal(check.state, profile.kind === 'android' && ['http', 'port'].includes(check.id) ? 'notApplicable' : 'unknown');
      assert.equal(check.httpStatus, undefined);
    }
  }
});

test('simulated ready never becomes proof of HTTP readiness', async () => {
  const web = (await api.listProfiles()).find(p => p.kind === 'process');
  if (api.profileValidation) {
    web.path = 'C:\\Fixtures\\Web';
    web.config = { port: 5180, readyMarker: 'fixture-ready' };
    await api.saveProfile(web);
  }
  await api.startProject(web.id);
  pending.shift()();
  assert.equal((await api.getStatuses()).find(s => s.id === web.id).phase, 'ready');
  const report = await api.projectDiagnostics(web.id);
  const http = report.checks.find(c => c.id === 'http');
  assert.equal(http.state, 'unknown');
  assert.equal(http.httpStatus, undefined);
  await assert.rejects(api.restartProject(web.id), /предпросмотре/);
});

test('unknown diagnostics profile is rejected without scheduling work', async () => {
  const before = pending.length;
  await assert.rejects(api.projectDiagnostics('not-a-profile'), /Неизвестный профиль/);
  assert.equal(pending.length, before);
});

test('native diagnostics and restart use explicit IPC ids and preserve backend errors', async () => {
  const calls = [];
  window.__TAURI_INTERNALS__ = { invoke: async (command, args) => {
    calls.push({ command, args });
    throw new Error(command === 'project_diagnostics' ? 'diagnostic unavailable' : 'graceful stop refused');
  } };
  const native = await import('../src/lib/api.ts?readiness-native-contract');
  assert.equal(native.isDemo, false);
  await assert.rejects(native.api.projectDiagnostics('fixture-project'), /diagnostic unavailable/);
  await assert.rejects(native.api.restartProject('fixture-project'), /graceful stop refused/);
  assert.deepEqual(calls, [
    { command: 'project_diagnostics', args: { id: 'fixture-project' } },
    { command: 'restart_project', args: { id: 'fixture-project' } },
  ]);
});
