// Project Shelf owns this bridge; the launched project's files stay untouched.
// Only protocol markers are emitted. Vite loads the project's existing config.
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { stat } from 'node:fs/promises';

function parseArguments(argv) {
  if (argv.length !== 4) throw new Error('invalid_arguments');
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (!['--project-root', '--port'].includes(flag) || values.has(flag)) {
      throw new Error('invalid_arguments');
    }
    values.set(flag, value);
  }
  const root = values.get('--project-root');
  const portValue = values.get('--port');
  const port = typeof portValue === 'string' && /^[1-9][0-9]{3,4}$/.test(portValue)
    ? Number(portValue) : NaN;
  if (!Number.isInteger(port) || port < 1024 || port > 65535 || typeof root !== 'string'
      || !/^[A-Za-z]:[\\/]/.test(root)
      || /[\x00-\x1f\x7f<>"|?*]/.test(root)
      || root.slice(2).includes(':')) {
    throw new Error('invalid_arguments');
  }
  const segments = root.slice(3).split(/[\\/]/).filter(Boolean);
  if (segments.some(segment => segment === '.' || segment === '..'
      || /[.\s]$/.test(segment))) {
    throw new Error('invalid_arguments');
  }
  return { root: path.win32.normalize(root), port };
}

function marker(stream, value) {
  // A parent can disconnect stdout/stderr when explicitly leaving projects
  // running. Broken protocol pipes must never take down the project server.
  try { stream.write(`${value}\n`); } catch { /* detached parent */ }
}

process.stdout.on('error', () => {});
process.stderr.on('error', () => {});

let server;
let closing = false;
let stopped = false;

async function shutdown() {
  if (closing || stopped || !server) return;
  closing = true;
  try {
    await server.close();
    stopped = true;
    // Let the event loop finish pending filesystem callbacks as well. A forced
    // process.exit() here could interrupt a project's asynchronous save even
    // after its HTTP connection has closed.
    process.exitCode = 0;
    marker(process.stdout, 'SHELF_STOPPED');
    process.stdin.pause();
    process.stdin.destroy();
  } catch {
    // A close failure may leave requests or file writes in flight. Keep this
    // process alive and permit an explicit retry instead of force-exiting.
    closing = false;
    marker(process.stderr, 'SHELF_ERROR shutdown_failed');
  }
}

async function main() {
  let root;
  let port;
  try {
    ({ root, port } = parseArguments(process.argv.slice(2)));
  } catch {
    marker(process.stderr, 'SHELF_ERROR invalid_arguments');
    process.exitCode = 1;
    return;
  }

  try {
    // This process is supervised and noninteractive. Vite otherwise installs
    // its own stdin-end handler that exits when Shelf chooses Leave. CI mode
    // disables that handler; it affects this child only, never the OS/.env.
    process.env.CI = 'true';
    if (!(await stat(root)).isDirectory()) throw new Error('invalid_root');
    const viteUrl = pathToFileURL(path.join(root, 'node_modules', 'vite', 'dist', 'node', 'index.js'));
    const { createServer } = await import(viteUrl.href);
    if (typeof createServer !== 'function') throw new Error('invalid_vite');
    server = await createServer({
      root,
      server: { host: '127.0.0.1', port, strictPort: true, open: false },
    });
    await server.listen();
  } catch {
    // No raw exception is printed: config errors can include project data.
    marker(process.stderr, 'SHELF_ERROR startup_failed');
    // Once Vite exists, even a startup failure can have created watchers or
    // partially active servers. Use the same graceful close contract.
    if (server) {
      try {
        await server.close();
      } catch {
        marker(process.stderr, 'SHELF_ERROR shutdown_failed');
        installInput();
        return;
      }
    }
    process.exitCode = 1;
    return;
  }

  installInput();
  marker(process.stdout, 'SHELF_READY');
}

function installInput() {
  // Parse only terminated lines: readline also emits an unterminated fragment
  // on EOF, which must not turn a parent's disconnect into a shutdown command.
  let pending = '';
  let discard = false;
  process.stdin.on('data', chunk => {
    for (const character of chunk.toString('utf8')) {
      if (character === '\n') {
        const line = pending.endsWith('\r') ? pending.slice(0, -1) : pending;
        if (!discard && line === 'shutdown') void shutdown();
        pending = '';
        discard = false;
      } else if (!discard) {
        pending += character;
        if (pending.length > 64) { pending = ''; discard = true; }
      }
    }
  });
  const ignoreDisconnect = () => { pending = ''; discard = false; };
  process.stdin.on('end', ignoreDisconnect);
  process.stdin.on('error', ignoreDisconnect);
  // EOF/error means the parent left, not that the project should stop.
}

await main();
