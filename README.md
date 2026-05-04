# FED Backend

## SQLx

We glue Rust together with Postgres using [SQLx](https://github.com/launchbadge/sqlx/blob/main/README.md). It handles migrations and does compile-time checks of the queries. To create new migrations, you will need [`sqlx-cli`](https://github.com/launchbadge/sqlx/blob/main/sqlx-cli/README.md) as well as a database.

To spin up a local Postgres instance, run `docker compose up -d`. The connection url, `DATABASE_URL`, is specified in `.env` and used by both the backend and the cli. It _should_ work out of the box.

The database will be automatically initialized when you start the backend.
