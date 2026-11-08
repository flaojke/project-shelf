import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { safeRelative, allowedSourcePath, scanBuffer, pngIssues, validateDefaults, makePackageManifest } from './ci-verify.mjs';

test('review paths reject parent traversal, drive and network roots', () => {
  assert.equal(safeRelative('licenses/npm/example/LICENSE'), 'licenses/npm/example/LICENSE');
  for (const value of ['../other', 'folder/../other', '/root', 'C:/root', '\\\\server\\share', 'folder//file']) assert.throws(() => safeRelative(value));
});

test('publication whitelist accepts product source and reviewed legal texts, not application data', () => {
  for (const name of ['.gitattributes', 'src/App.tsx', 'src/assets.d.ts', 'src/assets/covers/web-cover.png', 'profiles/defaults.json', '.github/workflows/windows-build.yml', 'docs/CI_WINDOWS.md']) assert.equal(allowedSourcePath(name), true);
  for (const name of ['profiles/local.json', 'data/application.json', 'verification/report.json', 'runtime/node.exe', 'docs/private-notes.md', 'src/assets/covers/unreviewed.png']) assert.equal(allowedSourcePath(name), false);
  assert.equal(allowedSourcePath('licenses/example/LICENSE'), false);
  assert.equal(allowedSourcePath('licenses/example/LICENSE', new Set(['licenses/example/LICENSE'])), true);
});

test('privacy scan checks both UTF-8 and UTF-16 without returning matched values', () => {
  const localPath = path.win32.join('C:\\', 'Users', 'Example', 'source.rs');
  for (const encoding of ['utf8', 'utf16le']) assert.deepEqual(scanBuffer(Buffer.from(localPath, encoding)), ['local-user-path']);
  const token = ['ghp', 'x'.repeat(32)].join('_');
  assert.deepEqual(scanBuffer(Buffer.from(token)), ['access-token']);
  const key = ['-----BEGIN ', 'PRIVATE KEY-----'].join('');
  assert.deepEqual(scanBuffer(Buffer.from(key)), ['private-key']);
  assert.deepEqual(scanBuffer(Buffer.from('Ordinary product code and mandatory copyright notices.')), []);
});

test('PNG metadata and appended payloads require review', () => {
  const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  const chunk = type => Buffer.concat([Buffer.alloc(4), Buffer.from(type), Buffer.alloc(4)]);
  const minimal = Buffer.concat([signature, chunk('IHDR'), chunk('IEND')]);
  assert.deepEqual(pngIssues(minimal), []);
  assert.deepEqual(pngIssues(Buffer.concat([signature, chunk('tEXt'), chunk('IEND')])), ['png-text-or-exif-needs-review']);
  assert.ok(pngIssues(Buffer.concat([minimal, Buffer.from('extra')])).includes('invalid-png-or-appended-data'));
  assert.deepEqual(pngIssues(Buffer.from('not an image')), ['invalid-png']);
});

test('public setup profiles cannot silently acquire installed project paths', () => {
  const profiles = Object.entries({ web: 'process', powershell: 'powershell', docker: 'docker', android: 'android' }).map(([id, kind]) => ({ id, kind, path: '', config: {} }));
  assert.doesNotThrow(() => validateDefaults(profiles));
  const configured = structuredClone(profiles); configured[0].path = 'C:/Projects/Example';
  assert.throws(() => validateDefaults(configured));
  const android = structuredClone(profiles); android[3].config.packageName = 'com.example.app';
  assert.throws(() => validateDefaults(android));
  assert.throws(() => validateDefaults(profiles.slice(0, 3)));
});

test('artifact manifest binds the exact commit and does not claim signing or native acceptance', () => {
  const files = [{ file: 'Project Shelf.exe', bytes: 20, sha256: 'a'.repeat(64) }];
  const manifest = makePackageManifest({ version: '0.2.0', commit: 'b'.repeat(40), runId: '123', runAttempt: '1' }, { contentManifestSha256: 'c'.repeat(64) }, files);
  assert.equal(manifest.commit, 'b'.repeat(40)); assert.equal(manifest.sourceContentManifestSha256, 'c'.repeat(64));
  assert.equal(manifest.signed, false); assert.equal(manifest.nativeManualAcceptance, false); assert.deepEqual(manifest.files, files);
  assert.equal('workspace' in manifest, false); assert.equal('environment' in manifest, false);
});
