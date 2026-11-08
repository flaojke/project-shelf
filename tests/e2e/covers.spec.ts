import { test, expect, type Page } from '@playwright/test';

const projects = [{"name":"Android-приложение","asset":"android","height":846},{"name":"Веб-проект","asset":"web","height":845},{"name":"PowerShell-сайт","asset":"powershell","height":846},{"name":"Docker-сервис","asset":"docker","height":845}];

async function assertCardImages(page: Page) {
  const urls: string[] = [];
  for (const project of projects) {
    const card = page.locator('.project-card').filter({ has: page.getByRole('heading', { name: project.name, exact: true }) });
    const image = card.locator('.project-cover > img');
    await expect(image).toBeVisible();
    await expect.poll(() => image.evaluate((element: HTMLImageElement) => element.complete && element.naturalWidth > 0)).toBe(true);
    const data = await image.evaluate((element: HTMLImageElement) => ({ src: element.currentSrc, width: element.naturalWidth, height: element.naturalHeight, fit: getComputedStyle(element).objectFit }));
    expect(data.width).toBe(1860);
    expect(data.height).toBe(project.height);
    expect(new URL(data.src).pathname).toMatch(new RegExp('/' + project.asset + '-cover(?:-[A-Za-z0-9_-]+)?\\.png$'));
    expect(new URL(data.src).origin).toBe(new URL(page.url()).origin);
    expect(data.fit).toBe('cover');
    urls.push(data.src);
  }
  expect(new Set(urls).size).toBe(4);
}

test('four final covers map to the correct cards and retain loaded detail crops at 1440 and 900 pixels', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator('.project-card')).toHaveCount(4);
  await assertCardImages(page);
  await page.mouse.move(1, 1);
  await page.screenshot({ path: 'test-results/covers-home-1440.png', fullPage: true });
  await page.setViewportSize({ width: 900, height: 1000 });
  await assertCardImages(page);
  expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(false);
  await page.mouse.move(1, 1);
  await page.screenshot({ path: 'test-results/covers-home-900.png', fullPage: true });

  const android = projects.find(project => project.asset === 'android')!;
  await page.getByRole('button', { name: 'Открыть страницу ' + android.name }).click();
  const detail = page.locator('.detail-cover > img');
  await expect(detail).toBeVisible();
  await expect.poll(() => detail.evaluate((element: HTMLImageElement) => element.complete && element.naturalWidth)).toBe(1860);
  expect(await detail.evaluate((element: HTMLImageElement) => element.naturalHeight)).toBe(846);
  await expect(detail).toHaveAttribute('src', /android-cover(?:-[A-Za-z0-9_-]+)?\.png(?:\?.*)?$/);
  expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(false);
  await page.mouse.move(1, 1);
  await page.screenshot({ path: 'test-results/covers-android-detail-900.png', fullPage: true });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await expect(detail).toBeVisible();
  expect(await detail.evaluate((element: HTMLImageElement) => getComputedStyle(element).objectFit)).toBe('cover');
  await page.mouse.move(1, 1);
  await page.screenshot({ path: 'test-results/covers-android-detail-1440.png', fullPage: true });
});

test('an image load error falls back to the SVG without blocking asset import modules or navigation', async ({ page }) => {
  const pageErrors: string[] = [], abortedTypes: string[] = [];
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.route('**/*android-cover*.png*', async route => {
    if (route.request().resourceType() === 'image') {
      abortedTypes.push(route.request().resourceType());
      await route.abort('failed');
    } else await route.continue();
  });
  await page.goto('/');
  await expect(page.locator('.project-card')).toHaveCount(4);
  const android = projects.find(project => project.asset === 'android')!;
  const card = page.locator('.project-card').filter({ has: page.getByRole('heading', { name: android.name, exact: true }) });
  await expect(card.locator('.project-cover > img')).toHaveCount(0);
  await expect(card.getByRole('img', { name: 'Авторская обложка ' + android.name })).toBeVisible();
  await expect(page.locator('.project-card .project-cover > img')).toHaveCount(3);
  await page.getByRole('button', { name: 'Открыть страницу ' + android.name }).click();
  await expect(page.getByRole('heading', { name: android.name, exact: true })).toBeVisible();
  await expect(page.locator('.detail-cover > img')).toHaveCount(0);
  await expect(page.locator('.detail-cover').getByRole('img', { name: 'Авторская обложка ' + android.name })).toBeVisible();
  expect(abortedTypes.length).toBeGreaterThan(0);
  expect(abortedTypes.every(type => type === 'image')).toBe(true);
  expect(pageErrors).toEqual([]);
});
