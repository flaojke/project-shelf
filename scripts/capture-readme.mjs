import { chromium } from '@playwright/test';
import { createServer } from 'vite';
import fs from 'node:fs/promises';

// Isolated browser demo: no native IPC, personal profiles or system projects.
const server = await createServer({ server: { host: '127.0.0.1', port: 1422, strictPort: true } });
let browser;
try {
  await server.listen();
  browser = await chromium.launch({ channel: 'msedge', headless: true });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1080 }, deviceScaleFactor: 1 });
  const page = await context.newPage();
  page.setDefaultTimeout(15_000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await fs.mkdir('docs/assets', { recursive: true });
  await page.goto('http://127.0.0.1:1422/');
  await page.locator('.project-card').last().waitFor();
  await page.evaluate(async () => { await document.fonts.ready; await Promise.all([...document.images].map(image => image.decode())); });
  await page.mouse.move(1, 1);
  await page.screenshot({ path: 'docs/assets/shelf-home.png', fullPage: true, animations: 'disabled', caret: 'hide' });
  await page.getByRole('button', { name: 'Открыть страницу Веб-проект', exact: true }).click();
  await page.locator('.detail-cover > img').waitFor();
  await page.evaluate(async () => { await Promise.all([...document.images].map(image => image.decode())); });
  await page.screenshot({ path: 'docs/assets/shelf-detail.png', fullPage: true, animations: 'disabled', caret: 'hide' });
  await page.getByRole('button', { name: /К коллекции/ }).click();
  const card = page.locator('.project-card').filter({ has: page.getByRole('heading', { name: 'Веб-проект', exact: true }) });
  await card.getByRole('button', { name: 'Настроить', exact: true }).click();
  await page.getByRole('dialog', { name: 'Настройка веб-проекта' }).waitFor();
  await page.getByText('Пока требуется настройка', { exact: true }).waitFor();
  await page.screenshot({ path: 'docs/assets/shelf-setup.png', fullPage: true, animations: 'disabled', caret: 'hide' });
  await page.getByRole('button', { name: 'Закрыть настройки проекта', exact: true }).click();
  const android = page.locator('.project-card').filter({ has: page.getByRole('heading', { name: 'Android-приложение', exact: true }) });
  await android.getByRole('button', { name: 'Настроить', exact: true }).click();
  await page.getByRole('dialog').waitFor();
  await page.getByText('Пока требуется настройка', { exact: true }).waitFor();
  await page.screenshot({ path: 'docs/assets/shelf-android.png', fullPage: true, animations: 'disabled', caret: 'hide' });
  await page.getByRole('button', { name: 'Закрыть настройки проекта', exact: true }).click();
  await page.getByRole('button', { name: 'Настройки', exact: true }).click();
  await page.getByRole('dialog', { name: 'Немного настроек' }).waitFor();
  await page.screenshot({ path: 'docs/assets/shelf-settings.png', fullPage: true, animations: 'disabled', caret: 'hide' });
  if (errors.length) throw new Error(errors.join('\n'));
  console.log('Captured five real browser-demo screenshots; no native project actions performed.');
} finally {
  if (browser) await browser.close();
  await server.close();
}
