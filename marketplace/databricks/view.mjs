// Presentation-only state. Status groups describe reported builds, never release approval.
export const GROUPS = Object.freeze(['all', 'success', 'failure', 'active', 'other']);
export const PAGE_SIZES = Object.freeze([10, 25, 50]);
export function statusGroup(status) {
  if (['success', 'succeeded', 'completed_success'].includes(status)) return 'success';
  if (['failure', 'failed', 'timed_out', 'completed_failure', 'startup_failure'].includes(status)) return 'failure';
  if (['queued', 'pending', 'waiting', 'requested', 'running', 'in_progress'].includes(status)) return 'active';
  return 'other';
}
export function summarize(runs) {
  const counts = { total: runs.length, success: 0, failure: 0, active: 0, other: 0 };
  for (const run of runs) counts[statusGroup(run.status)] += 1;
  return Object.freeze(counts);
}
export function readState(params) {
  const allowed = ['repository', 'status', 'page', 'pageSize'];
  if ([...params.keys()].some(k => !allowed.includes(k)) || allowed.some(k => params.getAll(k).length > 1)) throw new Error('invalid_filters');
  const repository = params.get('repository') ?? '';
  const status = params.get('status') ?? 'all';
  const pageText = params.get('page') ?? '1';
  const sizeText = params.get('pageSize') ?? '25';
  if (repository.length > 200 || (repository !== '' && (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository) || repository.split('/').some(p => p === '.' || p === '..')))
      || !GROUPS.includes(status) || !/^[1-9][0-9]*$/.test(pageText) || !/^[1-9][0-9]*$/.test(sizeText)
      || Number(pageText) > 20 || !PAGE_SIZES.includes(Number(sizeText))) throw new Error('invalid_filters');
  return Object.freeze({ repository, status, page: Number(pageText), pageSize: Number(sizeText) });
}
export function stateParams(state) {
  const params = new URLSearchParams();
  if (state.repository) params.set('repository', state.repository);
  if (state.status !== 'all') params.set('status', state.status);
  if (state.page !== 1) params.set('page', String(state.page));
  if (state.pageSize !== 25) params.set('pageSize', String(state.pageSize));
  return params;
}
export function selectView(runs, state) {
  if (!Array.isArray(runs) || runs.length > 200) throw new Error('invalid_snapshot');
  // Revalidate programmatic callers, not only URLs. Never mutate upstream ordering.
  const checked = readState(stateParams(state));
  const selected = runs.filter(run => (checked.repository === '' || run.repository === checked.repository)
    && (checked.status === 'all' || statusGroup(run.status) === checked.status));
  const pages = Math.max(1, Math.ceil(selected.length / checked.pageSize));
  const page = Math.min(checked.page, pages);
  const offset = (page - 1) * checked.pageSize;
  return { counts: summarize(runs), matched: selected.length, page, pages,
    rows: selected.slice(offset, offset + checked.pageSize), from: selected.length ? offset + 1 : 0,
    to: Math.min(offset + checked.pageSize, selected.length) };
}
