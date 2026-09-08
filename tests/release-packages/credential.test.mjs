import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
const helper = fileURLToPath(new URL('../../scripts/git-read-credential.sh', import.meta.url));
function run(input, token = 'example-read-token', operation = 'get') {
  return execFileSync('sh', [helper, operation], { input, encoding: 'utf8', env: { PATH: process.env.PATH, FLEET_READ_TOKEN: token } });
}
test('supplies credentials only to exact HTTPS GitHub origin', () => {
  assert.equal(run('protocol=https\nhost=github.com\n\n'), 'username=x-access-token\npassword=example-read-token\n\n');
});
for (const host of ['evil.example', 'github.com.evil.example', 'user@github.com', 'github.com:443']) {
  test(`refuses host ${host}`, () => assert.equal(run(`protocol=https\nhost=${host}\n\n`), ''));
}
test('refuses cleartext and missing tokens', () => {
  assert.equal(run('protocol=http\nhost=github.com\n\n'), '');
  assert.equal(run('protocol=https\nhost=github.com\n\n', ''), '');
});
test('does not implement persistent store or erase operations', () => {
  for (const op of ['store', 'erase']) assert.equal(run('protocol=https\nhost=github.com\n\n', 'example-read-token', op), '');
});
