# Backend implementation check

Checked 2026-08-05 against `../docs/krav.typ`. Scope is limited to
`../backend` and `../docs`.

## Result

The backend is **not complete and is not safe to deploy as-is**. The release
targets compile and the current tests pass, but there are production-blocking
authentication, authorization, payment, and deployment findings below.

Checkboxes in the requirements section mean:

- `[x]`: the backend portion was found and its important contract was checked.
- `[ ] Partial`: an implementation exists, but a required contract is missing
  or incorrect.
- `[ ] Missing`: no sufficient backend implementation was found.

Do not treat an unchecked item as accepted until it has been fixed and
revalidated.

## Verification performed

- [x] `cargo fmt --all -- --check`
- [x] `cargo check --workspace --release`
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
- [x] `cargo test -p minilith`: 12 library tests, 1 seeder test, 1 integration
  test, and 3 doc tests passed.
- [x] `cargo test -p fed-auth`: 5 tests passed.
- [x] `cargo test -p transactions`: 1 test passed.
- [x] `cargo test -p minilith-errors`: 1 test passed.
- [x] `cargo test -p fed-auth-verifier` and `cargo test -p bin-common`: build
  and doc-test targets passed; neither crate has behavior tests.
- [x] Rendered `compose.prod.yaml` with Docker Compose and inspected the merged
  port bindings.
- [x] Scanned tracked files for common private-key and production-token
  patterns. No tracked private key, Stripe live key, or webhook secret was
  found. Deliberate development credentials remain in `compose.yaml` and test
  fixtures.
- [ ] Dependency vulnerability scan was not run: neither `cargo-audit` nor
  `cargo-deny` is installed.

## Threat model

### Assets

- LU and email identities, refresh tokens, API keys, and signing keys.
- Group memberships/adminships and the organization tree.
- Ticket capacity, reservations, ownership, validation status, and addons.
- Money, provider payment references, receipts, and bookkeeping reports.
- User names/languages, email-address IDs, push tokens, and notification data.
- Uploaded public images and the Postgres/S3/observability services.

### Trust boundaries and attackers

- Unauthenticated Internet clients reach Traefik and, with the current Compose
  merge, several containers directly on host ports.
- Authenticated normal users may send arbitrary UUIDs, timestamps, addon
  choices, recipient IDs, and redirect URLs; frontend restrictions are not a
  security boundary.
- A compromised or malicious group admin must remain confined to groups they
  administer and their documented descendant scope.
- Fed-auth trusts LU SAML and email links; Minilith trusts fed-auth JWTs;
  Minilith and transactions trust signed callbacks; transactions trusts Swish
  and Stripe webhooks.
- An external-validation service is untrusted and must be confined to disposable
  test data.
- A stolen database dump should not trivially disclose protected user data or
  reusable credentials.

### Main abuse cases tested by inspection

- Forge or bypass a login; reuse an authorization code or long-lived token.
- Escalate from one administered group to an unrelated organizer.
- Read or mutate activities/tickets outside membership/admin scope.
- exceed capacity, buy/own duplicates, transfer outside the configured window,
  or forge/replay a validation QR.
- Forge provider callbacks, obtain another client's receipt, or lose the link
  between external money and a local transaction.
- Escape the testing namespace through memberships, join rules, or transfers.
- Reach databases, object storage, or observability without Traefik/TLS.

## Production blockers

### SEC-01 — fixed unauthenticated external-validation bearer token

`fed-auth-verifier/src/lib.rs:17-21,258-270` accepts the literal bearer token
`test:external-validation` as a user in every build. It has no signature,
expiry, deployment flag, per-review secret, or rate limit. Anyone who knows the
repository can impersonate the shared account in production.

Make this capability explicitly opt-in, use a rotatable high-entropy credential
or a normally signed short-lived token, and reject it in ordinary production.

### SEC-02 — public production test login provider

`fed-auth/src/oidc.rs:773-776` registers provider `test` unconditionally and
`fed-auth/src/api.rs:156-183` lets its caller choose any `test:<stil_id>`.
The Origin check only rejects a wrong Origin when the header is present, so a
non-browser client omits it. An attacker can complete a normal OIDC/PKCE flow
and receive a valid signed token for any test identity. If `seed-dev` has run,
the attacker can impersonate the seeded `test:*` users that are members of the
main `tlth` tree, not just the external-validation user.

Compile or configure this provider out of production and enforce the decision
server-side. Origin is not authentication.

### SEC-03 — LU login is still a mock implementation

- `fed-auth/src/oidc.rs:747-765` sends LU login to `mocksaml.com`.
- `fed-auth/src/saml2.rs:22-35` downloads MockSAML metadata.
- `fed-auth/src/saml2.rs:301-311` prints the assertion and returns a hard-coded
  name, empty email, and no guild.
- The request ID is read but not invalidated after ACS, leaving replay/race
  risk during its 30-minute cache lifetime.
- There is no Medcheck/personal-number verification, later manual verification,
  or automatic section/TLTH membership. `minilith/src/user.rs:208-227` ignores
  `lth_guild` from the signed callback.

Do not expose the LU provider as production-ready until the real LU IdP,
attribute mapping, request consumption, error handling, and membership flow are
implemented and tested. Remove assertion logging because it can contain PII.

### SEC-04 — production Compose publishes internal services directly

`compose.prod.yaml:3-4` includes `compose.yaml`; adding `expose` does not remove
the included `ports`. The rendered production model publishes on all host
interfaces:

- Postgres `5432`
- OTEL collector `4317`, `55679`
- Grafana `3000`
- Loki `3100`
- Prometheus `9090`
- RustFS API/console `9000`, `9001`
- Tempo `3200`

The source bindings are at `compose.yaml:9-23,34-39,48-67,73-90`. This bypasses
Traefik and TLS and exposes high-value internal services. Remove/reset inherited
port mappings in the production model; use only internal networks and the
intended Traefik routes. Host firewalling is defense in depth, not a fix for the
Compose contract.

### SEC-05 — Stripe webhook verification uses the API key as signing secret

`transactions/src/context.rs:108-160` uses `client_ids.stripe_secret` to build
the Stripe API client and creates a webhook but discards the new endpoint's
signing secret. `transactions/src/api.rs:613-628` then passes that same Stripe
API key to `Webhook::construct_event`, which requires the webhook signing
secret. Legitimate webhooks will not verify, and there is no stored `whsec_*`
value that could make this work.

Store a distinct webhook signing secret per client (including the returned
secret when creating an endpoint), use it for verification, and test signed
success/expiry events end to end.

## High-severity findings

### AUTHZ-01 — resolved 2026-08-05

Activity creation now requires and locks the caller's direct adminship for the
creator group, and only that creator is installed as the initial host. On edit,
`creator_id` is immutable, adding hosts through the activity update is rejected,
and a caller may remove only a non-creator host group they directly administer.
The activity and relevant adminship rows are locked in the same transaction.

New hosts use `activity_host_invites`: an existing host admin sends an invite,
and a direct admin of the invited group accepts or declines it. Acceptance
atomically consumes the invite and adds the host. Focused tests cover unrelated
host additions, creator removal, self-removal, invite authorization and
acceptance/decline (`minilith/src/admin.rs`). This closes the organizer-claiming
path originally recorded by AUTHZ-01.

### TICKET-01 — transfer and one-ticket-per-activity contracts are not enforced

- `allow_transfer_ticket_start/stop` are stored but never checked by
  `minilith/src/ticket.rs:1128-1157`.
- There is no activity-level transfer permission even though the requirement
  says both activity and ticket kind must allow transfer.
- Bypass mode skips all recipient access checks; it does not require the
  recipient to have access to the activity.
- The schema explicitly says the backend must prevent one owner from owning
  multiple tickets for an activity (`minilith/schema/12-tickets.sql:133-143`).
  `ensure_user_may_purchase_ticket` checks only membership
  (`minilith/src/ticket.rs:1926-1975`), despite its error text claiming a
  duplicate check. Purchase and transfer can therefore create duplicates.

Enforce all transfer gates and the ownership invariant transactionally, with
row/advisory locking or a database-enforced denormalized invariant. Add tests
for purchase races and transfer-to-existing-owner.

### TICKET-02 — ticket-validation QR remains replayable

Validation now returns `purchased_tickets.owner_id`, rejects timestamps outside
a five-minute clock-skew window in either direction, and has activity-authorized
list/grant/revoke endpoints for `activity_verifiers`. However, the QR payload
still has no signature, one-time nonce, or binding to a short-lived server
challenge. A captured ticket UUID and timestamp can therefore be replayed during
that window; recording prior validations detects but does not prevent the replay.

Authenticate the QR payload and consume or expire a server-issued one-time
challenge atomically.

### TICKET-03 — the user receipt endpoint is broken

`minilith/src/ticket.rs:505-520` decodes encrypted `users.name bytea` as a JSON
internationalized string. It then calls `/v0/receipt` without the transaction
ID (`minilith/src/ticket.rs:522-533`), while transactions exposes only
`POST /v0/:id/receipt` (`transactions/src/api.rs:719-795`). The ticket's
`transaction_id` is never loaded. Purchaser authorization itself is present,
but the successful path cannot work.

Load and pass the purchased ticket's transaction ID, decrypt the user's name as
a string, and add an integration test that downloads a paid receipt.

### GROUP-01 — adminship still creates and deletes normal membership

The deployed migration and combined schema allow adminship without membership,
but:

- `minilith/src/group/admin.rs:299-347` inserts a membership before an
  adminship.
- `minilith/src/admin.rs:1634-1672` deletes both, including a legitimate
  pre-existing membership.
- subgroup creation calls that helper (`minilith/src/admin.rs:1435-1437`).
- `seed-dev` inserts membership for every admin (`minilith/src/bin/seed-dev.rs:250-270`).
- the fragment `minilith/schema/2-groups.sql:53-61` still contains the old
  membership foreign key, while `minilith/schema/schema.sql:79-84` and the
  migration do not. Regenerating migrations from the stale fragment can
  reintroduce it.

Remove membership side effects, preserve independent memberships on admin
removal, align every schema source, and restore a removal test for both
"admin-only" and "admin plus member" cases.

### DATA-01 — `seed-dev` can corrupt a live production database

The binary is copied into the production Minilith image (`Dockerfile:44-50`)
and its comments say it can be used in production. A run seeds both the main
and testing namespaces (`minilith/src/bin/seed-dev.rs:594-600`), overwrites main
group names/descriptions (`:174-193`) and activity fixture data, and most
critically resets existing fixture ticket counters and purchase flags
(`:542-567`). If those deterministic IDs have live reservations/purchases,
capacity/accounting is corrupted.

Do not run this seeder against production. Split an idempotent bootstrap of
required production groups/admins from disposable demo data, and make the demo
binary refuse non-test databases.

### DATA-02 — user-field encryption is not authenticated

`minilith/src/context.rs` now generates a fresh random nonce for each encrypted
value and embeds it in the ciphertext envelope, so the database no longer has a
separate nonce column and nonce reuse between name and language is fixed. The
construction still uses raw ChaCha20 without an integrity tag, so ciphertext is
malleable and corruption cannot be authenticated.

Use an AEAD such as ChaCha20-Poly1305 in the versioned value envelope and migrate
existing ciphertexts.

## Other correctness and security findings

### Activity, group, and authorization

- **Group-setting writes always return an error after mutating data:**
  `PUT /user/group-settings` executes an upsert without `RETURNING`, but calls
  `fetch_one` as though the statement returned a row
  (`minilith/src/user.rs:180-203`). Every successful upsert therefore becomes
  `RowNotFound` and an HTTP 500 after the database change has already committed.
  Both activity-filter and notification-setting clients can report failure and
  retry a write that actually succeeded. Use `execute`, or add `RETURNING` and
  return the row, and cover insert and conflict-update paths with endpoint tests.
- **Hierarchy contract mismatch:** `krav.typ` says a group admin administers all
  descendants. Most mutation endpoints use direct adminship
  (`minilith/src/admin.rs:1313-1358,1443-1552`), and activity/ticket editing
  requires a direct activity host (`minilith/src/group/admin.rs:154-223`). The
  ancestor-capable `check_adminship` helper is unused. Define one hierarchy
  policy and apply it consistently.
- **List/detail mismatch:** delegated admins can see one hidden case in
  `minilith/src/activity-list.sql:41-57`, but `Context::test_activity_access`
  rejects every hidden delegated activity (`minilith/src/context.rs:375-388`).
  A listed activity can therefore fail when opened.
- **Soft deletion is not an access rule:** group deletion only sets
  `deleted=true` (`minilith/src/admin.rs:1443-1450`). Activity visibility,
  purchasing, adminship, descendants, and allowed-group checks generally ignore
  `deleted`, so a removed group can continue affecting the app.
- **Join-request mail is missing:** `minilith/src/group/mod.rs:125-161` inserts
  the request but never emails direct admins, as required. Existing mail code
  covers adminship changes, not membership requests.
- **Subgroup setting inheritance is missing:** group creation does not inherit
  notification/filter defaults from parent or sibling consensus.
- **Bulk membership is missing:** only single-member PUT/DELETE endpoints exist.
- **No optimistic conflict contract:** activity and ticket-kind PUTs have no
  revision, `ETag`/`If-Match`, or equivalent stale-edit detection. Row locks
  serialize writes but silently allow last-writer-wins.

### Tickets, queues, and addons

- **Sales start can be bypassed after release changes:** queueing checks
  `has_been_released` and only the stop time (`minilith/src/ticket.rs:587-601`).
  Editing a released kind's start into the future does not reset/enforce the
  start. A newly created kind whose start is more than five minutes in the past
  is never released by the worker (`:1455-1488`).
- **Too-late transaction start is allowed:** `begin_purchase` passes the
  reservation timeout to transactions but does not reject an expired
  reservation or the required final one-minute window
  (`minilith/src/ticket.rs:829-886,992-1022`).
- **Immutability begins too late and permits too much:** `has_been_purchased` is
  set only after payment (`minilith/src/ticket.rs:1739-1784`), not when the
  first transaction starts. The "limited" branch still changes name, max/min,
  transfer windows, and transfer bypass (`minilith/src/admin.rs:877-942`), in
  conflict with its documentation and schema comment.
- **`min_tickets` is inert:** it is stored and returned but never used to
  reserve/guarantee a distribution. Only max capacity is enforced.
- **First sold-out queue insertion can fail:** the placement expression at
  `minilith/src/ticket.rs:637-650` computes `NULL + 1` when the reservation
  queue is empty, violating the non-null placement column.
- **Unqueue is incomplete:** `DELETE /queue` removes only
  `ticket_release_queuers` (`minilith/src/ticket.rs:674-697`), not a reservation
  queue position.
- **Addon rows are zipped in unspecified order:** request addons are sorted, but
  the `WHERE id = ANY(...)` query has no `ORDER BY`; flags can be validated
  against the wrong addon (`minilith/src/ticket.rs:1827-1923`). Duplicate
  option indices are accepted and charged repeatedly, and `Some("")` satisfies
  a required text field. The API also makes `selected_options` and
  `selected_text` optional (`:157-162`), but inserts those `None` values into
  non-null columns (`:903-915`; `minilith/schema/12-tickets.sql:124-130`), so a
  contract-valid omitted field produces an HTTP 500.
- **Transferred ticket/history response is incomplete:** `GET /tickets` filters
  only current `owner_id` and omits purchaser/validation state
  (`minilith/src/ticket.rs:391-479`). A transferred-away ticket disappears even
  though the requirement says it remains in the purchaser's history; used
  state is also not exposed.
- **External transaction crash gap:** Minilith creates the external transaction
  before attaching its ID to the reservation
  (`minilith/src/ticket.rs:1022-1123`). A crash in that window can leave paid
  money without a locally tracked reservation; the code already acknowledges
  this.

### Transactions and callbacks

- **Provider side effects precede local validation/commit:** Swish and Stripe
  create an external payment/session before inserting the local transaction and
  wares (`transactions/src/api.rs:339-397,537-584`). A DB constraint or commit
  failure leaves an external payment with no local record. Validate first and
  use a durable outbox/idempotent reconciliation design.
- **Refund endpoint is a stub:** `POST /v0/:id/refund` always returns
  "not implemented" after validation (`transactions/src/api.rs:686-717`). This
  may be outside the no-return product policy, but an exposed endpoint must not
  claim an implemented capability.
- **Notification callback delivery is best-effort only:** failed transaction
  callbacks are logged and discarded (`transactions/src/callback.rs:49-63`).
  Startup/timeout reconciliation mitigates some cases, but there is no durable
  delivery queue.
- **Payment API validation is incomplete:** it accepts past timeouts, stores API
  bearer tokens and API keys in reusable plaintext, and has no rate limiting.
  API keys have no expiry or scope.

### Authentication hardening

- **Authorization-code race:** the token handler reads an auth session and only
  invalidates the code after callback and DB work
  (`fed-auth/src/oidc.rs:445-525`). Concurrent exchanges can both pass before
  invalidation. Consume the code atomically before issuing tokens.
- **Callback failure still issues a login:** a non-2xx Minilith auth callback is
  logged but token issuance continues (`fed-auth/src/oidc.rs:484-505`). The
  resulting token may refer to no user row and fail inconsistently downstream.
- **Refresh tokens never expire and cannot be revoked by a user:** rotation is
  transactional, but `fed-auth/schema/1-auth.sql` stores no expiry and there is
  no logout/revocation endpoint.
- **Email-login abuse:** Origin is optional, login is available to arbitrary
  addresses, and there is no rate limit (`fed-auth/src/api.rs:60-118`). An
  attacker can create authorize sessions and use the service to send unlimited
  fixed-template mail.
- **Redirect registration is origin-only:** `is_allowed_domain` compares only
  scheme and authority (`fed-auth/src/oidc.rs:28-60`), not exact registered
  redirect URIs. OAuth errors are concatenated without URL encoding
  (`:281-311`). Register exact redirect/callback URLs.

### Notifications, reports, and images

- **Notification recipients do not match the requirement:** recipients are
  based on ticket-kind allowed groups (`minilith/src/notification-recipients.sql`),
  not the activity's responsible/host groups. A user following the organizer
  can miss the notification, while a user following a broad ticket allowlist
  can receive it.
- **Failed push sends are not retried:** the due notification is deleted even
  when individual providers fail, and is also deleted when push support is
  unconfigured (`minilith/src/runtime.rs:103-201`).
- **Ticket sale categories are not configurable:** report generation always
  assigns base ticket revenue to `"null"`
  (`minilith/src/admin.rs:656-699`). Only addon options can have split
  bookkeeping categories. This does not meet the requirement that tickets and
  addons can both be categorized before or after sale.
- **Report inputs accept nonsensical values:** external sales and fees are
  unrestricted signed integers (`minilith/src/admin.rs:46-60`), so negative
  values can produce invalid totals. There is no automated accounting/legal
  validation of the generated report.
- **Image content is trusted by metadata:** upload policy accepts any
  `image/*` content type independently of extension, and registration checks
  only the key extension (`minilith/src/admin.rs:1234-1306,330-385`). There is
  no byte sniffing/re-encoding, quota, cleanup, or lifecycle policy, so the
  100-GB/three-year requirement is not enforced.

## External-validation account isolation

The plan names `test:external-verification`, but the implemented identity is
`test:external-validation`.

### What is isolated in the current seed

- The account has a single membership in `testing_tlth.e` and no adminship
  (`minilith/src/bin/seed-dev.rs:216-233`).
- Test paths use a separate first ltree label, `testing_tlth`, and deterministic
  IDs are XOR-separated from main fixture IDs (`:35-67`).
- Testing ticket allowlists point at the testing root, while main ticket
  allowlists point at the main root (`:468-581`). Normal path containment
  therefore does not cross between them.
- The namespace unit test passes.

With exactly this seed/configuration, the account cannot directly edit main
groups/activities or purchase main tickets. It can mutate the shared account's
own language, settings, push devices, queues, reservations, and testing tickets.
Every external reviewer shares those records, so reviewers can interfere with
one another.

### Ways the boundary can be crossed

- The fixed bearer authentication itself is global and has no environment
  boundary (SEC-01).
- A main ticket with transfer-bypass enabled can be transferred to this shared
  identity; transfer currently performs no activity-access check. The shared
  bearer can then operate on real ticket state.
- An admin can later add this identity to a main group or configure a testing
  group as a joiner/allowlist. There is no explicit namespace guard to reject
  such links.
- The public test OIDC provider can impersonate other seeded `test:*` users,
  including users that `seed-dev` places in the main namespace (SEC-02).

Conclusion: the seed is logically separated by current ltree data, but this is
not a security boundary. Production must enforce an explicit test tenant or use
a separate database/deployment, in addition to fixing authentication.

## Backend-applicable requirements from `krav.typ`

### User and authentication

- [ ] **Partial — create account and log in with LU:** OIDC/PKCE exists, but LU
  uses MockSAML, the attributes are hard-coded, Medcheck is absent, and no
  section/TLTH membership is assigned.
- [ ] **Partial — list visible activities:** membership/allowlist/filter queries
  exist, but list/detail delegated-admin rules differ, deleted groups remain
  effective, and ticket-release state can violate sale times.
- [x] **Show cross-section activity details:** details include image, localized
  title/description, location/time, responsible contact, all hosts/logos, and
  ticket-kind discovery.
- [ ] **Partial — buy a ticket:** randomized 15–18 minute reservations,
  capacity locking, free/Swish/Stripe calls, and callbacks exist; the final
  one-minute gate, duplicate ownership rule, addon correctness, and durable
  transaction linkage do not.
- [ ] **Partial — list all of my ticket states:** current-owned tickets are
  listed, but transferred-away history, used state, and transfer indication are
  not exposed.
- [ ] **Partial — food preferences/drink packages:** the addon model supports
  choices, prices, required/single/multiple/text flags, but validation can pair
  the wrong addon, duplicate-charge options, and accept empty required text.
- [ ] **Partial — filter activities by independent organization-tree settings:**
  explicit settings and nearest-ancestor inheritance are applied in the
  activity list, but every setting PUT reports an internal error after writing.
- [ ] **Missing/incorrect — transfer tickets:** transfer exists but its time
  window, activity-level permission, activity access fallback, and one-ticket
  invariant are not enforced.
- [x] **List direct memberships:** `/user` returns the user's non-deleted direct
  groups and `/groups/tree` supplies the organization filter tree.
- [x] **Request group membership:** eligible groups, request creation, admin
  listing, and acceptance are implemented.
- [ ] **Missing — organization details for a user:** no user-facing endpoint
  provides arbitrary group details plus admin contacts and permanent leave.
- [ ] **Partial — disable organization notifications:** per-user group
  notification levels are stored and applied to recipient selection, but every
  setting PUT reports an internal error after writing.
- [ ] **Partial — server-down/version support:** a database healthcheck exists,
  but it returns only `Ok :)`; there is no backend compatibility/version result
  or offline ticket/validation data contract.
- [ ] **Missing/incorrect — download a valid receipt:** the Minilith receipt
  integration calls the wrong transactions path and decodes the name as the
  wrong type.
- [x] **Swedish/English data and preferred language:** localized JSON fields,
  encrypted server-side language storage, and language-selected notifications
  exist. The encryption construction itself must be replaced (DATA-02).
- [ ] **Missing — delete/log out account endpoint:** no user account deletion or
  server-side refresh-token revocation endpoint exists.
- [x] **Push registration backend:** authenticated APNs/FCM device registration
  and deregistration are implemented.

### Administration

- [ ] **Partial — email login:** magic-link login exists, but needs rate limiting,
  atomic token/code consumption, and removal of production test auth.
- [ ] **Partial — activity calendar data:** bounded date-range listing and admin
  visibility exist, but list/detail authorization can disagree.
- [ ] **Partial — create/edit an activity for an authorized group:** fields,
  validation, capacity, image registration, immutable creators, and locked host
  authorization exist; stale concurrent field edits are not detected.
- [ ] **Partial — activity image with bounded storage:** presigned 4-MB uploads
  and image DB registration exist; content validation and total quota/lifecycle
  enforcement do not.
- [ ] **Partial — multi-section activity and primary recipient:** multiple hosts
  and an authorized invitation/acceptance flow exist, but there is no
  per-creator settlement/payment-client routing.
- [x] **Restrict ticket purchase to groups:** ticket-kind allowlists enforce
  direct/transitive membership and exclusive-group behavior.
- [ ] **Partial — easily see activity/purchase access:** ticket allowlist IDs and
  membership results are exposed, but there is no authoritative activity-level
  access summary and host/deletion inconsistencies remain.
- [ ] **Partial — configure addons:** full addon/option/bookkeeping structures
  are writable and readable, but selection validation is incorrect.
- [ ] **Missing/incorrect — freeze ticket structure/pricing after the first
  transaction:** the flag starts after payment and the limited edit branch
  changes more fields than allowed.
- [ ] **Missing — detect concurrent admin edits:** no version/precondition or
  conflict response exists.
- [ ] **Partial — edit groups, logos, join rules, and requests:** endpoints and
  image registration exist; hierarchy, deletion behavior, and join-request
  email are incomplete.
- [ ] **Partial — administer groups and all descendants:** subgroup CRUD and
  creator adminship exist, but most descendant operations require direct
  adminship, settings are not inherited, and adminship wrongly changes normal
  membership.
- [ ] **Partial — administer members in bulk:** individual add/remove exists;
  list input/bulk mutation does not.
- [ ] **Missing — minimum/guaranteed ticket distribution:** `min_tickets` is not
  used by allocation logic.
- [ ] **Partial — assign validators and validate tickets:** authorized
  list/grant/revoke endpoints exist and validation reports the ticket owner with
  bounded clock skew, but the QR data remains replayable.
- [ ] **Partial — scheduled activity notifications:** localized scheduled
  ticket-kind notifications exist, but recipient groups do not match activity
  hosts and failed delivery is discarded.
- [x] **Localized notifications:** delivery decrypts each user's language and
  resolves localized title/content accordingly.
- [x] **List ticket purchasers/current owners and addon selections:** the admin
  purchased-ticket endpoint returns purchaser, owner, transaction, and choices.
- [ ] **Partial — addon statistics across ticket kinds:** raw purchase/addon data
  can be fetched per kind, but no grouped cross-kind statistics or near-name
  warning is implemented in the backend.
- [ ] **Partial — bookkeeping sales report:** PDF generation, transaction fees,
  addon splits, and optional external sales/fees exist; base tickets cannot be
  categorized, inputs are insufficiently validated, and legal correctness has
  no test/independent validation.
- [ ] **Missing — Fortnox integration** (optional/possibly advanced requirement).
- [ ] **Missing — GDPR anonymization/deletion by a system administrator:** no
  endpoint remaps all account references to an anonymous identity.

### Developer and advanced requirements

- [x] **Add a new API area with localized changes:** routers are separated by
  feature and composed in each service entrypoint.
- [ ] **Partial — version APIs and keep old clients working for three months:**
  route prefixes use `/v0` or `/v1`, but there is no compatibility policy,
  parallel old implementation, or tested deprecation mechanism.
- [ ] **Partial — realistic no-money testing:** seeded scenarios, SQLx fixtures,
  and the zero-value free provider exist; critical paid, transfer, receipt,
  validation, notification, image, and provider-webhook flows lack end-to-end
  tests. Production test backdoors are not an acceptable test environment.
- [ ] **Partial — group administration with a restricted API token:** fed-auth
  can exchange an unscoped, non-expiring generic API key for a user token, but
  there is no group-scoped key lifecycle or restriction model.
- [ ] **Missing — group news with spam limits** (advanced requirement).
- [ ] **Missing — offline ticket validation bundle/synchronization** (advanced
  requirement).
- [ ] **Missing — automatic membership-register reconciliation** (advanced
  requirement).
- [ ] **Missing — membership proof during ticket validation** (advanced
  requirement).
- [ ] **Partial — physical-goods sales:** the activity/ticket model can emulate a
  bounded sale, but cannot represent a timeless standing sale or keep it out of
  the activity feed (advanced requirement).
- [ ] **Partial — sales to external users:** email identities and arbitrary root
  groups make parts possible, but no invitation/code flow or protected external
  login experience exists (advanced requirement).
- [ ] **Missing — calendar export/feed** (advanced requirement).

## Revalidation order

1. Remove/gate both test authentication paths and replace MockSAML.
2. Close inherited production ports and fix Stripe webhook secrets.
3. Fix adminship/membership semantics, transfer invariants, replay-proof ticket
   validation, and receipt integration. Activity-host authorization is fixed.
4. Make `seed-dev` production-safe or split it from production bootstrap.
5. Fix ticket timing/immutability/queue/addon contracts and replace encryption.
6. Complete the remaining requirement checklist and add endpoint-level tests for
   every fixed authorization and payment boundary.
