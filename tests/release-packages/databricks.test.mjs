import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { AppError, configFrom, queryReleases, COLUMNS, MAX_BYTES } from '../../marketplace/databricks/query.mjs';
import { handle } from '../../marketplace/databricks/server.mjs';
const values = { DATABRICKS_HOST: 'https://workspace.cloud.databricks.com', DATABRICKS_WAREHOUSE_ID: 'warehouse_1', INDIEBUILD_RELEASE_VIEW: 'catalog.schema.releases' };
const config = configFrom(values);
const headers = new Headers({ 'x-forwarded-access-token': 'test-user-token' });
const row = ['run_1', 'gha-indie-worker/worker', 'a'.repeat(40), '.github/workflows/ci.yml', 'success', '2026-09-08T10:00:00Z', null, 'linux'];
const payload = rows => ({ status: { state: 'SUCCEEDED' }, manifest: { format: 'JSON_ARRAY', total_row_count: rows.length, total_chunk_count: 1, schema: { column_count: COLUMNS.length, columns: COLUMNS.map((name, position) => ({ name, position })) } }, result: { data_array: rows } });
const ok = rows => async () => Response.json(payload(rows));
const rejects = (fn, code) => assert.rejects(fn, e => e instanceof AppError && e.code === code);

test('uses only caller identity, parameterized filter, bounded SELECT, and cancellation', async () => {
  let called = false;
  const snapshot = await queryReleases(config, headers, 'gha-indie-worker/worker', async (url, options) => {
    called = true;
    assert.equal(url, 'https://workspace.cloud.databricks.com/api/2.0/sql/statements');
    assert.equal(options.headers.authorization, 'Bearer test-user-token');
    assert.equal(options.redirect, 'error');
    const body = JSON.parse(options.body);
    assert.match(body.statement, /^SELECT .* FROM `catalog`\.`schema`\.`releases` AS release_runs WHERE release_runs\.`repository` = :repository ORDER BY release_runs\.`created_at` DESC, release_runs\.`id` DESC LIMIT 200$/);
    assert.equal(body.statement.includes('gha-indie-worker/worker'), false);
    assert.deepEqual(body.parameters, [{ name: 'repository', value: 'gha-indie-worker/worker', type: 'STRING' }]);
    assert.equal(body.on_wait_timeout, 'CANCEL');
    return Response.json(payload([row]));
  });
  assert.ok(called); assert.equal(snapshot.runs.length, 1); assert.equal(snapshot.promotionAllowed, false);
  assert.equal(snapshot.evidenceState, 'not-collected');
});
test('requires forwarded user authorization and never substitutes an app token', async () => {
  let called = false;
  await rejects(() => queryReleases(config, new Headers({ authorization: 'Bearer app-token' }), null, async () => { called = true; }), 'user_authorization_required');
  assert.equal(called, false);
});
for (const value of ['https://github.com', 'http://workspace.cloud.databricks.com', 'https://user:secret@workspace.cloud.databricks.com', 'https://workspace.cloud.databricks.com.evil.test', 'https://workspace.cloud.databricks.com/path', 'https://workspace.cloud.databricks.com?token=secret']) {
  test(`rejects unsafe workspace configuration ${value.replace(/secret/g, 'redacted')}`, () => assert.throws(() => configFrom({ ...values, DATABRICKS_HOST: value }), AppError));
}
test('rejects SQL identifiers and filter injection before I/O', async () => {
  for (const view of ['a.b.c;DROP TABLE x', 'a.b', 'a.b.`c`', 'a.b.c--']) assert.throws(() => configFrom({ ...values, INDIEBUILD_RELEASE_VIEW: view }), AppError);
  await rejects(() => queryReleases(config, headers, "org/repo' OR 1=1--", ok([row])), 'invalid_repository');
});
test('rejects incomplete, truncated, or paged evidence instead of partial success', async () => {
  for (const mutate of [p => { p.status.state = 'RUNNING'; }, p => { p.manifest.truncated = true; }, p => { p.manifest.total_chunk_count = 2; }, p => { p.result.next_chunk_index = 1; }]) {
    const p = payload([row]); mutate(p);
    await assert.rejects(() => queryReleases(config, headers, null, async () => Response.json(p)), AppError);
  }
});
test('rejects schema drift and oversized row count', async () => {
  const p = payload([row]); p.manifest.schema.columns[0].name = 'secret';
  await rejects(() => queryReleases(config, headers, null, async () => Response.json(p)), 'invalid_warehouse_response');
  await rejects(() => queryReleases(config, headers, null, ok(Array(201).fill(row))), 'invalid_warehouse_response');
});
test('rejects malformed identities and bounded display fields', async () => {
  for (const [index, value] of [[0, '../other'], [1, ''], [1, '../other'], [2, 'main'], [4, 'x'.repeat(33)], [6, 'x'.repeat(65)]]) {
    const bad = [...row]; bad[index] = value;
    await rejects(() => queryReleases(config, headers, null, ok([bad])), 'invalid_release_row');
  }
});
test('bounds streamed response bytes and hides transport errors', async () => {
  await rejects(() => queryReleases(config, headers, null, async () => new Response('x'.repeat(MAX_BYTES + 1))), 'response_too_large');
  await rejects(() => queryReleases(config, headers, null, async () => { throw new Error('sensitive upstream diagnostic'); }), 'warehouse_unavailable');
});
test('handles empty results without claiming healthy CI', async () => {
  const p = payload([]); delete p.result.data_array; p.manifest.total_row_count = 0;
  const snapshot = await queryReleases(config, headers, null, async () => Response.json(p));
  assert.deepEqual(snapshot.runs, []); assert.equal(snapshot.promotionAllowed, false);
});
test('requests under different identities never share a cached snapshot', async () => {
  const tokens = [];
  const fetcher = async (_, options) => { tokens.push(options.headers.authorization); return Response.json(payload([row])); };
  for (const token of ['user-a', 'user-b']) await queryReleases(config, new Headers({ 'x-forwarded-access-token': token }), null, fetcher);
  assert.deepEqual(tokens, ['Bearer user-a', 'Bearer user-b']);
});
test('HTTP adapter preserves denial, sanitizes errors and disallows mutation', async () => {
  const request = (path, options = {}) => new Request(`http://app.internal${path}`, { headers, ...options });
  assert.equal((await handle(request('/api/releases', { method: 'POST' }), config)).status, 405);
  assert.equal((await handle(request('/api/releases?tenant=other'), config)).status, 400);
  assert.equal((await handle(request('/api/releases?repository=a/b&repository=c/d'), config)).status, 400);
  const response = await handle(request('/api/releases'), config, async () => new Response('sensitive denial body', { status: 403 }));
  assert.equal(response.status, 403); assert.equal(response.headers.get('cache-control'), 'private, no-store');
  assert.deepEqual(await response.json(), { error: 'data_access_denied' });
});
test('static app has restrictive CSP and renders with text nodes, not HTML injection', async () => {
  const response = await handle(new Request('http://app.internal/'), config);
  assert.equal(response.status, 200); assert.match(response.headers.get('content-security-policy'), /default-src 'none'/);
  const source = readFileSync(new URL('../../marketplace/databricks/app.js', import.meta.url), 'utf8');
  assert.doesNotMatch(source, /innerHTML|outerHTML|insertAdjacentHTML|document\.write/);
  assert.match(source, /td\.textContent = value/);
});
