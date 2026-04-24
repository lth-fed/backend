create table activities (
    id uuid primary key,
    -- has to be user with email
    responsible_id text not null references users(id),
    creator_id uuid not null references groups(id),
    --
    title jsonb not null,
    description jsonb not null,
    location text not null,
    time tsrange not null,
    image_id uuid not null references images(id),
    is_hidden boolean not null,
    max_tickets integer not null check (max_tickets > 0) -- default MAX_INT
);

-- co-hosts of event. should not include activities.creator_id
create table activity_hosts (
    id uuid primary key,
    activity_id uuid not null references activities(id),
    group_id uuid not null references groups(id)
);
