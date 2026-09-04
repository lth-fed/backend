CREATE FUNCTION array_sum_money(money[]) RETURNS money
   LANGUAGE sql IMMUTABLE STRICT AS
'SELECT sum(e) FROM unnest($1) AS a(e)';

create table images (
    id uuid primary key default uuidv4(),
    created timestamptz not null default now(),
    size bigint not null,
    url text not null
);

-- Records every upload allowance, including uploads that are never attached to an entity.
-- This is the root set for future object-storage garbage collection.
create table image_uploads (
    key text primary key
);
