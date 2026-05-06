import { spawn, spawnSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import net from 'node:net';

function loadDotenvIfExists(path = '.env') {
  if (!existsSync(path)) {
    return;
  }

  const lines = readFileSync(path, 'utf8').split(/\r?\n/);
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith('#')) {
      continue;
    }

    const separator = trimmed.indexOf('=');
    if (separator === -1) {
      continue;
    }

    const key = trimmed.slice(0, separator).trim();
    const rawValue = trimmed.slice(separator + 1).trim();
    if (!key) {
      continue;
    }

    process.env[key] = rawValue.replace(/^(['"])(.*)\1$/, '$2');
  }
}

function applyDatabaseSchema() {
  console.log('sanchoris dev database: applying db/schema.sql');
  const result = spawnSync('pnpm', ['db:apply'], {
    env: process.env,
    stdio: 'inherit',
    shell: false,
  });

  if (result.error) {
    console.error(`db:apply failed to start: ${result.error.message}`);
    process.exit(1);
  }

  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

async function findAvailablePort() {
  return await new Promise((resolve, reject) => {
    const server = net.createServer();
    server.unref();
    server.on('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      if (!address || typeof address === 'string') {
        server.close(() => reject(new Error('Failed to allocate a local port.')));
        return;
      }

      const { port } = address;
      server.close(() => resolve(port));
    });
  });
}

function run(name, command, args, env) {
  const child = spawn(command, args, {
    env: { ...process.env, ...env },
    stdio: 'inherit',
    shell: false,
  });

  child.on('error', (error) => {
    if (shuttingDown) {
      return;
    }

    shuttingDown = true;
    console.error(`${name} failed to start: ${error.message}`);
    stopAll();
    process.exit(1);
  });

  child.on('exit', (code, signal) => {
    if (shuttingDown) {
      return;
    }

    shuttingDown = true;
    console.error(`${name} exited with ${signal ?? code}`);
    stopAll();
    process.exit(code ?? 1);
  });

  children.push(child);
  return child;
}

function stopAll() {
  for (const child of children) {
    if (child.exitCode === null && !child.killed) {
      child.kill('SIGTERM');
    }
  }
}

loadDotenvIfExists();
applyDatabaseSchema();

const children = [];
let shuttingDown = false;

const backendPort = await findAvailablePort();
const backendUrl = `http://127.0.0.1:${backendPort}`;

console.log(`sanchoris dev backend: ${backendUrl}`);
console.log('sanchoris dev frontend: managed by portless on http://sanchoris.localhost:1355 by default');

run(
  'backend',
  'cargo',
  ['watch', '-x', 'run -p sanchoris-backend'],
  {
    PORT: String(backendPort),
    SANCHORIS_BACKEND_ADDR: `127.0.0.1:${backendPort}`,
  },
);

run(
  'frontend',
  'pnpm',
  ['--filter', '@sanchoris/frontend', 'dev'],
  {
    PORTLESS_HTTPS: process.env.PORTLESS_HTTPS ?? '0',
    PORTLESS_PORT: process.env.PORTLESS_PORT ?? '1355',
    VITE_BACKEND_URL: backendUrl,
  },
);

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    shuttingDown = true;
    stopAll();
    process.exit(0);
  });
}
