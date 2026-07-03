# FED Backend

## SQLx

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

The environment variable `TESTING` (at compile time!) can be set to `true` or
`yes`, which makes the `User` extractor return `lund-university:aa0000bb-s`.

## Grafana & metrics & logs

See the [guide](https://docs.teknologappen.se/grafana/) on how to use Grafana to
view metrics, errors, logs etc.
