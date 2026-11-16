#!/bin/sh
set -eu
# Fly app secrets are shared; map uniquely named secrets to each service's existing contract.
case "$SERVICE_NAME" in
  minilith)
    export DATABASE_URL="${MINILITH_DATABASE_URL:-${DATABASE_URL:-}}"
    export MAIL_EMAIL="${MINILITH_MAIL_EMAIL:-${MAIL_EMAIL:-}}" ;;
  fed-auth)
    export DATABASE_URL="${AUTH_DATABASE_URL:-${DATABASE_URL:-}}"
    export PRIVATE_KEY="${AUTH_PRIVATE_KEY:-${PRIVATE_KEY:-}}"
    export MAIL_EMAIL="${AUTH_MAIL_EMAIL:-${MAIL_EMAIL:-}}" ;;
  transactions)
    export DATABASE_URL="${TRANSACTIONS_DATABASE_URL:-${DATABASE_URL:-}}"
    export PRIVATE_KEY="${TRANSACTIONS_PRIVATE_KEY:-${PRIVATE_KEY:-}}" ;;
  *) exit 1 ;;
esac
if [ -n "${DATABASE_GENERATION:-}" ]; then
  case "$DATABASE_GENERATION" in *[!a-z0-9_]*) exit 1 ;; esac
  # Deployment URLs deliberately have no query string; candidate databases are a suffix.
  case "$DATABASE_URL" in *\?*) echo 'DATABASE_GENERATION requires a URL without query parameters' >&2; exit 1 ;; esac
  export DATABASE_URL="${DATABASE_URL}${DATABASE_GENERATION}"
fi
exec "/app/$SERVICE_NAME" "$@"
