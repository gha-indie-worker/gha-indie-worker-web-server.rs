# IndieBuild Databricks release observations

Development package, not a published Marketplace listing or verified installation.
This platform adapter does not replace the primary Rust/MASH web server.

## Implemented

Repository and reported-status filters; URL-persisted view state; bounded pagination;
window summary counts; full-commit inspection; fresh authenticated JSON export;
explicit loading, empty, denied and unavailable states; keyboard-accessible controls.
The browser and server share presentation-only status classification in `view.mjs`.
Unknown, generic completed, cancelled and skipped statuses never imply success.
No build status grants release or deployment authority.

The SQL query projects the existing run-summary view, not a new authoritative domain
contract. Canonical TypeSpec and independent JSON Schema stay in the interfaces repo.
Queries use only the current caller's forwarded token and a parameterized repository
filter. Results must fit one bounded inline JSON chunk with consistent schema, row
counts and offsets. Duplicates and out-of-filter responses are rejected. Ordering
uses the source timestamp column, not its display-string cast.

## Trust boundary

Run only behind the managed Databricks Apps ingress that supplies
`X-Forwarded-Access-Token`. Direct untrusted access to this backend must be blocked.
A caller must consent to SQL access and have warehouse and approved view permissions.
Do not substitute app/service-principal credentials. No tokens are returned to the
browser, persisted, or logged. No deployment, mutation or runner-control route exists.

Configuration is owned by this package's `.cli-flags.toml`; the native flags-2-env
binding must resolve at startup. No parser fallback is allowed. `app.yaml` references
the `sql-warehouse` resource. Configure the approved three-part view and user SQL scope
in the app resource. This repository does not provision or grant those permissions.

## Tests

From this directory, `npm test` runs dependency-free adapter, presentation and real
localhost HTTP tests. Warehouse responses are simulated; this is not a live warehouse
or browser installation test. The production native flags-2-env bootstrap is not
exercised by these tests.

## Required before installation or publication

Generate and review the dependency lockfile through approved read-only dependency
authentication; verify the pinned native flags-2-env package on the target platform;
run startup/health checks and real browser tests; install in a non-production account;
prove two-user/denied-user isolation with real view policies; verify resource grants,
empty results, truncation and error behavior; then complete provider/listing review.
Do not claim ready, installed, or published until those gates have actual evidence.

Removal is an operator action: revoke consent/view/warehouse grants and stop/remove the
app. Snowflake Native App packaging is a separate unfinished delivery track.
