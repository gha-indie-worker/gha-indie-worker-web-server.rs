import { test } from 'node:test';
import assert from 'node:assert/strict';
import { request } from 'node:http';
import { once } from 'node:events';
import { createAppServer } from '../../marketplace/databricks/server.mjs';
import { configFrom, COLUMNS } from '../../marketplace/databricks/query.mjs';
const config = configFrom({ DATABRICKS_HOST: 'https://workspace.cloud.databricks.com', DATABRICKS_WAREHOUSE_ID:'warehouse_1', INDIEBUILD_RELEASE_VIEW:'catalog.schema.releases' });
const row = ['run_1','org/repo','a'.repeat(40),'ci.yml','success','2026-09-08T10:00:00Z',null,null];
function payload() { return { status:{state:'SUCCEEDED'}, manifest:{format:'JSON_ARRAY',total_row_count:1,total_chunk_count:1,schema:{column_count:8,columns:COLUMNS.map((name, position)=>({name,position}))}},result:{data_array:[row]}}; }
async function serving(t, upstream) {
  const server = createAppServer(config, upstream);
  server.listen(0,'127.0.0.1'); await once(server,'listening');
  t.after(() => new Promise(resolve => { server.closeAllConnections(); server.close(resolve); }));
  return `http://127.0.0.1:${server.address().port}`;
}
test('real HTTP listener serves modules and denies mutations and unauthenticated data', async t => {
  let calls = 0;
  const origin = await serving(t, async () => { calls++; return Response.json(payload()); });
  for (const path of ['/', '/app.js','/view.mjs','/style.css']) {
    const response = await fetch(origin + path); assert.equal(response.status,200);
    assert.equal(response.headers.get('cache-control'),'private, no-store');
    assert.match(response.headers.get('content-security-policy'), /default-src 'none'/);
  }
  assert.equal((await fetch(origin + '/api/releases')).status,401);
  assert.equal((await fetch(origin + '/api/releases',{method:'POST'})).status,405);
  assert.equal((await fetch(origin + '/missing')).status,404);
  assert.equal(calls,0);
});
test('real HTTP response includes honest counts and no forwarded credential', async t => {
  const tokens = [];
  const origin = await serving(t, async (_, options) => { tokens.push(options.headers.authorization); return Response.json(payload()); });
  for (const token of ['test-user-a','test-user-b']) {
    const response = await fetch(origin + '/api/releases?repository=org/repo',{headers:{'x-forwarded-access-token':token}});
    assert.equal(response.status,200);
    const text = await response.text(); assert.ok(!text.includes(token));
    const value = JSON.parse(text);
    assert.equal(value.counts.total,1); assert.equal(value.counts.success,1);
    assert.equal(value.promotionAllowed,false); assert.equal(value.evidenceState,'not-collected');
    assert.ok(Number.isFinite(Date.parse(value.observedAt)));
  }
  assert.deepEqual(tokens,['Bearer test-user-a','Bearer test-user-b']);
});
test('duplicate forwarded authorization headers fail before a warehouse call', async t => {
  let calls = 0;
  const origin = await serving(t, async () => { calls++; return Response.json(payload()); });
  const status = await new Promise((resolve,reject) => {
    const req = request(origin + '/api/releases',{ headers:['Host',new URL(origin).host,'X-Forwarded-Access-Token','test-a','X-Forwarded-Access-Token','test-b'] }, res => { res.resume(); res.on('end',() => resolve(res.statusCode)); });
    req.on('error',reject); req.end();
  });
  assert.equal(status,401); assert.equal(calls,0);
});
test('warehouse denial is not rewritten to an empty successful dashboard', async t => {
  const origin = await serving(t, async () => new Response('private upstream diagnostic',{status:403}));
  const response = await fetch(origin + '/api/releases',{headers:{'x-forwarded-access-token':'test-token'}});
  assert.equal(response.status,403); assert.equal(response.headers.get('cache-control'),'private, no-store');
  assert.deepEqual(await response.json(),{error:'data_access_denied'});
});
