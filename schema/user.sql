create table users (
    -- `stil-id:<stil-id>` or `email:<email>`
    id text primary key,
    name text not null,
    language text not null,
    -- don't remove name when becoming inactive!
    is_active boolean not null
    -- constraint no_name_when_deleted check ((not is_active) and name = null or is_active and name != null)
);
