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

-- for an activity without tickets, the admin frontend should create a free ticket
-- with stock = 0, to specify who can view the activity
create table activities (
    id uuid primary key default uuidv4(),
    responsible_name text not null,
    responsible_contact text not null,
    -- can be changed by admins afterwards
    sales_target_id uuid not null references sales_targets(id),
    --
    title jsonb not null,
    description jsonb not null,
    location location not null,
    time_start timestamptz not null,
    time_end timestamptz not null check (time_end > time_start),
    image_id uuid not null references images(id),
    -- only for normal users, admins can always see them
    is_hidden boolean not null default true,
    is_hidden_for_other_admins boolean not null default false,
    --
    -- should be references products(id),, but that'd make a cyclic dependency
    -- (+ it's on 12-... which is after this file!)
    --
    -- This can be set so that the randomized tickets fall back to this in case
    -- they sell out for individual tickets.
    -- Must always exist due to visibility concerns.
    base_ticket uuid not null,
    max_tickets_per_user integer not null default 1 check (max_tickets_per_user >= 1),

    -- for sales of stock items either:
    -- - to users in app at this activity 
    -- - OR to customers on location
    stock_group uuid references groups(id),

    -- Set only after the automatic sales report email has been accepted by SMTP.
    bookkept boolean not null default false
);
create table activity_tags (
    id uuid primary key,
    creator_id uuid not null references groups(id),
    -- hide instead of deleting so existing assignments and opt-outs stay valid
    hidden bool not null default false,
    title jsonb not null
);
create table activity_assigned_tags (
    activity_id uuid not null references activities(id),
    tag_id uuid not null references activity_tags(id),
    primary key (activity_id, tag_id)
);
create table activity_opt_out_tags (
    user_id text not null references users(id),
    tag_id uuid not null references activity_tags(id),
    primary key (user_id, tag_id)
);

-- co-hosts of event.
create table activity_hosts (
    activity_id uuid not null references activities(id) on delete cascade,
    group_id uuid not null references groups(id),
    primary key (activity_id, group_id)
);
-- invites to be `activity_hosts`
create table activity_host_invites (
    activity_id uuid not null references activities(id) on delete cascade,
    group_id uuid not null references groups(id),
    primary key (activity_id, group_id)
);

create table activity_verifiers (
    activity_id uuid not null references activities(id) on delete cascade,
    user_id text not null references users(id),
    primary key (activity_id, user_id)
);

create table activity_followers (
    user_id text not null references users(id),
    activity_id uuid not null references activities(id) on delete cascade,
    primary key(user_id, activity_id),
    follow boolean not null
);
