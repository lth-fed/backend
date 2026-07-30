-- Any number of these attributes can be null
create type location as (
    -- if you want to insert just one string, set it under e.g. "generic" here
    -- (since the lang code tries to find first the string for the chosen language,
    -- then english, then swedish, then the first in the table, which would be "generic")
    name jsonb,
    directions jsonb,
    coordinate_wgs84 point,
    url text
);

-- for an activity without tickets, the frontend should create a free ticket with max_tickets = 0, to specify who can view the activity
create table activities (
    id uuid primary key default uuidv4(),
    -- has to be user with email
    responsible_id text not null references users(id),
    responsible_contact text not null,
    creator_id uuid not null references groups(id),
    --
    title jsonb not null,
    description jsonb not null,
    location location not null,
    time_start timestamptz not null,
    time_end timestamptz not null,
    image_id uuid not null references images(id),
    -- only for normal users, admins can always see them
    is_hidden boolean not null default true,
    is_hidden_for_other_admins boolean not null default false,
    max_tickets integer not null check (max_tickets > 0) -- default MAX_INT
);

-- co-hosts of event. MUST include activities.creator_id
create table activity_hosts (
    activity_id uuid not null references activities(id),
    group_id uuid not null references groups(id),
    primary key (activity_id, group_id)
);
-- invites to be `activity_hosts`
create table activity_host_invites (
    activity_id uuid not null references activities(id),
    group_id uuid not null references groups(id),
    primary key (activity_id, group_id)
);

create table activity_verifiers (
    activity_id uuid not null references activities(id),
    user_id text not null references users(id),
    primary key (activity_id, user_id)
);
