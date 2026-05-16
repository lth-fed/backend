#!/bin/sh

cd "$(dirname "$0")"

rm schema.sql
touch schema.sql

for f in $(ls -v); do
    if ! echo $f | grep "[0-9]*-.*\\.sql" > /dev/null; then continue; fi
    echo "Adding $f to the schema."
    cat $f >> schema.sql
done

echo "Compiled schema to schema.sql."
