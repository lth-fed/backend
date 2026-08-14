#!/bin/env bash

cd "$(dirname "$0")"
cd minilith
cargo sqlx prepare -- --all-targets
cd ../fed-auth
cargo sqlx prepare -- --all-targets
cd ../transactions
cargo sqlx prepare -- --all-targets
