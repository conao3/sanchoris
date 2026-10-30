import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';

const profiles = { release: ['--release'], 'lambda-dev': ['--profile', 'lambda-dev'] };
const profile = process.argv[2];
if (process.argv.length !== 3 || !profiles[profile]) {
  console.error(`usage: pnpm build:lambda <${Object.keys(profiles).join('|')}>`);
  process.exit(2);
}

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

step('vp', ['run', 'build-frontend']);
if (!existsSync('apps/frontend/dist/index.html')) {
  console.error('build:lambda: apps/frontend/dist/index.html was not built');
  process.exit(1);
}
step('cargo', [
  'lambda',
  'build',
  ...profiles[profile],
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
