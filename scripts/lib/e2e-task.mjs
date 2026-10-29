export function taskName(repositoryId, issue) {
  return `github:${repositoryId}#${issue}`;
}

export function taskIndexFrom(time) {
  return new Date(time).toISOString().replace('Z', '000Z');
}

function plainValue(attribute) {
  if ('S' in attribute) return attribute.S;
  if ('N' in attribute) return Number(attribute.N);
  if ('BOOL' in attribute) return attribute.BOOL;
  if ('NULL' in attribute) return null;
  if ('L' in attribute) return attribute.L.map(plainValue);
  if ('M' in attribute) return Object.fromEntries(Object.entries(attribute.M).map(([k, v]) => [k, plainValue(v)]));
  if ('SS' in attribute) return attribute.SS;
  return attribute;
}

export function plainItem(item) {
  return Object.fromEntries(Object.entries(item).map(([k, v]) => [k, plainValue(v)]));
}

export function taskTable(dynamodb, table) {
  const query = (args) => dynamodb(['query', '--table-name', table, ...args]).Items.map(plainItem);
  const getItem = (pk, sk, consistent) =>
    dynamodb([
      'get-item',
      ...(consistent ? ['--consistent-read'] : []),
      '--table-name',
      table,
      '--key',
      JSON.stringify({ pk: { S: pk }, sk: { S: sk } }),
    ]);

  return {
    tenantIdBySlug(slug) {
      const guard = getItem(`UNIQ#slug#${slug}`, '#', false);
      if (!guard?.Item) {
        throw new Error(`tenant ${slug} not found in ${table}`);
      }
      return plainItem(guard.Item).tenant_id;
    },

    findTask(tenant, since, name) {
      return query([
        '--index-name',
        'gsi1',
        '--key-condition-expression',
        'gsi1pk = :p AND gsi1sk >= :s',
        '--expression-attribute-values',
        JSON.stringify({ ':p': { S: `T#${tenant}#TASKS` }, ':s': { S: taskIndexFrom(since) } }),
      ]).find((task) => task.name === name);
    },

    readTask(tenant, id) {
      return plainItem(getItem(`T#${tenant}#TASK#${id}`, '#TASK', true).Item);
    },

    taskItems(tenant, taskId) {
      const items = query([
        '--consistent-read',
        '--key-condition-expression',
        'pk = :p',
        '--expression-attribute-values',
        JSON.stringify({ ':p': { S: `T#${tenant}#TASK#${taskId}` } }),
      ]);
      return {
        runs: items.filter((item) => item.kind === 'run'),
        workspaces: items.filter((item) => item.kind === 'workspace'),
        outputs: items.filter((item) => item.kind === 'output'),
      };
    },

    runLogs(tenant, runId) {
      return query([
        '--key-condition-expression',
        'pk = :p AND begins_with(sk, :s)',
        '--expression-attribute-values',
        JSON.stringify({ ':p': { S: `T#${tenant}#RUN#${runId}` }, ':s': { S: 'LOG#' } }),
      ]).map((entry) => ({ seq: entry.seq, at: entry.at, turn: entry.turn, kind: entry.log_kind, data: entry.data }));
    },
  };
}
