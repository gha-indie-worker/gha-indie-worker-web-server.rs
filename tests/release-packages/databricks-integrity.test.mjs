import { test } from 'node:test';
import assert from 'node:assert/strict';
import { configFrom, queryReleases, COLUMNS } from '../../marketplace/databricks/query.mjs';
const config = configFrom({ DATABRICKS_HOST: 'https://workspace.cloud.databricks.com', DATABRICKS_WAREHOUSE_ID: 'warehouse_1', INDIEBUILD_RELEASE_VIEW: 'catalog.schema.releases' });
const headers = new Headers({ 'x-forwarded-access-token': 'test-user-token' });
const row = ['run_1', 'org/repo', 'a'.repeat(40), 'ci.yml', 'success', '2026-09-08T10:00:00Z', null, null];
function payload(rows = [row]) {
  return { status: { state: 'SUCCEEDED' }, manifest: { format: 'JSON_ARRAY', total_row_count: rows.length, total_chunk_count: 1, schema: { column_count: COLUMNS.length, columns: COLUMNS.map((name, position) => ({ name, position })) } }, result: { chunk_index: 0, row_offset: 0, row_count: rows.length, data_array: rows } };
}
const run = (value, repository = null) => queryReleases(config, headers, repository, async () => Response.json(value));
for (const [name, mutate] of [
  ['total row count mismatch', p => { p.manifest.total_row_count = 2; }],
  ['missing total count', p => { delete p.manifest.total_row_count; }],
  ['chunk row count mismatch', p => { p.result.row_count = 2; }],
  ['nonzero chunk offset', p => { p.result.row_offset = 1; }],
  ['nonzero first chunk', p => { p.result.chunk_index = 1; }],
  ['column count mismatch', p => { p.manifest.schema.column_count = 9; }],
  ['wrong column position', p => { p.manifest.schema.columns[0].position = 7; }],
  ['external links', p => { p.result.external_links = [{ external_link: 'https://example.test' }]; }],
  ['duplicate run identities', p => { p.result.data_array.push([...row]); p.result.row_count = 2; p.manifest.total_row_count = 2; }]
]) test(`rejects ${name}`, async () => {
  const p = payload(); mutate(p);
  await assert.rejects(() => run(p), e => e.status === 503);
});
test('rejects response outside the exact requested repository', async () => {
  await assert.rejects(() => run(payload(), 'other/repo'), e => e.status === 503);
});
test('sorts original timestamp column rather than its display cast', async () => {
  await queryReleases(config, headers, null, async (_, options) => {
    assert.match(JSON.parse(options.body).statement, /ORDER BY release_runs\.`created_at` DESC, release_runs\.`id` DESC/);
    return Response.json(payload());
  });
});
