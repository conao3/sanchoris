import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import {
  aws,
  awsAsync,
  awsProcessEnv,
  environment,
  exportsOf,
  fail,
  region,
  run,
  runLogged,
} from './lib/aws-env.mjs';

const usage = 'usage: node scripts/deploy.mjs <dev|prd> [--plan]';

if (!process.env.IN_NIX_SHELL) {
  const result = spawnSync('nix', ['develop', '--command', 'node', ...process.argv.slice(1)], { stdio: 'inherit' });
  process.exit(result.status ?? 1);
}

const args = process.argv.slice(2);
const planOnly = args.includes('--plan');
const positional = args.filter((arg) => !arg.startsWith('--'));
if (positional.length !== 1 || args.some((arg) => arg.startsWith('--') && arg !== '--plan')) {
  fail(usage);
}
const env = environment(positional[0]);

const infraDir = process.env.AWS_INFRA_K8S_DIR;
if (!infraDir || !existsSync(join(infraDir, 'src/conao3/aws_infra_k8s/sanchoris.clj'))) {
  fail('set AWS_INFRA_K8S_DIR to the aws-infra-k8s checkout');
}

const root = run('git', ['rev-parse', '--show-toplevel']).trim();
const logDir = join(root, '.sanchoris', 'deploy', env.name);
const lambdaFunctions = ['api', 'channel-authorizer', 'channel', 'worker', 'relay'];
const stackName = `${env.prefix}-sanchoris`;
const stackSettings = {
  SANCHORIS_ENABLE_WORKSPACES: 'true',
  SANCHORIS_WORKSPACE_NETWORK: 'public',
  SANCHORIS_WORKER_MAX_CONCURRENCY: '2',
};

const startedAt = Date.now();
const steps = [];

async function step(name, body) {
  const t0 = Date.now();
  process.stderr.write(`deploy ${env.name}: ${name}\n`);
  try {
    const detail = await body();
    steps.push({ name, ...detail, seconds: (Date.now() - t0) / 1000 });
  } catch (error) {
    steps.push({ name, result: 'failed', error: error.message, seconds: (Date.now() - t0) / 1000 });
    throw error;
  }
}

function contentHash(paths, extra) {
  const files = run('git', ['ls-files', '-co', '--exclude-standard', '-z', '--', ...paths], { cwd: root })
    .split('\0')
    .filter((file) => file && existsSync(join(root, file)))
    .sort();
  const hash = createHash('sha256');
  hash.update(JSON.stringify(extra));
  for (const file of files) {
    hash.update(`${file}\0`);
    hash.update(readFileSync(join(root, file)));
    hash.update('\0');
  }
  return hash.digest('hex').slice(0, 16);
}

const lambdaInputs = [
  'Cargo.toml',
  'Cargo.lock',
  'flake.lock',
  'package.json',
  'pnpm-lock.yaml',
  'pnpm-workspace.yaml',
  'scripts/build-lambda.mjs',
  'crates',
  'apps/frontend',
  'infra/tables.json',
  'infra/plans.json',
];

async function buildLambda(cognito) {
  const key = contentHash(lambdaInputs, { ...cognito, profile: env.lambdaProfile });
  const cacheRoot = join(root, 'target', 'lambda-cache', env.name);
  const cacheDir = join(cacheRoot, key);
  const outputDir = join(root, 'target', 'lambda');
  const cached = lambdaFunctions.every((fn) => existsSync(join(cacheDir, fn, 'bootstrap.zip')));
  if (planOnly) {
    return { result: cached ? 'would restore' : 'would build', profile: env.lambdaProfile, key };
  }
  if (!cached) {
    await runLogged('pnpm', ['build:lambda', env.lambdaProfile], {
      cwd: root,
      env: { ...process.env, VITE_COGNITO_DOMAIN: cognito.domain, VITE_COGNITO_CLIENT_ID: cognito.clientId },
      logFile: join(logDir, 'build-lambda.log'),
    });
    if (existsSync(cacheRoot)) {
      for (const entry of readdirSync(cacheRoot)) {
        rmSync(join(cacheRoot, entry), { recursive: true, force: true });
      }
    }
    for (const fn of lambdaFunctions) {
      mkdirSync(join(cacheDir, fn), { recursive: true });
      copyFileSync(join(outputDir, fn, 'bootstrap.zip'), join(cacheDir, fn, 'bootstrap.zip'));
    }
    return { result: 'built', profile: env.lambdaProfile, key };
  }
  for (const fn of lambdaFunctions) {
    mkdirSync(join(outputDir, fn), { recursive: true });
    copyFileSync(join(cacheDir, fn, 'bootstrap.zip'), join(outputDir, fn, 'bootstrap.zip'));
  }
  return { result: 'skipped', reason: 'restored from target/lambda-cache', profile: env.lambdaProfile, key };
}

function images(exports) {
  const workspaceRepo = exports.get('SanchorisWorkspaceRepositoryUri');
  const relayRepo = exports.get('SanchorisRelayRepositoryUri');
  const workspaceInputs = ['infra/workspace', 'package.json', 'pnpm-lock.yaml', 'pnpm-workspace.yaml', 'apps/workspace-agent'];
  return [
    {
      name: 'workspace-arm64',
      repository: workspaceRepo,
      tag: 'latest-arm64',
      platform: 'linux/arm64',
      dockerfile: 'infra/workspace/Dockerfile',
      context: '.',
      inputs: workspaceInputs,
    },
    {
      name: 'workspace-amd64',
      repository: workspaceRepo,
      tag: 'latest-amd64',
      platform: 'linux/amd64',
      dockerfile: 'infra/workspace/Dockerfile',
      context: '.',
      inputs: workspaceInputs,
    },
    {
      name: 'relay',
      repository: relayRepo,
      tag: 'latest',
      platform: 'linux/arm64',
      dockerfile: 'infra/relay/Dockerfile',
      context: 'apps/egress-relay',
      inputs: ['infra/relay', 'apps/egress-relay/package.json', 'apps/egress-relay/src'],
    },
  ];
}

function describeImage(repositoryName, tag) {
  const result = spawnSync(
    'aws',
    ['ecr', 'describe-images', '--repository-name', repositoryName, '--image-ids', `imageTag=${tag}`, '--output', 'json'],
    { encoding: 'utf8', env: awsProcessEnv(env) },
  );
  if (result.status !== 0) {
    if (result.stderr.includes('ImageNotFoundException')) {
      return null;
    }
    throw new Error(`aws ecr describe-images ${repositoryName}:${tag} failed\n${result.stderr.trim()}`);
  }
  return JSON.parse(result.stdout).imageDetails[0];
}

let dockerLoggedIn = false;

function dockerLogin(registry) {
  if (dockerLoggedIn) {
    return;
  }
  const password = run('aws', ['ecr', 'get-login-password'], { env: awsProcessEnv(env) });
  run('docker', ['login', '--username', 'AWS', '--password-stdin', registry], { input: password });
  dockerLoggedIn = true;
}

async function pushImage(image) {
  const sourceTag = `src-${contentHash(image.inputs, { platform: image.platform, dockerfile: image.dockerfile })}`;
  const repositoryName = image.repository.split('/').slice(1).join('/');
  const existing = describeImage(repositoryName, sourceTag);
  if (existing?.imageTags.includes(image.tag)) {
    return { result: 'skipped', reason: `${image.tag} is already ${sourceTag}`, sourceTag };
  }
  if (existing) {
    if (planOnly) {
      return { result: 'would retag', sourceTag };
    }
    const manifest = aws(env, [
      'ecr',
      'batch-get-image',
      '--repository-name',
      repositoryName,
      '--image-ids',
      `imageTag=${sourceTag}`,
    ]).images[0].imageManifest;
    aws(env, ['ecr', 'put-image', '--repository-name', repositoryName, '--image-tag', image.tag, '--image-manifest', manifest]);
    return { result: 'retagged', sourceTag };
  }
  if (planOnly) {
    return { result: 'would build and push', sourceTag };
  }
  dockerLogin(image.repository.split('/')[0]);
  const logFile = join(logDir, `image-${image.name}.log`);
  await runLogged(
    'docker',
    [
      'build',
      '--platform',
      image.platform,
      '-f',
      join(root, image.dockerfile),
      '-t',
      `${image.repository}:${image.tag}`,
      '-t',
      `${image.repository}:${sourceTag}`,
      join(root, image.context),
    ],
    { cwd: root, env: process.env, logFile },
  );
  for (const tag of [sourceTag, image.tag]) {
    await runLogged('docker', ['push', `${image.repository}:${tag}`], {
      cwd: root,
      env: process.env,
      logFile: join(logDir, `push-${image.name}-${tag}.log`),
    });
  }
  return { result: 'pushed', sourceTag };
}

function stack() {
  const result = spawnSync(
    'aws',
    ['cloudformation', 'describe-stacks', '--stack-name', stackName, '--output', 'json'],
    { encoding: 'utf8', env: awsProcessEnv(env) },
  );
  if (result.status !== 0) {
    if (result.stderr.includes('does not exist')) {
      return null;
    }
    throw new Error(`aws cloudformation describe-stacks failed\n${result.stderr.trim()}`);
  }
  const { StackStatus, LastUpdatedTime, CreationTime, Outputs } = JSON.parse(result.stdout).Stacks[0];
  return {
    status: StackStatus,
    lastUpdated: LastUpdatedTime ?? CreationTime,
    templateHash: Outputs?.find((output) => output.OutputKey === 'SanchorisTemplateHash')?.OutputValue ?? null,
  };
}

const infraJarInputs = ['src', 'resources', 'deps.edn', 'build.clj'];

async function infraJar() {
  const files = run('git', ['ls-files', '-co', '--exclude-standard', '-z', '--', ...infraJarInputs], { cwd: infraDir })
    .split('\0')
    .filter((file) => file && existsSync(join(infraDir, file)))
    .sort();
  const hash = createHash('sha256');
  for (const file of files) {
    hash.update(`${file}\0`);
    hash.update(readFileSync(join(infraDir, file)));
    hash.update('\0');
  }
  const jar = join(homedir(), '.cache', 'sanchoris-deploy', `aws-infra-k8s-${hash.digest('hex').slice(0, 16)}.jar`);
  if (existsSync(jar)) {
    return { jar, result: 'skipped', reason: 'uberjar of the same sources' };
  }
  await runLogged('nix', ['develop', '--command', 'clojure', '-T:build', 'uberjar'], {
    cwd: infraDir,
    env: process.env,
    logFile: join(logDir, 'infra-uberjar.log'),
  });
  const built = readdirSync(join(infraDir, 'target')).find((file) => file.endsWith('-standalone.jar'));
  mkdirSync(dirname(jar), { recursive: true });
  copyFileSync(join(infraDir, 'target', built), jar);
  return { jar, result: 'built' };
}

function infra(jar, args, logName) {
  return runLogged('nix', ['develop', '--command', 'java', '-jar', jar, ...args], {
    cwd: infraDir,
    env: awsProcessEnv(env, { DEPLOY_ENV: env.deployEnv, SANCHORIS_DIR: root, ...stackSettings }),
    logFile: join(logDir, `${logName}.log`),
  });
}

async function renderTemplate(jar) {
  const out = await infra(jar, ['template', 'sanchoris'], 'template');
  const { templateHash } = JSON.parse(out.trim().split('\n').pop());
  const deployed = stack();
  const direct = env.directCodeUpdates && deployed?.templateHash === templateHash;
  return {
    result: 'rendered',
    templateHash,
    deployedTemplateHash: deployed?.templateHash ?? null,
    route: direct ? 'lambda update-function-code' : 'cloudformation',
  };
}

function functionName(fn) {
  return `${env.prefix}-sanchoris-${fn}`;
}

async function updateFunctionCode(fn) {
  const zip = join(root, 'target', 'lambda', fn, 'bootstrap.zip');
  if (planOnly && !existsSync(zip)) {
    return { fn, result: 'would update' };
  }
  const local = createHash('sha256').update(readFileSync(zip)).digest('base64');
  const deployed = (await awsAsync(env, ['lambda', 'get-function-configuration', '--function-name', functionName(fn)])).CodeSha256;
  if (deployed === local) {
    return { fn, result: 'unchanged' };
  }
  if (planOnly) {
    return { fn, result: 'would update' };
  }
  await runLogged(
    'aws',
    ['lambda', 'update-function-code', '--function-name', functionName(fn), '--zip-file', `fileb://${zip}`, '--output', 'json'],
    { env: awsProcessEnv(env), logFile: join(logDir, `update-${fn}.log`) },
  );
  await runLogged('aws', ['lambda', 'wait', 'function-updated-v2', '--function-name', functionName(fn)], {
    env: awsProcessEnv(env),
    logFile: join(logDir, `wait-${fn}.log`),
  });
  return { fn, result: 'updated' };
}

async function updateFunctions() {
  const results = await Promise.all(lambdaFunctions.map(updateFunctionCode));
  return {
    result: results.some((r) => r.result === 'updated') ? 'updated' : 'unchanged',
    functions: Object.fromEntries(results.map((r) => [r.fn, r.result])),
  };
}

async function deployStack(jar) {
  if (planOnly) {
    return { result: 'would deploy', before: stack() };
  }
  const before = stack();
  await infra(jar, ['deploy', 'sanchoris'], 'deploy-sanchoris');
  let after = stack();
  const waiter = {
    CREATE_IN_PROGRESS: 'stack-create-complete',
    UPDATE_IN_PROGRESS: 'stack-update-complete',
    UPDATE_COMPLETE_CLEANUP_IN_PROGRESS: 'stack-update-complete',
  }[after.status];
  if (waiter) {
    run('aws', ['cloudformation', 'wait', waiter, '--stack-name', stackName], { env: awsProcessEnv(env) });
    after = stack();
  }
  if (!['CREATE_COMPLETE', 'UPDATE_COMPLETE'].includes(after.status)) {
    throw new Error(`stack ${stackName} is ${after.status}; log: ${join(logDir, 'deploy-sanchoris.log')}`);
  }
  const changed = !before || before.lastUpdated !== after.lastUpdated;
  return { result: changed ? 'updated' : 'unchanged', stack: { name: stackName, ...after } };
}

async function buildSeeder() {
  if (planOnly) {
    return { result: 'would build' };
  }
  await runLogged('cargo', ['build', '-q', '-p', 'sanchoris-core', '--bin', 'sanchoris-dynamo-create'], {
    cwd: root,
    env: process.env,
    logFile: join(logDir, 'build-seeder.log'),
  });
  return { result: 'built' };
}

async function seedPlans(exports) {
  if (planOnly) {
    return { result: 'would seed' };
  }
  const prices = aws(env, [
    'ssm',
    'get-parameters',
    '--names',
    ...['starter', 'pro', 'max'].map((plan) => `/${env.prefix}/sanchoris/stripe-price-${plan}`),
  ]);
  if (prices.InvalidParameters.length > 0) {
    throw new Error(`missing SSM parameters: ${prices.InvalidParameters.join(' ')}`);
  }
  const price = (plan) => prices.Parameters.find((p) => p.Name.endsWith(`stripe-price-${plan}`)).Value;
  await runLogged(join(root, 'target', 'debug', 'sanchoris-dynamo-create'), [], {
    cwd: root,
    env: awsProcessEnv(env, {
      SANCHORIS_TABLE_MAIN: exports.get('SanchorisMainTable'),
      SANCHORIS_TABLE_AUDIT: exports.get('SanchorisAuditTable'),
      SANCHORIS_STRIPE_PRICE_STARTER: price('starter'),
      SANCHORIS_STRIPE_PRICE_PRO: price('pro'),
      SANCHORIS_STRIPE_PRICE_MAX: price('max'),
    }),
    logFile: join(logDir, 'seed-plans.log'),
  });
  return { result: 'seeded' };
}

let ok = true;
try {
  let exports;
  let cognito;
  await step('read exports', () => {
    exports = exportsOf(env);
    cognito = { domain: exports.get('UserPoolDomain'), clientId: exports.get('SanchorisUserPoolClient') };
    return { result: 'read', cognito };
  });
  let jar;
  let route;
  const artifacts = await Promise.allSettled([
    step('infra uberjar', async () => {
      const detail = await infraJar();
      jar = detail.jar;
      return detail;
    }).then(() =>
      step('render template', async () => {
        const detail = await renderTemplate(jar);
        route = detail.route;
        return detail;
      }),
    ),
    step('build lambda', () => buildLambda(cognito)).then(() => step('build seeder', buildSeeder)),
    ...images(exports).map((image) => step(`image ${image.name}`, () => pushImage(image))),
  ]);
  const failed = artifacts.filter((artifact) => artifact.status === 'rejected');
  if (failed.length > 0) {
    throw new Error(failed.map((artifact) => artifact.reason.message).join('\n'));
  }
  await step('seed plans', () => seedPlans(exports));
  if (route === 'cloudformation') {
    await step('deploy stack', () => deployStack(jar));
  }
  await step(route === 'cloudformation' ? 'converge function code' : 'update function code', updateFunctions);
} catch (error) {
  ok = false;
  process.stderr.write(`deploy ${env.name}: ${error.message}\n`);
}

const finalStack = stack();
console.log(
  JSON.stringify(
    {
      env: env.name,
      ok,
      plan: planOnly,
      seconds: (Date.now() - startedAt) / 1000,
      steps,
      skipped: steps.filter((s) => s.result === 'skipped').map((s) => s.name),
      stack: finalStack && { name: stackName, region, ...finalStack },
      logs: logDir,
    },
    null,
    2,
  ),
);
process.exit(ok ? 0 : 1);
