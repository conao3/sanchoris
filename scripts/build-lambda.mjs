import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';

const frontendEnv = ['VITE_COGNITO_DOMAIN', 'VITE_COGNITO_CLIENT_ID'];
const missing = frontendEnv.filter((key) => !process.env[key]);
if (missing.length > 0) {
  console.error(
    `build:lambda: missing required env: ${missing.join(', ')}.\n` +
      'The api function embeds the frontend, which bakes these values in at build time.',
  );
  process.exit(1);
}

function step(command, args) {
  const result = spawnSync(command, args, { env: process.env, stdio: 'inherit', shell: false });
  if (result.error) {
    console.error(`build:lambda: ${command} failed to start: ${result.error.message}`);
    process.exit(1);
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

step('pnpm', ['--filter', '@sanchoris/frontend', 'build']);
if (!existsSync('apps/frontend/dist/index.html')) {
  console.error('build:lambda: apps/frontend/dist/index.html was not built');
  process.exit(1);
}
step('cargo', [
  'lambda',
  'build',
  '--release',
  '--compiler',
  'cargo',
  '--target',
  'aarch64-unknown-linux-musl',
  '--output-format',
  'zip',
  '-p',
  'sanchoris-api',
  '--bin',
  'api',
  '--bin',
  'channel',
  '--bin',
  'channel-authorizer',
  '--bin',
  'worker',
  '--bin',
  'relay',
]);
