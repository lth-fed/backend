create table images (
    id uuid primary key,
    created timestamptz not null,
    size bigint not null,
    url text not null
);
