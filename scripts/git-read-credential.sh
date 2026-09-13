#!/bin/sh
# Process-local Git credential helper. Never writes a credential to disk.
set -eu
[ "${1:-}" = get ] || exit 0
protocol=
host=
while IFS= read -r line && [ -n "$line" ]; do
  case "$line" in
    protocol=*) protocol=${line#protocol=} ;;
    host=*) host=${line#host=} ;;
  esac
done
[ "$protocol" = https ] && [ "$host" = github.com ] || exit 0
[ -n "${FLEET_READ_TOKEN:-}" ] || exit 0
printf 'username=x-access-token\npassword=%s\n\n' "$FLEET_READ_TOKEN"
