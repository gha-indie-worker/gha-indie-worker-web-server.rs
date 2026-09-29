# Authenticated customer dashboard v2

This is the implementation contract for the signed-in IndieBuild web product. It is stacked on the runnable Axum/MASH web-server work and intentionally avoids introducing React.

## Host and auth boundary

- `https://indiebuild.dev/` remains the public marketing site.
- The marketing site's **Sign in** action should send the user to the dynamic product surface, preferably `https://app.indiebuild.dev/login`.
- `app.indiebuild.dev` owns the customer dashboard and session cookie.
- Session cookies must be `Secure`, `HttpOnly`, `SameSite=Lax` or stricter, and host-scoped unless a cross-host flow explicitly requires otherwise.
- Shared Auth / Supabase / Neon identity tokens are exchanged server-side. Do not expose database credentials or provider credentials to the browser.
- After sign-in, select an organization from claims/membership data and establish the tenant context before any canonical query.
- Browser writes for CI/CD state go through typed API/RPC calls. The only direct Supabase browser writes are user-owned `giw_ui` preference/inbox state protected by RLS.

## Navigation

Persistent left navigation:

1. Overview
2. Applications
3. Pipelines
4. Deployments
5. Environments
6. Infrastructure
7. Runners
8. Errors
9. Artifacts & caches
10. Ecosystem
11. Audit
12. Settings

Top bar:

- current organization switcher;
- global search / command palette;
- active incidents/degraded count;
- user menu;
- compact connection indicator for live updates.

## Overview

The overview is an operational home page, not a marketing dashboard. Prefer small status groups followed by dense lists.

Status groups:

- Applications: Healthy / Progressing / Degraded / Unknown
- Pipelines: Running / Queued / Failed recently / Successful recently
- Deployments: Waiting approval / Deploying / Failed
- Infrastructure: In sync / Drifted / Failed
- Errors: unresolved P1/P2 groups from `ores-err-trace`
- Runners: Idle / Busy / Draining / Offline
- ORES ecosystem: Missing / Outdated / Misconfigured dependencies

Each status count is a link that applies the corresponding filter on the owning page.

Below the summary:

- Recent pipeline runs
- Applications needing attention
- Pending deployment approvals
- Drifted stacks/resources
- Top unresolved error groups
- ORES adoption findings

## Applications

The list page supports URL-addressable filters, search, saved views, and table/tile modes.

Columns:

- application
- environment
- repository
- desired revision
- observed revision
- sync
- health
- age / last reconcile

The detail page centers on a resource tree. Selecting a node opens a right-side drawer with:

- Summary
- Desired / Observed
- Diff
- Events
- Errors
- External links

Health vocabulary should be: `healthy`, `progressing`, `degraded`, `suspended`, `missing`, `unknown`.
Sync vocabulary should be: `synced`, `out_of_sync`, `error`, `unknown`.

## Pipelines

The primary run visualization is a DAG/stage graph rather than a flat log list.

Header:

- repository / workflow
- ref + commit
- trigger
- state/conclusion
- duration
- attempt selector
- provider deep link

Graph:

- nodes are jobs;
- edges come from `needs`;
- job selection expands steps and duration/status;
- retries are represented as explicit run/job attempts;
- failure/error badges link to the associated `ores-err-trace` groups.

Detail lower panel:

- live logs
- artifacts
- caches
- deployment/environment
- errors/traces
- external provider references

Do not infer success from process exit alone when the provider reports a richer conclusion.

## Deployments and environments

Environment detail must clearly separate these states:

- queued for execution;
- waiting for protection/approval;
- approved;
- deploying;
- verifying;
- completed.

Environment page:

- protection rules
- allowed branch/ref policy
- current applications/revisions
- recent deployments
- pending approvals
- error/drift summary

Never render actual secret values. Render only the configured secret source/reference and access state.

## Infrastructure

Two complementary views:

### Stacks

- provider/account
- environment
- engine
- desired/observed digest
- drift state
- latest plan counts
- last plan/apply/observation

### Resource tree

Group by application/stack/provider. Resource drawer shows desired/observed metadata, diff, events, errors, and the pipeline/deployment that last changed it.

Supported engines should include Terraform, OpenTofu, Pulumi, CloudFormation, Kubernetes, Helm, ores-compose, and provider-native deployment models.

## Errors

Errors are grouped through `ORESoftware/ores-err-trace`; the UI must not create its own error-grouping semantics.

List defaults:

- unresolved first;
- priority descending;
- last-seen descending.

Columns:

- priority / severity
- normalized error
- service
- occurrence count
- first / last seen
- latest release
- repositories / environments
- status

Detail drawer:

- fingerprint policy + fingerprint
- recurrence and reopen history
- releases
- repositories/environments
- related trace IDs
- redacted sample/meta context
- resolution history
- linked run/job/step/deployment/application/resource

Structured context shown in the UI must come after the recursive redaction boundary in `ores-err-trace`.

## Ecosystem

This page makes ORES integration quality visible.

Rows are connected repositories. Columns are relevant shared components/capabilities. Cells show:

- current
- outdated
- missing
- misconfigured
- blocked
- unknown

The catalog should be discovered from:

- `ORESoftware/ores-*` repositories;
- repositories in `ores-*` GitHub organizations;
- critical non-prefix tooling such as `ORESoftware/api-docs` and `ORESoftware/typespec-json-schema-validator`.

Evidence drawer should show manifest/config path, requested ref, detected immutable ref/commit, last audit time, and suggested remediation. Never put access tokens or secret contents in evidence.

## UI rendering architecture

Keep the Rust HTML-first/MASH direction:

- Axum routes and typed handlers;
- server-rendered HTML for initial navigation;
- HTMX for partial refresh/mutations where appropriate;
- WebSocket/stateful TCP event stream for live status/log updates;
- small Rust/WASM islands only when graph/tree interactivity warrants them;
- no React requirement.

Use a consistent shell for all pages so live status, organization context, command palette, and navigation are not reimplemented per route.

## Typed API boundary

Use `ORESoftware/api-docs` for the customer dashboard's RPC/HTTP contract. The route string/method/body/query/header combination must not be duplicated as ad-hoc switch logic in browser code.

Recommended read models:

- `dashboard.overview.get`
- `applications.list`
- `applications.get`
- `applications.resourceTree`
- `pipelines.list`
- `pipelines.run.get`
- `pipelines.run.logs.stream`
- `deployments.list`
- `deployments.approve`
- `environments.list`
- `infrastructure.stacks.list`
- `infrastructure.resourceTree`
- `errors.list`
- `errors.get`
- `ecosystem.adoption.list`
- `audit.list`

Writes must go through the API server and emit audit entries. Read endpoints still enforce org membership and role.

## Route skeleton

Suggested product routes:

```text
/login
/logout
/app
/app/applications
/app/applications/:id
/app/pipelines
/app/pipelines/:run_id
/app/deployments
/app/environments
/app/environments/:id
/app/infrastructure
/app/runners
/app/errors
/app/errors/:id
/app/artifacts
/app/ecosystem
/app/audit
/app/settings
```

Filters use query parameters so a filtered view can be bookmarked/shared. Saved-view persistence belongs in Supabase `giw_ui.saved_views`.

## Security and failure behavior

- unauthenticated `/app/**` => redirect to `/login`, never render partial customer state;
- authenticated but no org => organization/onboarding chooser;
- authenticated but unauthorized org => 403 with no resource existence leak;
- canonical DB/API unavailable => explicit degraded shell with last-known UI metadata only; do not silently serve stale execution state as current;
- live stream disconnected => visible reconnecting indicator and polling/backoff fallback;
- unknown health/drift => render Unknown, never coerce to Healthy;
- provider credentials are server-side only;
- all tenant-scoped canonical reads run with the established `giw.org_id` context.

## Incremental implementation order

1. Merge/land the runnable Axum parent work.
2. Add product shell + `/login` + auth callback/session middleware.
3. Add Overview with typed read model.
4. Add Pipelines run DAG using existing runs/jobs/log chunks plus new attempts/steps.
5. Add Applications/resource tree and Infrastructure/drift.
6. Add Deployments/Environments/approvals.
7. Add Errors backed by `ores-err-trace`.
8. Add Ecosystem adoption matrix.
9. Add Supabase saved views/preferences/inbox.
10. Exercise the entire flow in `gha-indie-worker-test` before promotion.
