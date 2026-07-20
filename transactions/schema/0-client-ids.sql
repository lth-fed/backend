create table client_ids (
    client_id text primary key,
    swish_number text not null,

    -- for receipts
    name text not null,
    email text not null,
    address text not null,
    organization_number text not null
);
