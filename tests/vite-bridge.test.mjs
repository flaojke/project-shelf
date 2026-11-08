import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import { spawn } from 'node:child_process';

const bridge = path.resolve('src-tauri/resources/vite-supervisor.mjs');
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(condition, message) { for (let i = 0; i < 150; i++) { if (condition()) return; await wait(20); } throw new Error(message); }
function fixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'shelf-vite-bridge-'));
  const moduleRoot = path.join(root, 'node_modules/vite'); fs.mkdirSync(path.join(moduleRoot, 'dist/node'), { recursive: true });
  fs.writeFileSync(path.join(moduleRoot, 'package.json'), '{"type":"module"}');
  fs.writeFileSync(path.join(moduleRoot, 'dist/node/index.js'), `
import fs from 'node:fs'; import path from 'node:path'; import http from 'node:http';
export async function createServer(options) {
  // Match Vite 7's setupSIGTERMListener: non-CI stdin EOF exits the server.
  if(process.env.CI!=='true')process.stdin.on('end',()=>process.exit(0));
  const root=options.root; const server=http.createServer((_req,res)=>{res.statusCode=204;res.end();}); let count=0;
  return {
    config:{root,server:options.server}, printUrls(){}, bindCLIShortcuts(){},
    async listen(){ await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve)); fs.writeFileSync(path.join(root,'ready.json'),JSON.stringify({port:server.address().port,options})); },
    async close(){count++;fs.writeFileSync(path.join(root,'close-count.txt'),String(count)); await new Promise(resolve=>setTimeout(resolve,250)); await new Promise(resolve=>server.close(resolve)); fs.writeFileSync(path.join(root,'closed.txt'),'closed');setTimeout(()=>fs.writeFileSync(path.join(root,'drained.txt'),'pending I/O complete'),100);}
  };
}
`);
  return root;
}
async function setup(port = 5180) {
  assert.equal(fs.existsSync(bridge), true, 'Production Vite bridge must exist before these checks run.');
  const root = fixture();
  const child = spawn(process.execPath, [bridge, '--project-root', root, '--port', String(port)], { stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true, env: { ...process.env, CI: 'false' } });
  let stderr = ''; child.stderr.on('data', data => { stderr += data; }); child.stdout.resume();
  try { await until(() => fs.existsSync(path.join(root, 'ready.json')), `Bridge not ready: ${stderr}`); }
  catch (error) { if (child.exitCode === null) child.kill(); safeRemove(root); throw error; }
  return { root, child, ready: JSON.parse(fs.readFileSync(path.join(root, 'ready.json'), 'utf8')), stderr: () => stderr };
}
function safeRemove(root) {
  const target = path.resolve(root), relative = path.relative(path.resolve(os.tmpdir()), target);
  assert.ok(relative && !relative.startsWith('..') && !path.isAbsolute(relative) && path.basename(target).startsWith('shelf-vite-bridge-'));
  fs.rmSync(target, { recursive: true, force: true });
}
async function cleanup({ root, child }) {
  if (child.exitCode === null) { child.kill(); await until(() => child.exitCode !== null || child.signalCode !== null, 'Own fixture child did not exit'); }
  safeRemove(root);
}

test('production Vite bridge closes the server before exiting, preserving required config', async () => {
  const scope = await setup(5198);
  try {
    assert.equal(scope.ready.options.root, scope.root);
    assert.deepEqual(scope.ready.options.server, { host: '127.0.0.1', port: 5198, strictPort: true, open: false });
    scope.child.stdin.write('shutdown\n');
    await until(() => scope.child.exitCode !== null, `Graceful bridge did not exit: ${scope.stderr()}`);
    assert.equal(scope.child.exitCode, 0);
    assert.equal(fs.readFileSync(path.join(scope.root, 'closed.txt'), 'utf8'), 'closed');
  } finally { await cleanup(scope); }
});

test('invalid bridge arguments fail before loading the configured project', async () => {
  const root = fixture();
  try {
    for (const args of [
      ['--project-root', root, '--port', '80'],
      ['--project-root', root, '--port', '70000'],
      ['--project-root', root, '--command', 'ignored'],
    ]) {
      const child = spawn(process.execPath, [bridge, ...args], { stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });
      let stderr = ''; child.stderr.on('data', data => { stderr += data; }); child.stdout.resume();
      const code = await new Promise((resolve, reject) => { child.once('error', reject); child.once('exit', resolve); });
      assert.equal(code, 1);
      assert.equal(stderr.trim(), 'SHELF_ERROR invalid_arguments');
      assert.equal(fs.existsSync(path.join(root, 'ready.json')), false);
    }
  } finally { safeRemove(root); }
});

test('stdin EOF from launcher Leave keeps the Vite server running', async () => {
  const scope = await setup();
  try {
    scope.child.stdin.end(); await wait(350);
    assert.equal(scope.child.exitCode, null);
    assert.equal(fs.existsSync(path.join(scope.root, 'closed.txt')), false);
    const code = await new Promise((resolve, reject) => { http.get(`http://127.0.0.1:${scope.ready.port}`, res => { res.resume(); resolve(res.statusCode); }).on('error', reject); });
    assert.equal(code, 204);
  } finally { await cleanup(scope); }
});

test('repeated shutdown messages call Vite close exactly once', async () => {
  const scope = await setup();
  try {
    scope.child.stdin.write('shutdown\nshutdown\n');
    await until(() => scope.child.exitCode !== null, 'Repeated shutdown did not finish');
    assert.equal(scope.child.exitCode, 0);
    assert.equal(fs.readFileSync(path.join(scope.root, 'close-count.txt'), 'utf8'), '1');
  } finally { await cleanup(scope); }
});

test('shutdown drains pending project callbacks after the Vite close promise', async () => {
  const scope = await setup();
  try {
    scope.child.stdin.write('shutdown\n');
    await until(() => scope.child.exitCode !== null, 'Bridge did not drain and exit');
    assert.equal(scope.child.exitCode, 0);
    assert.equal(fs.readFileSync(path.join(scope.root, 'drained.txt'), 'utf8'), 'pending I/O complete');
  } finally { await cleanup(scope); }
});
