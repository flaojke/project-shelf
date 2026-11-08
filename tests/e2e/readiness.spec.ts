import { test, expect, type Page } from '@playwright/test';

const ids = {"web":"web","shell":"powershell","docker":"docker","android":"android"};
const shellKind = 'powershell';
const webTechnologies = ["Vite","Node.js"];

async function nativeFixture(page: Page, mode = 'http503') {
  await page.addInitScript(({ ids, shellKind, mode }) => {
    const browser = window as any;
    const names = { web: 'Web Fixture', shell: 'Write-safe Site', docker: 'Container Fixture', android: 'Android Fixture' };
    const profiles = [
      { id: ids.web, name: names.web, kind: 'process', url: 'http://127.0.0.1:5180/', config: { port: 5180, readyMarker: 'fixture-ready' } },
      { id: ids.shell, name: names.shell, kind: shellKind, url: 'http://127.0.0.1:8877/', config: { port: 8877, service: 'fixture-site', allowStop: false } },
      { id: ids.docker, name: names.docker, kind: 'docker', url: 'http://127.0.0.1:8080/', config: { composeFile: 'C:\\Fixtures\\Service\\compose.yaml', service: 'web', healthMessage: 'fixture-ready' } },
      { id: ids.android, name: names.android, kind: 'android', config: { sdkRoot: 'C:\\Fixtures\\Sdk', avd: 'ExampleDevice', serial: 'fixture-device', packageName: 'com.example.app', activity: '.MainActivity' } },
    ].map(profile => ({ ...profile, path: 'C:\\Fixtures\\Project', subtitle: 'Mocked native contract', accent: '#c66145', cover: profile.kind, favorite: false, tags: [] }));
    const state = browser.readinessFixture = { mode, calls: [] as { command: string; id?: string }[], activeDiagnostics: 0, maxDiagnostics: 0 };
    const statuses: Record<string, any> = {
      [ids.web]: { id: ids.web, phase: ['external', 'conflict'].includes(mode) ? mode === 'external' ? 'external' : 'error' : 'ready', owned: !['external', 'conflict'].includes(mode), pid: 4242 },
      [ids.shell]: { id: ids.shell, phase: 'ready', owned: true, pid: 4343 },
      [ids.docker]: { id: ids.docker, phase: 'external', owned: false },
      [ids.android]: { id: ids.android, phase: mode === 'ownedAndroid' ? 'ready' : 'stopped', owned: mode === 'ownedAndroid', serial: mode === 'ownedAndroid' ? 'fixture-device' : undefined },
    };
    const delay = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
    const diagnostics = (id: string) => {
      const status = statuses[id];
      const android = id === ids.android, shell = id === ids.shell, external = !status.owned;
      const conflict = id === ids.web && state.mode === 'conflict';
      const measuredError = id === ids.web && state.mode === 'http503';
      const reason = shell ? 'Safe stop is unavailable because this site may be writing files.' : external ? 'Resource belongs to another process; this session cannot stop it.' : undefined;
      return {
        id, checkedAt: new Date().toISOString(), technologies: android ? ['Android', 'ADB'] : shell ? ['PowerShell', 'HTTP'] : id === ids.docker ? ['Docker', 'Compose'] : ['Vite', 'Node.js'],
        canStop: status.owned && !shell, canRestart: status.owned && !shell,
        stopReason: reason, restartReason: reason,
        checks: [
          { id: 'dependencies', label: 'Зависимости', state: 'ready', message: 'Mock dependency check completed.', value: 'Компоненты найдены' },
          { id: 'process', label: 'Процесс', state: android || conflict ? 'unknown' : 'ready', message: android ? 'Application PID has not been checked.' : external ? 'Внешний процесс; ownership не подтверждён этой сессией.' : 'Owned process identity confirmed by mock.', value: android ? undefined : 'PID 4242' },
          { id: 'http', label: 'HTTP', state: android ? 'notApplicable' : measuredError || conflict ? 'error' : 'ready', message: android ? 'Android does not use HTTP readiness.' : measuredError ? 'Measured HTTP 503 from fixture.' : conflict ? 'Measured HTTP 409 from another service.' : 'Measured HTTP 200 from fixture.', value: android ? undefined : measuredError ? 'HTTP 503' : conflict ? 'HTTP 409' : 'HTTP 200', httpStatus: android ? undefined : measuredError ? 503 : conflict ? 409 : 200 },
          { id: 'port', label: 'Порт', state: android ? 'notApplicable' : conflict ? 'error' : 'ready', message: android ? 'No HTTP port is configured for Android.' : conflict ? 'Порт 5180 занят другим приложением.' : external ? 'Порт занят внешним процессом.' : 'Owned listener verified by mock.', value: android ? undefined : '127.0.0.1:5180' },
        ],
      };
    };
    browser.__TAURI_INTERNALS__ = { invoke: async (command: string, args?: Record<string, unknown>) => {
      const id = typeof args?.id === 'string' ? args.id : undefined;
      state.calls.push({ command, id });
      if (command === 'list_profiles') return structuredClone(profiles);
      if (command === 'get_settings') return { theme: 'light', closeToTray: true, autoOpenBrowser: false, liveActionsEnabled: true };
      if (command === 'profile_validation') return { complete: true, issues: [] };
      if (command === 'get_statuses') return structuredClone(Object.values(statuses));
      if (command === 'android_devices') return [{ serial: 'fixture-device', state: 'device', model: 'Fixture device' }];
      if (command === 'project_logs') {
        if (state.mode === 'staleLogs' && id === ids.web) await delay(1100);
        return [{ timestamp: '2026-01-01T10:00:00Z', level: 'info', message: id === ids.web ? 'WEB event only' : id === ids.docker ? 'DOCKER event only' : 'OTHER event only' }];
      }
      if (command === 'project_diagnostics') {
        state.activeDiagnostics++; state.maxDiagnostics = Math.max(state.maxDiagnostics, state.activeDiagnostics);
        try {
          await delay(state.mode === 'http503' ? 800 : 120);
          if (state.mode === 'diagnosticFailure') throw new Error('Mock diagnostic connection failed');
          return diagnostics(id!);
        } finally { state.activeDiagnostics--; }
      }
      if (command === 'restart_project') {
        await delay(550);
        if (state.mode === 'restartFailure') throw new Error('Mock graceful stop refused; project kept running');
        return structuredClone(statuses[id!]);
      }
      if (command === 'stop_project') {
        await delay(400);
        if (state.mode === 'stopFailure') throw new Error('Mock safe stop refused; resource retained');
        statuses[id!] = { id, phase: 'stopped', owned: false };
        return structuredClone(statuses[id!]);
      }
      if (command === 'start_project' || command === 'open_project') return structuredClone(statuses[id!]);
      throw new Error('Unexpected fixture command: ' + command);
    } };
  }, { ids, shellKind, mode });
  await page.goto('/');
  await expect(page.locator('.project-card')).toHaveCount(4);
}

const readiness = (page: Page) => page.getByRole('region', { name: 'Проверка готовности проекта' });
const check = (page: Page, id: string) => readiness(page).locator('[data-check-id="' + id + '"]');
const calls = (page: Page, command: string) => page.evaluate(command => (window as any).readinessFixture.calls.filter((call: any) => call.command === command).length, command);

test('technology labels and measured HTTP diagnostics do not inherit global ready', async ({ page }) => {
  await nativeFixture(page);
  const card = page.locator('.project-card').filter({ has: page.getByRole('heading', { name: 'Web Fixture', exact: true }) });
  for (const technology of webTechnologies) await expect(card.getByText(technology, { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Открыть страницу Web Fixture' }).click();
  await expect(check(page, 'http')).toHaveAttribute('data-state', 'checking');
  await expect(check(page, 'http')).not.toContainText('200');
  await expect(check(page, 'http')).toHaveAttribute('data-state', 'error');
  await expect(check(page, 'http')).toContainText('503');
  await expect(check(page, 'http')).not.toContainText('200');
  await page.evaluate(() => { (window as any).readinessFixture.mode = 'http200'; });
  await page.getByRole('button', { name: 'Обновить проверку готовности' }).click();
  await expect(check(page, 'http')).toHaveAttribute('data-state', 'ready');
  await expect(check(page, 'http')).toContainText('200');
  expect(await page.evaluate(() => (window as any).readinessFixture.maxDiagnostics)).toBe(1);
  await expect(page.getByText('WEB event only', { exact: true })).toBeVisible();
  await page.screenshot({ path: 'test-results/readiness-detail-wide.png', fullPage: true });
  await page.setViewportSize({ width: 900, height: 1000 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(false);
  await page.screenshot({ path: 'test-results/readiness-detail-900.png', fullPage: true });
});

test('Android renders unknown process and inapplicable HTTP/port without fabricated success', async ({ page }) => {
  await nativeFixture(page, 'http200');
  await page.getByRole('button', { name: 'Открыть страницу Android Fixture' }).click();
  await expect(check(page, 'process')).toHaveAttribute('data-state', 'unknown');
  await expect(check(page, 'http')).toHaveAttribute('data-state', 'notApplicable');
  await expect(check(page, 'port')).toHaveAttribute('data-state', 'notApplicable');
  await expect(readiness(page)).not.toContainText('HTTP 200');
});

test('owned Android target remains locked while the session manages the resource', async ({ page }) => {
  await nativeFixture(page, 'ownedAndroid');
  await page.getByRole('button', { name: 'Открыть страницу Android Fixture' }).click();
  await expect(page.getByRole('combobox', { name: 'Устройство Android' })).toBeDisabled();
  expect(await calls(page, 'save_profile')).toBe(0);
});

for (const mode of ['external', 'conflict']) {
  test(mode + ' resource shows ownership/port reason and never enables destructive actions', async ({ page }) => {
    await nativeFixture(page, mode);
    await page.getByRole('button', { name: 'Открыть страницу Web Fixture' }).click();
    await expect(page.getByRole('button', { name: 'Перезапустить', exact: true })).toBeDisabled();
    await expect(page.getByRole('button', { name: 'Остановить', exact: true })).toBeDisabled();
    await expect(readiness(page)).toContainText('Resource belongs to another process');
    await expect(check(page, 'port')).toHaveAttribute('data-state', mode === 'conflict' ? 'error' : 'ready');
    await expect(check(page, 'port')).toContainText(mode === 'conflict' ? 'другим приложением' : 'внешним процессом');
    expect(await calls(page, 'stop_project')).toBe(0);
    expect(await calls(page, 'restart_project')).toBe(0);
  });
}

test('write-safe site explains unavailable stop and restart even when mock claims ownership', async ({ page }) => {
  await nativeFixture(page, 'http200');
  await page.getByRole('button', { name: 'Открыть страницу Write-safe Site' }).click();
  await expect(readiness(page)).toContainText('Safe stop is unavailable because this site may be writing files.');
  await expect(page.getByRole('button', { name: 'Остановить', exact: true })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Перезапустить', exact: true })).toBeDisabled();
  expect(await calls(page, 'stop_project')).toBe(0);
  expect(await calls(page, 'restart_project')).toBe(0);
});

test('same-tick restart clicks invoke one operation and failure keeps controls usable', async ({ page }) => {
  await nativeFixture(page, 'restartFailure');
  await page.getByRole('button', { name: 'Открыть страницу Web Fixture' }).click();
  const restart = page.getByRole('button', { name: /^Перезапус/ });
  await expect(restart).toBeEnabled();
  await restart.evaluate((button: HTMLButtonElement) => { button.click(); button.click(); });
  await expect(restart).toBeDisabled();
  await expect(page.getByRole('alert').filter({ hasText: 'Mock graceful stop refused; project kept running' })).toBeVisible();
  await expect(restart).toBeEnabled();
  expect(await calls(page, 'restart_project')).toBe(1);
  expect(await calls(page, 'stop_project')).toBe(0);
  await expect(page.locator('.launch-panel .status-ready')).toBeVisible();
});

test('stop error is visible and does not report a successful stop', async ({ page }) => {
  await nativeFixture(page, 'stopFailure');
  await page.getByRole('button', { name: 'Открыть страницу Web Fixture' }).click();
  const stop = page.getByRole('button', { name: /^Остан/ });
  await expect(stop).toBeEnabled();
  await stop.click();
  await expect(page.getByRole('alert').filter({ hasText: 'Mock safe stop refused; resource retained' })).toBeVisible();
  await expect(stop).toBeEnabled();
  await expect(page.locator('.launch-panel .status-ready')).toBeVisible();
  expect(await calls(page, 'stop_project')).toBe(1);
});

test('successful atomic restart and stop refresh ownership without duplicate commands', async ({ page }) => {
  await nativeFixture(page, 'http200');
  await page.getByRole('button', { name: 'Открыть страницу Web Fixture' }).click();
  const restart = page.getByRole('button', { name: /^Перезапус/ });
  await expect(restart).toBeEnabled();
  await restart.click();
  await expect(restart).toBeDisabled();
  await expect(restart).toBeEnabled();
  await expect(page.locator('.launch-panel .status-ready')).toBeVisible();
  expect(await calls(page, 'restart_project')).toBe(1);
  expect(await calls(page, 'stop_project')).toBe(0);
  const stop = page.getByRole('button', { name: /^Остан/ });
  await stop.click();
  await expect(page.locator('.launch-panel .status-stopped')).toBeVisible();
  await expect(stop).toBeDisabled();
  await expect(restart).toBeDisabled();
  expect(await calls(page, 'stop_project')).toBe(1);
});

test('late logs from the previous project never enter the selected project journal', async ({ page }) => {
  await nativeFixture(page, 'staleLogs');
  await page.getByRole('button', { name: 'Открыть страницу Web Fixture' }).click();
  await expect(readiness(page)).toBeVisible();
  await page.getByRole('button', { name: /К коллекции/ }).click();
  await page.getByRole('button', { name: 'Открыть страницу Container Fixture' }).click();
  await expect(page.getByText('DOCKER event only', { exact: true })).toBeVisible();
  await page.waitForTimeout(1300);
  await expect(page.getByText('WEB event only', { exact: true })).toHaveCount(0);
  await expect(page.getByText('DOCKER event only', { exact: true })).toBeVisible();
});

test('diagnostic transport errors are visible and refresh can recover without a start', async ({ page }) => {
  await nativeFixture(page, 'diagnosticFailure');
  await page.getByRole('button', { name: 'Открыть страницу Web Fixture' }).click();
  await expect(readiness(page)).toContainText('Mock diagnostic connection failed');
  await page.evaluate(() => { (window as any).readinessFixture.mode = 'http200'; });
  await page.getByRole('button', { name: 'Обновить проверку готовности' }).click();
  await expect(check(page, 'http')).toHaveAttribute('data-state', 'ready');
  expect(await calls(page, 'start_project')).toBe(0);
});
