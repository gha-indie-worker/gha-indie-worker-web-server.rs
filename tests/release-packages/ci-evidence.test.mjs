import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const ci = readFileSync(new URL('../../.github/workflows/ci.yml', import.meta.url), 'utf8');
test('missing dependency access fails rather than skips to green', () => {
  assert.match(ci, /if \[ -z "\$\{FLEET_READ_TOKEN:-\}" \]; then[\s\S]*?exit 1/);
  assert.doesNotMatch(ci, /if: env\.FLEET_READ_TOKEN/);
});
test('all compilation steps are frozen and unconditional', () => {
  for (const command of ['clippy', 'test', 'check', 'doc']) assert.match(ci, new RegExp(`cargo ${command} --frozen`));
  assert.doesNotMatch(ci, /continue-on-error/);
});
test('no persistent credential URL and no workflow-wide token', () => {
  assert.doesNotMatch(ci, /git config --global|x-access-token:|^env:/m);
  assert.equal((ci.match(/secrets\.FLEET_READ_TOKEN/g) ?? []).length, 1);
});
test('acceptance requires completed build evidence and package checks', () => {
  assert.match(ci, /if: always\(\)/);
  assert.match(ci, /test "\$RUST_EXECUTED" = true/);
  assert.match(ci, /needs: \[rust, assets, policy, release-packages\]/);
});
