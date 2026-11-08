import { test, expect } from '@playwright/test';
test('slow native status polling has one in-flight call and renders setup safely', async ({ page }) => {
  await page.addInitScript(() => {
    const profiles = ['web', 'powershell', 'docker', 'android'].map((id, i) => ({ id, name: id, subtitle: 'Unconfigured example', kind: ['process', 'powershell', 'docker', 'android'][i], path: '', accent: '#c66145', cover: id, config: {}, tags: [], favorite: false }));
    const state = window as unknown as { maxConcurrency: number; dangerousCalls: string[]; __TAURI_INTERNALS__: { invoke: (command: string) => Promise<unknown> } };
    let active = 0; state.maxConcurrency = 0; state.dangerousCalls = [];
    state.__TAURI_INTERNALS__ = { invoke: async command => {
      if (command === 'list_profiles') return profiles;
      if (command === 'get_settings') return { theme: 'light', closeToTray: true, autoOpenBrowser: false, liveActionsEnabled: true };
      if (command === 'profile_validation') return { complete: false, issues: ['Configure the profile'] };
      if (command === 'get_statuses') {
        active++; state.maxConcurrency = Math.max(active, state.maxConcurrency);
        await new Promise(resolve => setTimeout(resolve, 2500)); active--;
        return profiles.map(p => ({ id: p.id, phase: 'setup', owned: false, message: 'Configure first' }));
      }
      state.dangerousCalls.push(command); throw new Error('Unexpected action before setup');
    } };
  });
  await page.goto('/'); await expect(page.locator('.status-setup')).toHaveCount(4, { timeout: 8000 });
  await page.waitForTimeout(2800);
  expect(await page.evaluate(() => (window as unknown as { maxConcurrency: number }).maxConcurrency)).toBe(1);
  expect(await page.evaluate(() => (window as unknown as { dangerousCalls: string[] }).dangerousCalls)).toEqual([]);
  await page.evaluate(() => window.dispatchEvent(new Event('shelf:close-requested')));
  await expect(page.getByRole('dialog', { name: 'Убрать Project Shelf в трей?' })).toBeVisible();
});
