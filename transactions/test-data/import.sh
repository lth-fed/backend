#!/bin/sh

PGPASSWORD=postgres psql -h localhost -p 5432 -U postgres -d "${2:-transactions}" -a -f "$1"
