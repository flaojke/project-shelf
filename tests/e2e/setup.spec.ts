import { test, expect } from '@playwright/test';

test('fresh public app shows four setup cards; partial save never starts a project', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  await expect(page.locator('.project-card')).toHaveCount(4);
  await expect(page.locator('.status-setup')).toHaveCount(4);
  await expect(page.getByRole('button', { name: 'Настроить', exact: true })).toHaveCount(4);
  await page.screenshot({ path: 'test-results/public-setup-home.png', fullPage: true });
  const card = page.locator('.project-card').filter({ has: page.getByRole('heading', { name: 'Веб-проект', exact: true }) });
  await card.getByRole('button', { name: 'Настроить', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Настройка веб-проекта' });
  await dialog.locator('input[name=path]').fill('C:\\Projects\\ExampleWeb');
  await dialog.getByRole('button', { name: 'Сохранить', exact: true }).click();
  await expect(dialog.getByText(/Настройка сохранена/)).toBeVisible();
  await expect(dialog).toBeVisible();
  await page.keyboard.press('Control+k');
  await expect(dialog.locator('input[name=path]')).not.toBeFocused();
  await dialog.getByRole('button', { name: 'Закрыть', exact: true }).click();
  await expect(card.locator('.status-setup')).toBeVisible();
  expect(errors).toEqual([]);
});

test('valid web configuration saves first; only an explicit Start runs the demo', async ({ page }) => {
  await page.goto('/');
  const card = page.locator('.project-card').filter({ has: page.getByRole('heading', { name: 'Веб-проект', exact: true }) });
  await card.getByRole('button', { name: 'Настроить', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Настройка веб-проекта' });
  await dialog.locator('input[name=path]').fill('C:\\Projects\\ExampleWeb');
  await dialog.locator('input[name=readyMarker]').fill('Example Web');
  await expect(dialog.getByText('Настройки готовы к запуску')).toBeVisible();
  await dialog.getByRole('button', { name: 'Сохранить', exact: true }).click();
  await expect(dialog).not.toBeVisible();
  await expect(card.locator('.status-stopped')).toBeVisible();
  await card.getByRole('button', { name: 'Запустить', exact: true }).click();
  await expect(card.locator('.status-ready')).toBeVisible({ timeout: 8000 });
  await page.getByRole('button', { name: 'Открыть страницу Веб-проект' }).click();
  await expect(page.getByRole('button', { name: 'Настройки проекта', exact: true })).toBeDisabled();
  await expect(page.getByRole('heading', { name: 'Журнал проекта' })).toBeVisible();
  await page.screenshot({ path: 'test-results/public-configured-detail.png', fullPage: true });
});

test('all four setup forms are structured; Android setup does not require an already booted device', async ({ page }) => {
  await page.goto('/');
  const cases = [
    ['PowerShell-сайт', 'Настройка PowerShell-сайта', ['path', 'port', 'service']],
    ['Docker-сервис', 'Настройка Docker-сервиса', ['composeFile', 'service', 'url', 'dockerPath']],
    ['Android-приложение', 'Настройка Android-приложения', ['sdkRoot', 'avd', 'serial', 'packageName', 'activity']],
  ] as const;
  for (const [name, title, fields] of cases) {
    await page.locator('.project-card').filter({ has: page.getByRole('heading', { name, exact: true }) }).getByRole('button', { name: 'Настроить', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: title });
    for (const field of fields) await expect(dialog.locator(`input[name=${field}]`)).toBeVisible();
    await expect(dialog.locator('input[name=command],input[name=args],input[name=shell]')).toHaveCount(0);
    await dialog.getByRole('button', { name: 'Отмена', exact: true }).click();
  }
  await expect(page.locator('.status-setup')).toHaveCount(4);
});

test('search, favorites and narrow desktop layout remain usable', async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 800 }); await page.goto('/');
  await expect(page.locator('.project-card')).toHaveCount(4);
  await page.keyboard.press('Control+k');
  const search = page.getByRole('textbox', { name: 'Найти проект' }); await expect(search).toBeFocused();
  await search.fill('Docker'); await expect(page.locator('.project-card')).toHaveCount(1);
  await page.getByRole('button', { name: 'Очистить поиск' }).click();
  await page.getByRole('button', { name: 'Добавить Docker-сервис в избранное' }).click();
  await page.getByRole('button', { name: /^Избранное/ }).click();
  await expect(page.locator('.project-card')).toHaveCount(3);
  expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(false);
});

test('window close and full quit are distinct even with no configured projects', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Выйти из Project Shelf' }).click();
  const dialog = page.getByRole('dialog', { name: 'Завершить Project Shelf?' });
  await expect(dialog.getByRole('button', { name: /Оставить проекты работать/ })).toBeFocused();
  await dialog.getByRole('button', { name: 'Отмена', exact: true }).click();
  await expect(page.locator('.status-setup')).toHaveCount(4);
});
