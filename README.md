# FED Backend

## SQLx

> Before committing, run `cargo sqlx prepare -- --all-targets`. Without
> `--all-targets` it doesn't generate the test files.

> Outer joins must override the variable to be optional:
> `select id, stripe_id as "stripe_id?" from transactions txn left outer join stripe_checkouts s on txn.id = s.transaction_id`
> otherwise SQLx will try to parse it as if it could not be null. This might be
> true even if the field in question is nullable.

We glue Rust together with Postgres using
[SQLx](https://github.com/launchbadge/sqlx/blob/main/README.md). It handles
migrations and does compile-time checks of the queries. To create new
migrations, you will need
[`sqlx-cli`](https://github.com/launchbadge/sqlx/blob/main/sqlx-cli/README.md)
as well as a database.

To spin up a local Postgres instance, run `docker compose up -d`. The connection
url, `DATABASE_URL`, is specified in `.env` and used by both the backend and the
cli. It _should_ work out of the box.

The database will be automatically initialized when you start the backend.

For making the database types work in the CI, please run `cargo sqlx prepare`
whenever you add new queries or migrations.

> This is because in the CI we don't have a database, so sqlx needs to know the
> type of the queries beforehand, which `cargo sqlx prepare` does by writing
> files about the queries.

## Auth

The auth tries to get certs from the local auth (on port `8001`). If this fails,
it defaults to always returning the user `lund-university:aa0000bb-s` or
whatever is after `Bearer` in the `authorization` header.

## Grafana & metrics & logs

See the [guide](https://docs.teknologappen.se/grafana/) on how to use Grafana to
view metrics, errors, logs etc.

## Local development

Start the shared infrastructure with:

```sh
podman compose up -d
```

SQLx performs compile-time query checks. After adding or changing queries, run
`cargo sqlx prepare` so CI can build with `SQLX_OFFLINE=true`.

The auth verifier uses the local auth service during development and the public
HTTPS API in production.

## Build and push production images

Production secrets are runtime configuration and are never needed to build the
images. The backend `.dockerignore` excludes every `.env` file.

Choose a registry and an immutable tag, then build and push:

```sh
export CONTAINER_REGISTRY=registry.esek.se/esek
export CONTAINER_TAG=0.0.1-alpha.1

podman login registry.esek.se
./build-images.sh
POSTGRES_PASSWORD=unused GRAFANA_ADMIN_PASSWORD=unused podman compose -f compose.prod.yaml push fed-auth transactions minilith
```

`build-images.sh` supplies placeholder values for the two secrets which Compose
validates while reading the file. They are not build arguments and are not
stored in an image. It requires `CONTAINER_REGISTRY` and `CONTAINER_TAG` so a
build cannot silently create differently named local images. The Dockerfile uses
`cargo-chef`, so dependency compilation is reused after source-only changes. The
script also embeds Git's compact `<revision>[-modified]` description without
sending the `.git` directory into the container build.

## Deploy pushed images

The deployment host needs this repository's `compose.yaml`, `compose.prod.yaml`,
observability configuration, and the three service `.env` files. It does not
need the Rust source or build tooling. It's however the easiest to just pull
this repo (or run
`rsync compose.prod.yaml compose.yaml grafana.yml loki-config.yaml otel-collector.yaml prometheus.yml tempo.yaml macapar@extrovert:/srv/teknologappen/backend/staging`
and then `sudo cp staging/* .` from the directory).

Create an untracked `backend/.env`:

```dotenv
CONTAINER_REGISTRY=registry.esek.se/esek
CONTAINER_TAG=0.0.1-alpha.1

POSTGRES_USER=postgres
POSTGRES_PASSWORD=replace-with-a-long-random-password
GRAFANA_ADMIN_USER=admin
GRAFANA_ADMIN_PASSWORD=replace-with-a-long-random-password

TRAEFIK_NETWORK=traefik
TRAEFIK_ENTRYPOINT=websecure
TRAEFIK_CERT_RESOLVER=letsencrypt
MINILITH_DOMAIN=api.teknologappen.se
FED_AUTH_DOMAIN=api.auth.teknologappen.se
TRANSACTIONS_DOMAIN=transactions.teknologappen.se
GRAFANA_DOMAIN=grafana.teknologappen.se
S3_DOMAIN=s3.teknologappen.se
S3_CONSOLE_DOMAIN=s3-console.teknologappen.se
```

Copy each `.env.example` to `.env` in `minilith`, `fed-auth`, and
`transactions`, then replace the example values. Compose supplies
`DATABASE_URL`, so it may be omitted from these service files.

The host must also have:

- DNS records for the application and Grafana domains pointing to the host.
- A running Traefik instance with its Docker provider connected to the
  Docker/Podman API socket.
- A `websecure` entrypoint and an ACME certificate resolver named `letsencrypt`
  (or matching values in `.env`).
- The external network shared by Traefik and this stack:

```sh
podman network exists traefik || podman network create traefik
```

Deploy only the already-pushed application images:

```sh
podman compose -f compose.prod.yaml pull fed-auth transactions minilith
podman compose -f compose.prod.yaml up -d --no-build
```

You must also insert a token for `minilith` into the `transactions` DB. Generate
a random UUID and insert it into the DB and then set it as the env variable for
`minilith/.env#TRANSACTIONS_TOKEN`.

### Scale minilith

There is no host-port binding on `minilith`. Traefik discovers every replica
through the shared network and load-balances them under the same domain:

```sh
podman compose -f compose.prod.yaml up -d --no-build --scale minilith=3
```

All replicas intentionally share the same environment and PostgreSQL database.
