"""Preserved source-policy checks from ci.yml; no credentials are needed."""
import pathlib
import re
import subprocess
import sys
import tomllib

flags = tomllib.loads(pathlib.Path('.cli-flags.toml').read_text())['flags']
declared = {flag['env'] for flag in flags.values() if 'env' in flag}
external = {'PORT', 'RUST_LOG', 'APP_ENV', 'ORES_MIDDLEWARE_ENV',
            'ORES_MIDDLEWARE_RATE_LIMIT_HMAC_SECRET',
            'GHA_INDIE_WORKER_DATABASE_URL_CANONICAL'}
used, errors = set(), []
origins = {'https://github.com/gha-indie-worker', 'https://unpkg.com'}
url_pattern = re.compile(r'(?:src|href)\s*=\s*\(?"?(https?://[^\")\s]+)')
write_pattern = re.compile(r'\b(INSERT INTO|UPDATE |DELETE FROM|CREATE TABLE|ALTER TABLE|DROP TABLE)\b', re.I)
for path in pathlib.Path('src').rglob('*.rs'):
    text = path.read_text()
    used.update(re.findall(r'var\("([A-Z0-9_]+)"\)', text))
    for match in url_pattern.finditer(text):
        if not any(match.group(1).startswith(prefix) for prefix in origins):
            errors.append(f'{path}: third-party page origin')
    for match in write_pattern.finditer(text):
        line = text[:match.start()].count('\n')
        if path.name == 'db.rs' and 'is_select' in text.splitlines()[line]:
            continue
        errors.append(f'{path}:{line + 1}: web-server write statement')
missing = sorted(used - declared - external)
if missing:
    errors.append('undeclared configuration keys: ' + ', '.join(missing))
tracked = subprocess.check_output(['git', 'ls-files', '.env', '.env.*', 'env/dec/*.env'], text=True)
if any(path != '.env.example' for path in tracked.splitlines()):
    errors.append('plaintext environment is tracked')
if errors:
    sys.exit('\n'.join(errors))
