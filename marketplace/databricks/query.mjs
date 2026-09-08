import { summarize } from './view.mjs';
// Platform adapter only: no runner, deploy, admin, or database-write capability.
export const MAX_ROWS = 200;
export const MAX_BYTES = 1024 * 1024;
export const COLUMNS = Object.freeze(['id', 'repository', 'revision', 'workflow_path', 'status', 'created_at', 'duration', 'profile']);
export class AppError extends Error {
  constructor(status, code) { super(code); this.status = status; this.code = code; }
}
export function configFrom(values) {
  let host;
  try { host = new URL(values.DATABRICKS_HOST); } catch { throw new AppError(503, 'invalid_configuration'); }
  const allowed = ['.cloud.databricks.com', '.azuredatabricks.net', '.gcp.databricks.com'];
  if (host.protocol !== 'https:' || host.username || host.password || host.port || host.search || host.hash
      || host.pathname !== '/' || !allowed.some(suffix => host.hostname.endsWith(suffix))) {
    throw new AppError(503, 'invalid_configuration');
  }
  const warehouse = values.DATABRICKS_WAREHOUSE_ID;
  const view = values.INDIEBUILD_RELEASE_VIEW;
  if (typeof warehouse !== 'string' || !/^[A-Za-z0-9_-]{1,128}$/.test(warehouse)
      || typeof view !== 'string' || !/^[A-Za-z_][A-Za-z0-9_]{0,62}(\.[A-Za-z_][A-Za-z0-9_]{0,62}){2}$/.test(view)) {
    throw new AppError(503, 'invalid_configuration');
  }
  return Object.freeze({ host: host.origin, warehouse, view: view.split('.').map(s => `\`${s}\``).join('.') });
}
function tokenFrom(headers) {
  const token = headers.get('x-forwarded-access-token');
  if (!token || token.length > 8192 || /[\s,\x00-\x1f\x7f]/.test(token)) throw new AppError(401, 'user_authorization_required');
  return token;
}
function repositoryFilter(value) {
  if (value === null || value === '') return null;
  if (typeof value !== 'string' || value.length > 200 || !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(value)
      || value.split('/').some(s => s === '.' || s === '..')) throw new AppError(400, 'invalid_repository');
  return value;
}
async function boundedJson(response) {
  if (!response.ok) {
    if (response.status === 401) throw new AppError(401, 'user_authorization_required');
    throw new AppError(response.status === 403 ? 403 : 503, response.status === 403 ? 'data_access_denied' : 'warehouse_unavailable');
  }
  if (!response.body) throw new AppError(503, 'invalid_warehouse_response');
  const chunks = []; let size = 0;
  try {
    for await (const chunk of response.body) {
      size += chunk.byteLength;
      if (size > MAX_BYTES) throw new AppError(503, 'response_too_large');
      chunks.push(chunk);
    }
    return JSON.parse(Buffer.concat(chunks).toString('utf8'));
  } catch (error) {
    if (error instanceof AppError) throw error;
    throw new AppError(503, 'invalid_warehouse_response');
  }
}
function rowsFrom(body) {
  if (body?.status?.state !== 'SUCCEEDED' || body?.manifest?.truncated === true) throw new AppError(503, 'incomplete_query');
  const columns = body?.manifest?.schema?.columns;
  const rows = body?.result?.data_array ?? (body?.manifest?.total_row_count === 0 ? [] : undefined);
  if (!Array.isArray(columns) || columns.length !== COLUMNS.length || columns.some((c, i) => c?.name !== COLUMNS[i])
      || !Array.isArray(rows) || rows.length > MAX_ROWS || body?.result?.next_chunk_index != null
      || (body?.manifest?.total_chunk_count ?? 1) > 1) throw new AppError(503, 'invalid_warehouse_response');
  if (!Number.isSafeInteger(body.manifest.total_row_count) || body.manifest.total_row_count !== rows.length
      || body.manifest.format !== 'JSON_ARRAY' || body.manifest.schema.column_count !== COLUMNS.length
      || columns.some((column, index) => column?.position !== index)
      || body?.result?.external_links != null
      || (body?.result?.chunk_index != null && body.result.chunk_index !== 0)
      || (body?.result?.row_offset != null && body.result.row_offset !== 0)
      || (body?.result?.row_count != null && body.result.row_count !== rows.length)) {
    throw new AppError(503, 'invalid_warehouse_response');
  }
  const identities = new Set();
  return rows.map(row => {
    if (!Array.isArray(row) || row.length !== COLUMNS.length
        || row.some((value, i) => (value === null && i >= 6) ? false : typeof value !== 'string' || value.length > [128, 200, 40, 512, 32, 64, 64, 128][i])) {
      throw new AppError(503, 'invalid_release_row');
    }
    if (!/^[A-Za-z0-9_-]{1,128}$/.test(row[0]) || !/^[a-fA-F0-9]{40}$/.test(row[2])) throw new AppError(503, 'invalid_release_row');
    try { if (!repositoryFilter(row[1])) throw new Error('empty_repository'); } catch { throw new AppError(503, 'invalid_release_row'); }
    const identity = `${row[1]}:${row[0]}`;
    if (identities.has(identity)) throw new AppError(503, 'invalid_release_row');
    identities.add(identity);
    return { id: row[0], repository: row[1], revision: row[2], workflowPath: row[3], status: row[4], createdAt: row[5], duration: row[6], profile: row[7] };
  });
}
export async function queryReleases(config, headers, repository = null, fetcher = fetch) {
  const token = tokenFrom(headers);
  const filter = repositoryFilter(repository);
  const statement = `SELECT ${COLUMNS.map(c => `CAST(\`${c}\` AS STRING) AS \`${c}\``).join(', ')} FROM ${config.view} AS release_runs${filter ? ' WHERE release_runs.`repository` = :repository' : ''} ORDER BY release_runs.\`created_at\` DESC, release_runs.\`id\` DESC LIMIT ${MAX_ROWS}`;
  const parameters = filter ? [{ name: 'repository', value: filter, type: 'STRING' }] : [];
  let response;
  try {
    response = await fetcher(`${config.host}/api/2.0/sql/statements`, {
      method: 'POST', redirect: 'error', signal: AbortSignal.timeout(15000),
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      body: JSON.stringify({ warehouse_id: config.warehouse, statement, parameters,
        disposition: 'INLINE', format: 'JSON_ARRAY', byte_limit: MAX_BYTES, wait_timeout: '10s', on_wait_timeout: 'CANCEL' })
    });
  } catch { throw new AppError(503, 'warehouse_unavailable'); }
  const runs = rowsFrom(await boundedJson(response));
  if (filter && runs.some(run => run.repository !== filter)) throw new AppError(503, 'invalid_warehouse_response');
  return { schemaVersion: 1, source: 'databricks-user-authorized-view', coverage: 'latest-200-matching-runs',
    readOnly: true, promotionAllowed: false, evidenceState: 'not-collected', observedAt: new Date().toISOString(), counts: summarize(runs), runs };
}
