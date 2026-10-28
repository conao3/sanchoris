import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync } from 'node:fs';
import net from 'node:net';
import { join, resolve } from 'node:path';

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
    if (!key || process.env[key] !== undefined) {
      continue;
    }

    const doubleQuoted = /^"(.*)"$/.exec(rawValue);
    process.env[key] = doubleQuoted
      ? doubleQuoted[1].replace(/\\n/g, '\n')
      : rawValue.replace(/^'(.*)'$/, '$1');
  }
}

function runStep(name, args) {
  const result = spawnSync('pnpm', args, {
    env: process.env,
    stdio: 'inherit',
    shell: false,
  });

  if (result.error) {
    console.error(`${name} failed to start: ${result.error.message}`);
    process.exit(1);
  }

  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function createTables() {
  console.log('sanchoris dev database: creating the DynamoDB tables');
  runStep('dynamo:create', ['dynamo:create']);
}

function buildFrontend() {
  console.log('sanchoris dev frontend: building apps/frontend/dist for the api fallback');
  runStep('frontend build', ['--filter', '@sanchoris/frontend', 'build']);
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

// Ambient env (direnv) wins; .env.local overrides .env for a direct `pnpm dev`.
loadDotenvIfExists('.env.local');
loadDotenvIfExists('.env');

// The api, the worker, and the frontend require this config to start. Fail loudly
// here so a missing var is obvious rather than surfacing as a startup error in
// the sanchoris binary or a blank Hosted UI redirect.
const requiredEnv = [
  'SANCHORIS_TABLE_MAIN',
  'SANCHORIS_TABLE_AUDIT',
  'AWS_REGION',
  'AWS_ENDPOINT_URL_DYNAMODB',
  'COGNITO_USER_POOL_ID',
  'COGNITO_CLIENT_ID',
  'VITE_COGNITO_DOMAIN',
  'VITE_COGNITO_CLIENT_ID',
  'SANCHORIS_PUBLIC_URL',
  'GITHUB_URL',
  'GITHUB_API_URL',
  'GITHUB_APP_ID',
  'GITHUB_APP_PRIVATE_KEY',
  'GITHUB_APP_CLIENT_ID',
  'GITHUB_APP_CLIENT_SECRET',
  'GITHUB_APP_SLUG',
  'GITHUB_WEBHOOK_SECRET',
  'STRIPE_SECRET_KEY',
  'STRIPE_API_URL',
  'STRIPE_WEBHOOK_SECRET',
  'STRIPE_PORTAL_CONFIGURATION',
  'SANCHORIS_STRIPE_PRICE_STARTER',
  'SANCHORIS_STRIPE_PRICE_PRO',
  'SANCHORIS_STRIPE_PRICE_MAX',
  'SANCHORIS_CREDENTIAL_KEY',
  'RUN_TOKEN_SIGNING_KEY',
  'TASK_TOKEN_SIGNING_KEY',
  'OAUTH_STATE_SIGNING_KEY',
  'SANCHORIS_RECORDS_STORE',
  'SANCHORIS_GIT_URL',
  ...(process.env.SANCHORIS_RECORDS_STORE === 's3'
    ? ['AWS_REGION', 'SANCHORIS_RECORDS_BUCKET']
    : ['RECORDS_SIGNING_KEY']),
];
const missingEnv = requiredEnv.filter((key) => !process.env[key]);
if (missingEnv.length > 0) {
  console.error(
    `sanchoris dev: missing required env: ${missingEnv.join(', ')}.\n` +
      'Set them in the gitignored repo-root .env.local (see .env.example).',
  );
  process.exit(1);
}

createTables();
buildFrontend();

const children = [];
let shuttingDown = false;

const backendPort = await findAvailablePort();
const backendUrl = `http://127.0.0.1:${backendPort}`;
const devDir = resolve('.sanchoris/dev');
for (const dir of ['docs', 'records', 'sockets', 'workspaces']) {
  mkdirSync(join(devDir, dir), { recursive: true });
}

console.log(`sanchoris dev api: ${backendUrl}`);
console.log(`sanchoris dev public origin: ${process.env.SANCHORIS_PUBLIC_URL}`);

run(
  'api',
  'cargo',
  ['watch', '-x', 'run -p sanchoris-api --bin sanchoris -- all'],
  {
    SANCHORIS_BIND_ADDR: `127.0.0.1:${backendPort}`,
    SANCHORIS_CONTROL_URL: `ws://127.0.0.1:${backendPort}/runs/channel`,
    SANCHORIS_WORKSPACE_COMMAND: `node --no-warnings ${resolve('apps/workspace-agent/src/bin/workspace-agent.ts')}`,
    SANCHORIS_DOCS_DIR: join(devDir, 'docs'),
    SANCHORIS_FRONTEND_DIR: resolve('apps/frontend/dist'),
    SANCHORIS_SOCKET_DIR: join(devDir, 'sockets'),
    SANCHORIS_WORKSPACES_DIR: join(devDir, 'workspaces'),
    ...(process.env.SANCHORIS_RECORDS_STORE === 's3' ? {} : { SANCHORIS_RECORDS_DIR: join(devDir, 'records') }),
  },
);

run(
  'frontend',
  'pnpm',
  ['--filter', '@sanchoris/frontend', 'dev'],
  {
    VITE_BACKEND_URL: backendUrl,
    // Cognito Hosted UI config baked into the Vite client (import.meta.env.VITE_*).
    VITE_COGNITO_DOMAIN: process.env.VITE_COGNITO_DOMAIN,
    VITE_COGNITO_CLIENT_ID: process.env.VITE_COGNITO_CLIENT_ID,
  },
);

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    shuttingDown = true;
    stopAll();
    process.exit(0);
  });
}
