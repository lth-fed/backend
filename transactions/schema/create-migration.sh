#!/bin/sh

cd "$(dirname "$0")"

set -e

if [ -s .venv ]; then
    echo "You've got everything set up!"
else
    echo "Installing needed tools..."
    echo
    echo
    if ! which uv; then
        echo "You need to have UV (python) installed!"
        exit 1
    fi
    uv venv
    uv pip install results

    echo
    echo
    echo "Everything's installed!"
    echo
    echo
fi

name=$1
if [ -z $name ]; then
    echo "Name cannot be empty!"
    exit 1
fi

./compile.sh

echo
echo

echo "Starting temporary databases..."
podman compose up -d 2>/dev/null

echo "Waiting for postgres to be up and running..."
while ! PGPASSWORD=postgres psql -h localhost -p 9832 -U postgres -d dev -a -c "" 2>/dev/null; do
    sleep 1
done
while ! PGPASSWORD=postgres psql -h localhost -p 9833 -U postgres -d dev -a -c "" 2>/dev/null; do
    sleep 1
done

echo "Inserting schema into one DB"
PGPASSWORD=postgres psql -h localhost -p 9833 -U postgres -d dev -a -f schema.sql>/dev/null

cd ..
echo "Inserting old schema into another DB"
sqlx database reset -D "postgres://postgres:postgres@localhost:9832/dev" -fy >/dev/null
cd schema
PGPASSWORD=postgres psql -h localhost -p 9832 -U postgres -d dev -a -c "drop table _sqlx_migrations">/dev/null

filename="$(date +%+4Y%m%d%H%M%S)_$name.sql"
path=../migrations/"$filename"

echo "Creating migration..."
mkdir -p ../migrations
echo $(uv run results dbdiff --schema public postgresql://postgres:postgres@localhost:9832/dev postgresql://postgres:postgres@localhost:9833/dev >$path)

if [ -s $path ]; then
    echo "Done! New migration: $filename"
    echo
else
    echo "Migration is empty!"
    rm $path
fi

echo "Shutting down databases..."
podman compose down 2>/dev/null
rm -rf .db
