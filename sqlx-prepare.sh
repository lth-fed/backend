#!/bin/env bash

cd "$(dirname "$0")"
cd minilith
sqlx migrate run
cargo sqlx prepare -- --all-targets
cd ../fed-auth
sqlx migrate run
cargo sqlx prepare -- --all-targets
cd ../transactions
sqlx migrate run
cargo sqlx prepare -- --all-targets
