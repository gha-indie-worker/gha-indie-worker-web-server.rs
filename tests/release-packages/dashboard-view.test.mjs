import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readState, stateParams, selectView, statusGroup, summarize } from '../../marketplace/databricks/view.mjs';
const state = query => readState(new URLSearchParams(query));
for (const status of ['completed', 'cancelled', 'canceled', 'skipped', 'neutral', 'action_required', 'SUCCESS', '', 'new-status']) {
  test(`status ${JSON.stringify(status)} is never counted as success`, () => assert.equal(statusGroup(status), 'other'));
}
test('recognized successes, failures and active states have explicit groups', () => {
  for (const status of ['success', 'succeeded', 'completed_success']) assert.equal(statusGroup(status), 'success');
  for (const status of ['failure', 'failed', 'timed_out', 'startup_failure', 'completed_failure']) assert.equal(statusGroup(status), 'failure');
  for (const status of ['queued', 'pending', 'waiting', 'running', 'in_progress', 'requested']) assert.equal(statusGroup(status), 'active');
});
for (const query of ['token=never', 'status=success&status=failure', 'repository=a/b&repository=c/d', 'page=0', 'page=-1', 'page=1.5', 'page=Infinity', 'page=21', 'pageSize=200', 'status=ready', 'repository=../private']) {
  test(`rejects invalid saved state ${query}`, () => assert.throws(() => state(query), /invalid_filters/));
}
test('canonical URL state round-trips without credentials or default noise', () => {
  assert.equal(stateParams(state('')).toString(), '');
  const original = state('repository=org/repo&status=failure&page=2&pageSize=10');
  assert.deepEqual(readState(stateParams(original)), original);
});
test('pagination preserves observed order, original input, and window totals', () => {
  const runs = Array.from({ length: 31 }, (_, i) => Object.freeze({ id: String(i), repository: 'org/repo', status: i % 2 ? 'failure' : 'success' }));
  Object.freeze(runs);
  const view = selectView(runs, state('repository=org/repo&status=failure&page=2&pageSize=10'));
  assert.deepEqual(view.rows.map(r => r.id), ['21','23','25','27','29']);
  assert.deepEqual([view.from, view.to, view.page, view.pages, view.matched], [11,15,2,2,15]);
  assert.deepEqual(view.counts, { total:31, success:16, failure:15, active:0, other:0 });
});
test('out-of-range saved page clamps to the available window', () => {
  const view = selectView([{ repository:'org/repo', status:'success' }], state('page=20&pageSize=10'));
  assert.equal(view.page, 1); assert.equal(view.rows.length, 1);
});
test('empty and filtered-empty snapshots do not imply healthy CI', () => {
  for (const runs of [[], [{ repository: 'org/repo', status: 'skipped' }]]) {
    const view = selectView(runs, state('status=success'));
    assert.deepEqual([view.matched, view.from, view.to, view.pages, view.counts.success], [0,0,0,1,0]);
  }
});
test('summary partitions every input status without dropping unknown observations', () => {
  const values = ['success','failure','queued','running','skipped','future-status'];
  for (let size = 0; size <= 200; size++) {
    const counts = summarize(Array.from({ length:size }, (_, i) => ({ status:values[i % values.length] })));
    assert.equal(counts.success + counts.failure + counts.active + counts.other, size);
  }
});
test('presentation rejects unbounded source data', () => assert.throws(() => selectView(Array(201).fill({}), state('')), /invalid_snapshot/));
test('repository filtering is exact, including mixed case', () => {
  const runs = [{ repository:'Org/repo',status:'success' },{ repository:'org/repo',status:'failed' }];
  assert.equal(selectView(runs, state('repository=org/repo')).rows[0].status, 'failed');
});
