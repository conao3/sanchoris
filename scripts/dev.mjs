import { spawn } from 'node:child_process';
import net from 'node:net';

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
