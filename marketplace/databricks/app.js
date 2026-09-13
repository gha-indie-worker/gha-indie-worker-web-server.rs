import { readState, stateParams, selectView } from './view.mjs';
const form = document.querySelector('#filter');
const message = document.querySelector('#message');
const body = document.querySelector('#runs');
const controls = Object.fromEntries(['repository', 'status', 'pageSize'].map(id => [id, document.getElementById(id)]));
let generation = 0;
let active;
let snapshot;
let state;
function reflectState() {
  for (const [key, element] of Object.entries(controls)) element.value = String(state[key]);
  const query = stateParams(state).toString();
  history.replaceState(null, '', `${location.pathname}${query ? `?${query}` : ''}`);
  document.querySelector('#filters').textContent = `Repository: ${state.repository || 'all authorized'} · Status: ${state.status} · ${state.pageSize} per page`;
}
function resetSnapshot() {
  snapshot = undefined;
  body.replaceChildren();
  for (const key of ['total', 'success', 'failure', 'active', 'other']) document.getElementById(key).textContent = '—';
  for (const id of ['previous', 'next']) document.getElementById(id).disabled = true;
  document.getElementById('pagination').textContent = '';
  document.getElementById('observed').textContent = '';
  document.getElementById('export').hidden = true;
}
function render() {
  const view = selectView(snapshot.runs, state);
  state = { ...state, page: view.page };
  reflectState();
  body.replaceChildren();
  for (const run of view.rows) {
    const tr = document.createElement('tr');
    for (const value of [run.repository, run.revision.slice(0, 12), run.workflowPath, run.status, run.createdAt, run.profile ?? 'Unreported']) {
      const td = document.createElement('td'); td.textContent = value; tr.append(td);
    }
    const td = document.createElement('td');
    const details = document.createElement('details');
    const summary = document.createElement('summary'); summary.textContent = 'Inspect';
    const description = document.createElement('p');
    description.textContent = `Run: ${run.id} · Exact commit: ${run.revision} · Duration: ${run.duration ?? 'Unreported'} · Release evidence: not collected. Promotion disabled.`;
    details.append(summary, description); td.append(details); tr.append(td); body.append(tr);
  }
  for (const [key, value] of Object.entries(view.counts)) document.getElementById(key).textContent = String(value);
  document.getElementById('pagination').textContent = `${view.from}–${view.to} of ${view.matched} matching observations · Page ${view.page} of ${view.pages}`;
  document.getElementById('previous').disabled = view.page <= 1;
  document.getElementById('next').disabled = view.page >= view.pages;
  document.getElementById('observed').textContent = `Snapshot fetched: ${snapshot.observedAt}. Counts cover the retrieved window, before status filtering.`;
  const download = document.getElementById('export');
  download.href = `/api/releases${state.repository ? `?${new URLSearchParams({ repository: state.repository })}` : ''}`;
  download.hidden = false;
  message.textContent = view.matched ? 'Authorized observations loaded. Release evidence remains unverified.' : 'No matching observations. This is not evidence of healthy CI.';
}
async function refresh(event, requestedPage = 1) {
  event?.preventDefault();
  active?.abort(); active = new AbortController();
  const current = ++generation;
  resetSnapshot();
  try {
    state = readState(new URLSearchParams({ repository: controls.repository.value.trim(), status: controls.status.value, pageSize: controls.pageSize.value, page: String(requestedPage) }));
    reflectState();
    message.textContent = 'Loading authorized observations…';
    const response = await fetch(`/api/releases${state.repository ? `?${new URLSearchParams({ repository: state.repository })}` : ''}`, { signal: active.signal, cache: 'no-store' });
    if (!response.ok) throw new Error('request_failed');
    const value = await response.json();
    if (value.schemaVersion !== 1 || value.readOnly !== true || value.promotionAllowed !== false || value.evidenceState !== 'not-collected'
        || !Array.isArray(value.runs) || value.runs.length > 200 || typeof value.observedAt !== 'string') throw new Error('invalid_response');
    if (current !== generation) return;
    snapshot = value; render();
  } catch (error) {
    if (error.name !== 'AbortError' && current === generation) {
      resetSnapshot();
      message.textContent = 'Observations unavailable. Check saved filters, app consent, warehouse access, and view permissions. No release readiness is inferred.';
    }
  }
}
form.addEventListener('submit', refresh);
for (const [id, delta] of [['previous', -1], ['next', 1]]) document.getElementById(id).addEventListener('click', () => {
  if (!snapshot) return;
  state = { ...state, page: state.page + delta }; render();
});
document.getElementById('reset').addEventListener('click', () => {
  state = readState(new URLSearchParams()); reflectState(); refresh();
});
// Back/forward and shared URLs refetch under the current caller, never cached credentials.
async function fromLocation() {
  active?.abort(); ++generation; resetSnapshot();
  try {
    state = readState(new URLSearchParams(location.search));
    reflectState(); await refresh(undefined, state.page);
  } catch { message.textContent = 'Invalid saved filters. Clear filters to begin a new authorized query.'; }
}
window.addEventListener('popstate', fromLocation);
fromLocation();
