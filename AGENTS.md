# Project analysis

## Project overview

Rust workspace (edition 2024, toolchain `1.95`) at `backend/`, the server side of
Teknologappen (a student-association app). Three deployable HTTP services plus
shared libraries:

| Crate | Role | Dev port |
| --- | --- | --- |
| `minilith` | Main application: tickets, groups, activities, admin, users, push. Biggest crate. | 8000 |
| `fed-auth` | OIDC + SAML2 identity provider. | 8001 |
| `transactions` | Payment provider integration (Stripe, Swish, free) + receipts. | 8002 |
| `bin-common` | Shared infra: DB pool, OTEL/tracing setup, listeners, shutdown. | — |
| `minilith-errors` | Shared error type, extension traits, alert/email system. | — |
| `fed-auth-verifier` | JWT/JWKS verification, `User` extractor, shared transaction callbacks. | — |
| `loadtesting` | Standalone load-test binary. | — |

Everything is HTTP over `poem` + `poem-openapi` (OpenAPI docs + Swagger UI at
`/v0/docs`, spec at `/v0/spec.json`). Postgres via `sqlx`. Observability via
OpenTelemetry into the collector/Prometheus/Loki/Tempo/Grafana stack.

Non-obvious: `bin_common::listeners(port)` binds **two** listeners — plain TCP on
`port` and TLS (self-signed cert) on `port + 50`. So minilith is 8000/8050,
fed-auth 8001/8051, transactions 8002/8052. Production Traefik talks to the TLS
ports. This is to use HTTP/2 connection pooling for high-load situations.

## Architecture and directory responsibilities

### Service skeleton (identical in all three services)

1. `src/main.rs`: `Server::new(listeners(PORT)).run_with_graceful_shutdown(get_endpoint(None, true).await?, shutdown_signal(), ...)`. Startup errors alert L2.
2. `src/lib.rs::get_endpoint(test_db: Option<PgPool>, migrate: bool)`:
   - `get_otel(pkg_name, is_test)` (`bin-common`).
   - Build `Context::new(test_db, migrate)` wrapped in `Arc` (`ContextWrapper`).
   - When not testing: `runtime::initial_checks(&context)` then `runtime::spawn(&context)`.
   - Build one `OpenApiService` per module router, nest under a prefix, add swagger/spec, then `.with(OpenTelemetryMetrics::new())`, `.with(OpenTelemetryTracing::new(...))`, `.with(cors)`.
   - Passing `Some(db)` marks the run as a test (skips runtime tasks + external service setup) and is how integration tests build the app.
3. `src/context.rs`: the DI container. Holds `PgPool`, `DebugConfig`, encryption key, external HTTP clients, S3 bucket, email/push clients, typst report base. `Context::new` and the functions it calls use `color_eyre`; everything request-path uses `minilith-errors`.
4. `src/runtime.rs`: `initial_checks` (one-shot on boot) and `spawn` (long-lived polling loops).

### Module pattern

Each feature area is a module exposing a `Router` struct that holds
`context: ContextWrapper` and `impl Deref<Target = ContextWrapper>`. Endpoints
are declared as `#[OpenApi(prefix_path = "...")] impl Router` with `#[oai(path,
method)]` methods in an `api.rs`; business logic lives in sibling files
(`purchase.rs`, `queue.rs`, `catalog.rs`, ...). Register new routers in the
service's `get_endpoint`.

`minilith/src/ticket/` is the reference implementation: `api.rs` (transport),
`mod.rs` (invariants), `access.rs` (authz queries), `flow.rs` (purchase-flow
state machine + row locks), `allocation.rs`, `catalog.rs`, `purchase.rs`,
`queue.rs`, `release.rs`, `transfer.rs`, `validation.rs`, `models.rs`,
`notifications.rs`, `tests.rs`.

Other `minilith` modules: `group/` (with `admin.rs`, `member.rs`, `path.rs`),
`admin.rs` (large monolith, ~3.2k lines — endpoint + types colocated),
`activities.rs`, `user.rs`, `push_notifications.rs`, `accounting.rs`,
`report.rs` (+ `.typ` template), `transactions.rs` (client to the transactions
service).

### Authorization

Auth is a custom `poem-openapi` `SecurityScheme` extractor. `fed-auth-verifier`
provides `User` (JWT via `JwkContext<AuthUrl>` from `req.data()`); transactions
provides `ApiAuth` (DB token lookup). Handlers receive the extracted identity as
an argument. Module docs state the rule: the API layer is only transport; each
operation enforces its own authorization, `*_unchecked` functions may only run
after the caller verified authz, and reusable authz queries live in `access.rs`.

### Databases

One Postgres instance, three separate databases: `fed` (minilith), `auth`
(fed-auth), `transactions`. In production compose injects each service's
`DATABASE_URL`; locally each crate's `.env` points at its own DB. Schema sources
live in `<crate>/schema/*.sql`, compiled to `schema.sql`, and migrations in
`<crate>/migrations/`. See "Schema workflow" below.

## Coding conventions

- CI (`.github/workflows/rust.yml`) runs `cargo fmt --check`,
  `cargo build --workspace`, `cargo clippy --workspace -- -D warnings`,
  `cargo test --workspace`.
- Lints are strict (`Cargo.toml` `[workspace.lints]`): `pedantic` is deny, as are
  `unwrap_used`, `expect_used`, `panic`, `indexing_slicing`, `str_to_string`,
  `todo`, `unimplemented`, and more. `clippy.toml` re-allows `unwrap`/`expect`
  in tests. Suppressions must carry a `reason` (`allow_attributes_without_reason`
  is deny), e.g. `#[allow(clippy::too_many_lines, reason = "...")]`.
- Never use `chrono`; use the `time` crate (`OffsetDateTime`, `Duration`).
- DB access only through `sqlx` macros: `sqlx::query!`, `query_as!`,
  `query_scalar!` (compile-time checked, offline metadata in `.sqlx/`). Prefer
  `query_scalar!` for single values and `query_as!` for row structs. `query!`
  dominates in `minilith`.
- Outer joins: alias nullable columns as `"col?"` or sqlx assumes non-null
  (see `README.md`).
- Error handling in handlers uses `minilith_errors` (`MinilithResult`,
  `MinilithEndpointError` + `wrap_err_*` extension traits). `color_eyre` is only
  for `Context::new` and startup.
- Public visibility is fine — this is internal code; widen it to reuse code
  instead of duplicating.
- Keep doc comments on endpoints: they document the contract and list the error
  cases (e.g. `/// # Errors` / `/// AUTH, DB`). Document only non-obvious things.
- `#[cfg(test)] mod tests` for in-file unit tests; integration tests in
  `<crate>/tests/`.

## Common implementation patterns

### Error handling and alerts

- `MinilithEndpointError` variants map to 400/401/403/404/500 and carry a
  `MinilithError { message, field }`.
- Constructor helpers: `bad_frontend_code`, `bad_user_input`, `unauthorized`,
  `not_found`, `internal_error`. Extension traits: `wrap_err_bad_frontend`,
  `wrap_err_bad_user`, `wrap_err_unauthorized`, `wrap_err_internal`,
  `wrap_err_not_found`, `wrap_err_encryption`.
- `From<sqlx::Error>` (feature `sqlx`) logs and alerts: DB errors L3, connection
  errors L2.
- `internal_error` automatically emails an alert whose level is parsed from the
  message prefix: `l1:` → L1, `l2:` → L2, otherwise L3. Prefixes `noalert:` and
  `email:` suppress the alert. Explicit `alert(AlertLevel::Lx, msg)` /
  `alert_html(...)` exist for non-error paths. L1 = loss of funds, L2 = service
  breaking, L3 = potential behaviour breakage. `configure_alert_email` validates
  all recipient env vars at startup.

### Database transactions

```rust
let mut txn = ctx.db.begin().await?;
// ... queries via .fetch_*(&mut txn.executor()) ...
txn.commit().await?;
```

`sqlx_tracing` transactions need `.executor()`. Use `ensure_affected_rows`
(`minilith/src/ticket/mod.rs`) after bulk writes to assert the expected row
count.

### External service calls

`Context` exposes `transactions_get` / `transactions_post` returning a configured
`reqwest::RequestBuilder` with the bearer token already set. Check
`.error_for_status()` and map failures with the right alert level.

### Background work (`runtime.rs`)

`initial_checks` runs once at boot; `spawn` starts `tokio::spawn` polling loops.
Minilith scales horizontally, so every loop must be safe to run concurrently
across replicas — rely on SQL row locks / `skip locked` rather than in-process
state. Prefer the DB as the coordination point.

### Auth extractor sketch

```rust
#[derive(SecurityScheme)]
#[oai(ty = "bearer", checker = "User::from_token")]
pub struct User(String);
```

The extractor pulls its verification context from `req.data()` (registered in
`get_endpoint` via `.data(...)`).

### Fixtures and tests

Fixtures are `.sql` files in `src/fixtures/`, `<module>/fixtures/`, or
`tests/fixtures/`, applied with `#[sqlx::test(fixtures("name"))]`. Integration
tests build the real endpoint with `get_endpoint(Some(db), false)` and drive it
with `poem::test::TestClient` (see `minilith/tests/healthcheck.rs` and
`minilith/tests/lib.rs`).

## How to add or modify features

- **New endpoint**: add the `#[oai]` method to the owning module's `api.rs`, keep
  it a thin delegate to a domain function (ONLY if it's a big module (for
  tickets, groups, but NOT for user)); document the contract and error cases.
- **New module**: create `<module>/mod.rs` with a `Router` (Deref to
  `ContextWrapper`), split transport/logic into `api.rs` + domain files, and
  register the router in the service's `get_endpoint` `OpenApiService::new((...))`.
  (only applicable to large new areas)
- **New table/schema change**: see "Schema workflow". Do not hand-edit files in
  `migrations/` except when doing data-carry-over for the migration. Don't
  manually create files there.
- **New shared type/callback**: put it in `fed-auth-verifier/src/callbacks.rs` so
  minilith and transactions stay in sync (they share the transaction callback
  types). ONLY FOR TRANSACTION CALLBACK TYPES.
- **Alert-worthy failure**: choose the level via the message prefix or an explicit
  `alert` call; don't alert on expected user input errors.
- **New service-wide dependency/config**: add it to the crate's `Context` and set
  it up in `Context::new` (using `color_eyre` there).

## Schema workflow (per crate)

1. Add/change `.sql` files in `<crate>/schema/` (numbered like `12-tickets.sql`).
2. `cd <crate>/schema && ./compile.sh` → regenerates `schema.sql`.
3. `./create-migration.sh <name>` (needs `uv` and podman): spins up temp DBs,
   diffs old vs new schema with `results dbdiff`, writes
   `migrations/<timestamp>_<name>.sql`, and removes empty migrations.
4. In the crate dir run `sqlx database reset -fy`.
5. Restart the affected service (ask the human; per the rules below, do not run
   `cargo sqlx prepare` yourself).

## Build, test, and validation commands

```sh
cargo fmt --check
cargo build --workspace
cargo clippy --workspace -- -D warnings
cargo test --workspace                 # needs a live DATABASE_URL
podman compose up -d                   # local Postgres + observability stack (human ONLY)
./sqlx-prepare.sh                      # or `cargo sqlx prepare -- --all-targets` — human runs before commit
./build-images.sh                      # build prod images (human ONLY)
./build-push.sh <tag>                  # build + push prod images (human ONLY)
```

## Important pitfalls and non-obvious constraints

- **Do not run `cargo sqlx prepare`** — the human runs it before committing. CI
  builds with `SQLX_OFFLINE=true` against the checked-in `.sqlx/` metadata, so
  queries and metadata must stay in sync.
- `bin_common::listeners(port)` opens `port` (HTTP) and `port + 50` (HTTPS); when
  adding a service, account for both. Prefer connecting to `port + 50`.
- Three separate databases on one Postgres. A migration in one crate does not
  affect the others.
- Auth verifier: when the local auth service is unreachable, `User` falls back to
  the dev identity `lund-university:aa0000bb-s` (or the raw bearer token). The
  token `test:external-validation` (`EXTERNAL_VALIDATION_USER_ID`) bypasses the
  signature check in all builds; `test:` subjects are rejected in production.
- `Context::new` and startup code use `color_eyre`; everything request-path uses
  `minilith-errors`.
- Alert prefixes matter: an `l1:`/`l2:` prefix on an internal error escalates the
  alert and pages more people. Use deliberately.
- Horizontal scaling: background loops run on every minilith replica; make them
  concurrency-safe.
- Rustls uses a self-signed cert in dev (`bin_common::debug_rustls_config`); the
  external HTTP client disables TLS verification when `DebugConfig.enabled`.
- `DebugConfig` comes from env (`bin_common`); `debug.enabled` vs
  `debug.service_urls` gate CORS origins and internal vs public URLs.

## Representative files

- Service wiring: `minilith/src/lib.rs`, `minilith/src/main.rs`,
  `transactions/src/lib.rs`, `fed-auth/src/lib.rs`.
- DI container: `minilith/src/context.rs`, `transactions/src/context.rs`.
- Background jobs: `minilith/src/runtime.rs`.
- Errors/alerts/email: `minilith-errors/src/lib.rs`.
- Shared infra: `bin-common/src/lib.rs`.
- Auth extraction: `fed-auth-verifier/src/lib.rs`,
  `fed-auth-verifier/src/callbacks.rs`, `transactions/src/lib.rs` (`ApiAuth`).
- Domain example: `minilith/src/ticket/` (`mod.rs`, `api.rs`, `purchase.rs`,
  `flow.rs`, `access.rs`).
- Tests/fixtures: `minilith/tests/healthcheck.rs`, `minilith/tests/lib.rs`,
  `minilith/src/ticket/tests.rs`, `fed-auth/tests/tokens.rs`.
- Schema tooling: `minilith/schema/{compile.sh,create-migration.sh,README.md}`.
- Deployment: `compose.base.yaml`, `compose.yaml`, `compose.prod.yaml`,
  `Dockerfile`, `build-images.sh`, `build-push.sh`.

# Working rules

(written by hand by Erik Davidsson)

- Don't run `cargo sqlx prepare`, the human will do that before committing.
- use sqlx macros for database queries
- ask the human to restart minilith or other service when regenerating schema
- To update the schema: write to files under `schema/xx-<name>.sql` in the
  crate. Run `create-migration.sh initial` after removing the
  `migrations/xxx_initial.sql`, then `sqlx database reset -fy`.
- Use the error handling in `minilith-errors` (NOT color_eyre), except for
  functions which are called in `Context::new` in the respective folders
- We have an alert system set up. If something warrants sending an email use it.
  For internal errors, prefixing the message with `l1` or `l2` makes the error
  handling send an alert of level 1 respectively level 2 instead of the default
  level 3. In general the higher the level the more recipients and the higher
  criticality. Level 1 is generally for loss of funds for us or customer, level
  2 for service breaking signs, and level 3 for potential behaviour breakage.
- run `cargo clippy` to fix all lints
- keep the documentation amount low, just document the non-obvious necessary
  things
- alert the human if anything is ambiguous
- keep down code duplication
- the goal of the backend is to be maintainable easily by future developers. Do
  extra work to make a plan of the best way to approach the problem before
  beginning to code.
- follow the vision and goals outlined in `../docs/krav.typ`,
  `../docs/vision-och-värdegrund.typ`, `../docs/beslut/beslutsmetod.typ`
- always give feedback on the prompt and ideas the human presents in relation to
  the vision and goals outlined above
- a list of our technologies can be found in `../docs/beslut/teknologier/`,
  including example motivations of how to make decisions
- change visibility for existing structs & similar to reuse code. It's OK to
  make everything public since this is internal code
- write any contracts in the endpoint documentation
- if anything is against good practice or could be made better, please notify
  the human
- if anything can be refactored to clean up the code and understanding when
  reading the code, notify the human
- don't EVER use chrono
- only make targeted unit tests if it's very hard to reason about code

You are not writing code to make something work, you are writing the code to
make the project more maintainable.

After compacting the context, re-read this document and the documents mentioned
above so they stay in their full in your context.
