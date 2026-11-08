import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

const MAX_REVIEW_BYTES = 96 * 1024 * 1024;
const COVERS = ['android', 'web', 'powershell', 'docker'].map(kind => `src/assets/covers/${kind}-cover.png`);
const sha256 = data => crypto.createHash('sha256').update(data).digest('hex');
const sourceRootFiles = new Set(['.gitignore', '.gitattributes', 'README.md', 'LICENSE', 'NOTICE', 'THIRD_PARTY_NOTICES.md', 'index.html', 'package.json', 'package-lock.json', 'tsconfig.json', 'vite.config.ts', 'playwright.config.ts', 'profiles/defaults.json', 'public/brand-mark.svg', 'src/assets.d.ts', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock', 'src-tauri/build.rs', 'src-tauri/tauri.conf.json', '.github/workflows/windows-build.yml']);
const checks = [
  ['local-user-path', /[A-Za-z]:[\\/]Users[\\/][^\s"<>]+/i],
  ['local-home-path', /\/(?:home|Users)\/[^/\s"<>]+\//i],
  ['private-key', /-----BEGIN (?:RSA |EC |OPENSSH |DSA |ENCRYPTED )?PRIVATE KEY-----/],
  ['access-token', /\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{30,}|AKIA[A-Z0-9]{16}|xox[baprs]-[A-Za-z0-9-]{20,})\b/],
  ['secret-literal', /(?:api[_-]?key|access[_-]?token|client[_-]?secret|password)\s*[:=]\s*["'][A-Za-z0-9_+\/-]{16,}["']/i],
];

export function safeRelative(value) {
  if (!value || path.isAbsolute(value) || /^[A-Za-z]:/.test(value) || /[\u0000-\u001f]/.test(value)) throw new Error('Unsafe relative path');
  const parts = value.replaceAll('\\', '/').split('/');
  if (parts.some(part => !part || part === '.' || part === '..')) throw new Error('Unsafe relative path');
  return parts.join('/');
}

export function allowedSourcePath(value, legalPaths = new Set()) {
  const name = safeRelative(value);
  return sourceRootFiles.has(name)
    || legalPaths.has(name)
    || ['docs/assets/hero.svg', 'docs/assets/shelf-home.png', 'docs/assets/shelf-detail.png', 'docs/assets/shelf-setup.png', 'scripts/capture-readme.mjs'].includes(name)
    || /^docs\/[A-Z][A-Z0-9_-]*\.md$/.test(name)
    || /^scripts\/(?:Build-ProjectShelf\.ps1|Create-Shortcut\.ps1|ci-verify\.mjs|ci-verify\.test\.mjs)$/.test(name)
    || /^src\/(?:[A-Za-z0-9_-]+\/)*[A-Za-z0-9_-]+\.(?:ts|tsx|css)$/.test(name)
    || COVERS.includes(name)
    || /^src-tauri\/src\/(?:[A-Za-z0-9_-]+\/)*[A-Za-z0-9_-]+\.rs$/.test(name)
    || /^src-tauri\/icons\/(?:[A-Za-z0-9_@.-]+\/)*[A-Za-z0-9_@.-]+\.(?:png|ico)$/.test(name)
    || /^src-tauri\/capabilities\/[A-Za-z0-9_-]+\.json$/.test(name)
    || name === 'src-tauri/resources/vite-supervisor.mjs'
    || /^tests\/(?:[A-Za-z0-9_-]+\/)*[A-Za-z0-9_.-]+\.(?:mjs|ts)$/.test(name);
}

export function scanBuffer(bytes) {
  const found = new Set();
  for (const encoding of ['utf8', 'utf16le']) {
    const text = bytes.toString(encoding);
    for (const [category, pattern] of checks) if (pattern.test(text)) found.add(category);
  }
  return [...found];
}

export function pngIssues(bytes) {
  if (!bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) return ['invalid-png'];
  let offset = 8, ended = false;
  const issues = new Set();
  while (offset + 12 <= bytes.length) {
    const length = bytes.readUInt32BE(offset), type = bytes.toString('ascii', offset + 4, offset + 8);
    if (length > bytes.length - offset - 12) return ['invalid-png-chunk'];
    if (['tEXt', 'iTXt', 'zTXt', 'eXIf'].includes(type)) issues.add('png-text-or-exif-needs-review');
    offset += length + 12;
    if (type === 'IEND') { ended = true; break; }
  }
  if (!ended || offset !== bytes.length) issues.add('invalid-png-or-appended-data');
  return [...issues];
}

export function validateDefaults(profiles) {
  const kinds = { web: 'process', powershell: 'powershell', docker: 'docker', android: 'android' };
  if (!Array.isArray(profiles) || profiles.length !== 4 || new Set(profiles.map(p => p.id)).size !== 4) throw new Error('Four public profile slots are required');
  const requiredEmpty = { web: ['nodePath', 'readyMarker'], powershell: ['service'], docker: ['dockerPath', 'composeFile', 'service', 'healthMessage'], android: ['sdkRoot', 'avd', 'serial', 'avdHome', 'androidUserHome', 'packageName', 'activity', 'apkPath'] };
  for (const profile of profiles) {
    if (kinds[profile.id] !== profile.kind || profile.path !== '' || !profile.config || typeof profile.config !== 'object') throw new Error('Invalid public profile identity');
    for (const key of requiredEmpty[profile.id]) if (profile.config[key] != null && profile.config[key] !== '') throw new Error('Public profiles must remain unconfigured');
    for (const value of Object.values(profile.config)) if (typeof value === 'string' && /^[A-Za-z]:[\\/]/.test(value)) throw new Error('Public default contains a local path');
  }
}

function walkFiles(root) {
  const files = [];
  const walk = directory => {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const absolute = path.join(directory, entry.name);
      if (entry.isSymbolicLink()) throw new Error('Links are not allowed in review materials');
      if (entry.isDirectory()) walk(absolute);
      else if (entry.isFile()) files.push(safeRelative(path.relative(root, absolute)));
      else throw new Error('Unsupported review file type');
    }
  };
  walk(root);
  return files.sort();
}

function auditFile(root, relative, options = {}) {
  const name = safeRelative(relative), absolute = path.join(root, name);
  const rootReal = fs.realpathSync(root), fileReal = fs.realpathSync(absolute);
  if (!fileReal.toLowerCase().startsWith(rootReal.toLowerCase() + path.sep)) throw new Error('File escaped the review root');
  if (!fs.lstatSync(absolute).isFile()) throw new Error('Only regular files are reviewed');
  const bytes = fs.readFileSync(absolute);
  const findings = scanBuffer(bytes);
  if (name.endsWith('.png')) findings.push(...pngIssues(bytes));
  if (!options.allowExecutable && bytes[0] === 0x4d && bytes[1] === 0x5a) findings.push('unexpected-executable');
  if (name.endsWith('.map')) findings.push('unexpected-source-map');
  if (name.endsWith('.svg') && /<script|<foreignObject|onload=|onerror=|(?:href|src)=["'][^#]/i.test(bytes.toString('utf8'))) findings.push('active-or-external-svg-content');
  if (findings.length) throw new Error(`Privacy review failed: ${name} [${[...new Set(findings)].join(', ')}]`);
  return { file: name, bytes: bytes.length, sha256: sha256(bytes) };
}

function githubContext() {
  if (process.env.GITHUB_ACTIONS !== 'true' || !/^[0-9a-f]{40}$/i.test(process.env.GITHUB_SHA || '')) throw new Error('This command requires an approved GitHub Actions run');
  const root = process.cwd(), checkedOut = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
  if (checkedOut !== process.env.GITHUB_SHA) throw new Error('Checked-out commit does not match the workflow commit');
  return { root, commit: checkedOut, runId: process.env.GITHUB_RUN_ID || '', runAttempt: process.env.GITHUB_RUN_ATTEMPT || '' };
}

function sourceSnapshot(context) {
  const tracked = execFileSync('git', ['ls-files', '-z'], { encoding: 'utf8' }).split('\0').filter(Boolean).sort();
  if (!tracked.length) throw new Error('No tracked source files');
  const inventory = JSON.parse(fs.readFileSync(path.join(context.root, 'licenses/inventory.json'), 'utf8'));
  const legalPaths = new Set(['licenses/inventory.json']);
  for (const item of [...inventory.npm, ...inventory.rust]) for (const file of item.legalFiles) {
    const name = `licenses/${safeRelative(file.path)}`;
    legalPaths.add(name);
    if (!tracked.includes(name) || sha256(fs.readFileSync(path.join(context.root, name))) !== file.sha256) throw new Error(`Legal text is missing or changed: ${name}`);
  }
  for (const name of tracked) if (!allowedSourcePath(name, legalPaths)) throw new Error(`Source file is outside the publication whitelist: ${safeRelative(name)}`);
  const forbidden = /(?:^|\/)(?:\.env(?:\.|$)|\.aws\/|\.codex\/|\.secrets\/|node_modules\/|target\/|dist\/|runtime\/|verification\/|_deployments\/)|(?:\.private\.|\.local\.)|\.(?:exe|dll|lib|msi|lnk|zip|7z|db|sqlite|csv|xlsx|pfx|pem|key|keystore|jks|log)$/i;
  for (const name of tracked) if (forbidden.test(name) && !name.startsWith('licenses/')) throw new Error(`Unpublishable source file: ${safeRelative(name)}`);
  for (const cover of COVERS) if (!tracked.includes(cover)) throw new Error(`Final cover is missing: ${cover}`);
  validateDefaults(JSON.parse(fs.readFileSync(path.join(context.root, 'profiles/defaults.json'), 'utf8')));
  const config = JSON.parse(fs.readFileSync(path.join(context.root, 'src-tauri/tauri.conf.json'), 'utf8'));
  if (config.identifier !== 'app.projectshelf.launcher') throw new Error('The public application namespace must stay isolated');
  const files = tracked.map(name => auditFile(context.root, name));
  return { commit: context.commit, files, contentManifestSha256: sha256(files.map(f => `${f.sha256}  ${f.file}\n`).join('')) };
}

function tempChild(name) {
  if (!process.env.RUNNER_TEMP) throw new Error('RUNNER_TEMP is required');
  const parent = fs.realpathSync(process.env.RUNNER_TEMP), child = path.resolve(parent, name);
  if (!child.toLowerCase().startsWith(parent.toLowerCase() + path.sep)) throw new Error('Unexpected build directory');
  return child;
}

function assertSameSource(before, after) {
  if (before.commit !== after.commit || before.contentManifestSha256 !== after.contentManifestSha256) throw new Error('Tracked source changed during the build');
}

function copyFile(source, destination) {
  fs.mkdirSync(path.dirname(destination), { recursive: true });
  fs.copyFileSync(source, destination, fs.constants.COPYFILE_EXCL);
}

function copyDirectory(source, destination) {
  for (const relative of walkFiles(source)) copyFile(path.join(source, relative), path.join(destination, relative));
}

export function makePackageManifest(context, source, files) {
  return {
    schemaVersion: 1, applicationVersion: context.version, commit: context.commit,
    workflowRunId: context.runId, workflowRunAttempt: context.runAttempt,
    sourceContentManifestSha256: source.contentManifestSha256,
    signed: false, nativeManualAcceptance: false, binaryMetadata: context.binaryMetadata || [],
    privacyReview: 'Whitelist, prepared payload and UTF-8/UTF-16 string checks; not proof of the absence of every possible secret or hidden payload.',
    files,
  };
}

function buildReviewPackage(context) {
  if (process.platform !== 'win32') throw new Error('Windows packaging must run on Windows');
  const before = JSON.parse(fs.readFileSync(tempChild('project-shelf-source-review.json'), 'utf8'));
  const current = sourceSnapshot(context);
  assertSameSource(before, current);
  const target = tempChild('project-shelf-target');
  if (path.resolve(process.env.CARGO_TARGET_DIR || '').toLowerCase() !== target.toLowerCase()) throw new Error('Unexpected Cargo target directory');
  const output = tempChild('project-shelf-review');
  if (path.resolve(process.env.SHELF_REVIEW_OUTPUT || '').toLowerCase() !== output.toLowerCase() || fs.existsSync(output)) throw new Error('Review destination must be new and isolated');
  const exe = path.join(target, 'release/project-shelf.exe');
  if (!fs.statSync(exe).isFile()) throw new Error('Application EXE is missing');
  const nsisDirectory = path.join(target, 'release/bundle/nsis');
  const installers = fs.readdirSync(nsisDirectory).filter(name => name.endsWith('.exe'));
  if (installers.length !== 1) throw new Error('Expected one NSIS installer');
  const installer = path.join(nsisDirectory, installers[0]);
  const binaryMetadata = JSON.parse(fs.readFileSync(tempChild('project-shelf-binary-info.json'), 'utf8').replace(/^\uFEFF/, ''));
  const expectedVersion = JSON.parse(fs.readFileSync(path.join(context.root, 'package.json'), 'utf8')).version;
  if (!Array.isArray(binaryMetadata) || binaryMetadata.length !== 2) throw new Error('Binary metadata is missing');
  for (const [index, file] of [exe, installer].entries()) {
    const item = binaryMetadata[index];
    const version = typeof item.productVersion === 'string' ? item.productVersion : '';
    const versionMatches = version === expectedVersion || version.startsWith(`${expectedVersion}.`) || version.startsWith(`${expectedVersion}+`) || version.startsWith(`${expectedVersion}-`);
    if (typeof item.sha256 !== 'string' || item.sha256.toLowerCase() !== sha256(fs.readFileSync(file)) || item.signatureStatus !== 'NotSigned' || !versionMatches) throw new Error('Unexpected binary version, signing state or hash');
  }
  for (const file of walkFiles(path.join(context.root, 'dist'))) auditFile(path.join(context.root, 'dist'), file);
  auditFile(path.dirname(exe), path.basename(exe), { allowExecutable: true });
  auditFile(nsisDirectory, installers[0], { allowExecutable: true });
  const extracted = tempChild('project-shelf-installer-review');
  if (fs.existsSync(extracted)) throw new Error('Installer review directory must be new');
  fs.mkdirSync(extracted);
  execFileSync('7z', ['x', installer, `-o${extracted}`, '-y', '-bso0', '-bsp0'], { stdio: ['ignore', 'pipe', 'pipe'] });
  const extractedFiles = walkFiles(extracted);
  if (!extractedFiles.some(name => name.toLowerCase().endsWith('.exe') && sha256(fs.readFileSync(path.join(extracted, name))) === sha256(fs.readFileSync(exe)))) throw new Error('Installer payload does not match the built application');
  for (const file of extractedFiles) auditFile(extracted, file, { allowExecutable: true });
  fs.mkdirSync(output);
  copyFile(exe, path.join(output, 'Project Shelf.exe'));
  copyFile(installer, path.join(output, 'Project Shelf-setup.exe'));
  for (const name of ['LICENSE', 'NOTICE', 'THIRD_PARTY_NOTICES.md']) copyFile(path.join(context.root, name), path.join(output, name));
  copyDirectory(path.join(context.root, 'licenses'), path.join(output, 'licenses'));
  const files = walkFiles(output).map(file => auditFile(output, file, { allowExecutable: true }));
  if (files.some(file => file.file.split('/').some(part => part.startsWith('.')))) throw new Error('Hidden review files would be excluded by artifact upload');
  const size = files.reduce((total, file) => total + file.bytes, 0);
  if (size > MAX_REVIEW_BYTES) throw new Error('Review package exceeds the 96 MiB storage guard');
  context.version = expectedVersion;
  context.binaryMetadata = binaryMetadata;
  const manifest = makePackageManifest(context, current, files);
  const manifestBytes = Buffer.from(JSON.stringify(manifest, null, 2) + '\n');
  if (size + manifestBytes.length > MAX_REVIEW_BYTES) throw new Error('Review package including manifest exceeds storage guard');
  fs.writeFileSync(path.join(output, 'BUILD-MANIFEST.json'), manifestBytes, { flag: 'wx' });
  console.log(`Reviewed ${files.length + 1} package files for commit ${context.commit}; ${size + manifestBytes.length} bytes.`);
  console.log('The package is unsigned and has not passed native manual acceptance.');
}

export function main(mode) {
  const context = githubContext();
  if (mode === 'source') {
    const source = sourceSnapshot(context);
    fs.writeFileSync(tempChild('project-shelf-source-review.json'), JSON.stringify(source) + '\n', { flag: 'wx' });
    console.log(`Reviewed ${source.files.length} tracked source files for commit ${source.commit}.`);
  } else if (mode === 'package') buildReviewPackage(context);
  else throw new Error('Use source or package');
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try { main(process.argv[2]); }
  catch (error) { console.error(error instanceof Error ? error.message : 'CI review failed'); process.exitCode = 1; }
}
