import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { AppError, configFrom, queryReleases } from './query.mjs';
const assets = new Map([
  ['/', ['text/html; charset=utf-8', readFileSync(new URL('./index.html', import.meta.url))]],
  ['/view.mjs', ['text/javascript; charset=utf-8', readFileSync(new URL('./view.mjs', import.meta.url))]],
  ['/app.js', ['text/javascript; charset=utf-8', readFileSync(new URL('./app.js', import.meta.url))]],
  ['/style.css', ['text/css; charset=utf-8', readFileSync(new URL('./style.css', import.meta.url))]]
]);
const security = {
  'cache-control': 'private, no-store', 'x-content-type-options': 'nosniff',
  'referrer-policy': 'no-referrer', 'vary': 'X-Forwarded-Access-Token',
  'content-security-policy': "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'"
};
export async function handle(request, config, fetcher = fetch) {
  const url = new URL(request.url);
  if (request.method !== 'GET') return new Response('Method not allowed', { status: 405, headers: { ...security, allow: 'GET' } });
  if (assets.has(url.pathname)) {
    const [type, body] = assets.get(url.pathname);
    return new Response(body, { headers: { ...security, 'content-type': type } });
  }
  if (url.pathname !== '/api/releases') return new Response('Not found', { status: 404, headers: security });
  try {
    if ([...url.searchParams.keys()].some(k => k !== 'repository') || url.searchParams.getAll('repository').length > 1) throw new AppError(400, 'invalid_query');
    const value = await queryReleases(config, request.headers, url.searchParams.get('repository'), fetcher);
    return Response.json(value, { headers: security });
  } catch (error) {
    const known = error instanceof AppError;
    return Response.json({ error: known ? error.code : 'service_unavailable' }, { status: known ? error.status : 503, headers: security });
  }
}
export async function start() {
  // Native SDK resolution must succeed. Do not silently replace flags-2-env.
  const { parseOverridesFromArgs } = await import('@oresoftware/f2e');
  const overrides = parseOverridesFromArgs(['indiebuild-databricks', ...process.argv.slice(2)], {
    configPath: fileURLToPath(new URL('./.cli-flags.toml', import.meta.url))
  });
  if (overrides.isHelpMenu) { overrides.printTable(); return; }
  const values = { ...process.env, ...overrides };
  const config = configFrom(values);
  const port = Number(values.DATABRICKS_APP_PORT ?? '8000');
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new AppError(503, 'invalid_configuration');
  const server = createAppServer(config);
  server.listen(port, '0.0.0.0');
  for (const signal of ['SIGTERM', 'SIGINT']) process.once(signal, () => server.close());
}
export function createAppServer(config, fetcher = fetch) {
  const server = createServer({ maxHeaderSize: 16384 }, async (req, res) => {
    try {
      const headers = new Headers();
      for (const [name, value] of Object.entries(req.headers)) if (typeof value === 'string') headers.set(name, value);
      const response = await handle(new Request(new URL(req.url, 'http://app.internal'), { method: req.method, headers }), config, fetcher);
      res.writeHead(response.status, Object.fromEntries(response.headers));
      res.end(Buffer.from(await response.arrayBuffer()));
    } catch { res.writeHead(503, security); res.end('Service unavailable'); }
  });
  server.headersTimeout = 10000; server.requestTimeout = 20000; server.keepAliveTimeout = 5000;
  return server;
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  start().catch(() => { console.error('Databricks app startup failed; check approved configuration and packaged dependencies.'); process.exitCode = 1; });
}
