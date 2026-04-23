create table images (
    id uuid primary key,
    created timestamp not null,
    size bigint not null,
    url text not null
)
