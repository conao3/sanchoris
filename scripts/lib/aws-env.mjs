import { spawn, spawnSync } from 'node:child_process';
import { createWriteStream, mkdirSync } from 'node:fs';
import { dirname } from 'node:path';

export const region = 'ap-northeast-1';

export const environments = {
  dev: {
    deployEnv: 'dev',
    prefix: 'dev-k8s',
    profile: 'conao3.dev.k8s',
    directCodeUpdates: true,
    e2eRepo: 'sancode-org/sanchoris-e2e-dev',
  },
  prd: {
    deployEnv: 'prd',
    prefix: 'prd-k8s',
    profile: 'conao3.prd.k8s',
    directCodeUpdates: false,
    e2eRepo: 'sancode-org/sanchoris-e2e',
  },
};

export const e2eTenantSlug = 'sanchoris-e2e';

const localAwsVariables = [
  'AWS_ACCESS_KEY_ID',
  'AWS_SECRET_ACCESS_KEY',
  'AWS_SESSION_TOKEN',
  'AWS_DEFAULT_PROFILE',
  'SANCHORIS_TABLE_MAIN',
  'SANCHORIS_TABLE_AUDIT',
];

export function fail(message) {
  console.error(message);
  process.exit(1);
}

export function environment(name) {
  const env = environments[name];
  if (!env) {
    fail(`unknown environment ${JSON.stringify(name)}; expected one of ${Object.keys(environments).join(', ')}`);
  }
  return { name, ...env };
}

export function awsProcessEnv(env, extra = {}) {
  const result = { ...process.env };
  for (const key of Object.keys(result)) {
    if (localAwsVariables.includes(key) || key.startsWith('AWS_ENDPOINT_URL')) {
      delete result[key];
    }
  }
  return { ...result, AWS_PROFILE: env.profile, AWS_REGION: region, ...extra };
}

export function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    encoding: 'utf8',
    maxBuffer: 256 * 1024 * 1024,
    ...options,
  });
  if (result.error) {
    throw new Error(`${command} failed to start: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(' ')} exited with ${result.status}\n${(result.stderr ?? '').trim()}`);
  }
  return result.stdout;
}

export function aws(env, args, input) {
  const out = run('aws', [...args, '--output', 'json'], { env: awsProcessEnv(env), input });
  return out.trim() === '' ? null : JSON.parse(out);
}

export function exportsOf(env) {
  const exports = new Map(aws(env, ['cloudformation', 'list-exports']).Exports.map((item) => [item.Name, item.Value]));
  return {
    get(suffix) {
      const name = `${env.prefix}-${suffix}`;
      const value = exports.get(name);
      if (!value) {
        fail(`CloudFormation export ${name} not found in ${env.profile}`);
      }
      return value;
    },
  };
}

export function runLogged(command, args, { cwd, env, logFile }) {
  mkdirSync(dirname(logFile), { recursive: true });
  const log = createWriteStream(logFile);
  return new Promise((resolvePromise, rejectPromise) => {
    const child = spawn(command, args, { cwd, env, stdio: ['ignore', 'pipe', 'pipe'] });
    const stdout = [];
    child.stdout.on('data', (chunk) => stdout.push(chunk));
    child.stdout.pipe(log, { end: false });
    child.stderr.pipe(log, { end: false });
    child.on('error', (error) => rejectPromise(new Error(`${command} failed to start: ${error.message}`)));
    child.on('close', (code) => {
      log.end();
      if (code === 0) {
        resolvePromise(Buffer.concat(stdout).toString('utf8'));
      } else {
        rejectPromise(new Error(`${command} ${args.join(' ')} exited with ${code}; log: ${logFile}`));
      }
    });
  });
}

export function sleep(ms) {
  return new Promise((resolvePromise) => setTimeout(resolvePromise, ms));
}

export function stringAttribute(item, name) {
  return item?.[name]?.S;
}
