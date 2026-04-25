create type group_type as enum ('private', 'exclusive');
create table groups (
    id uuid primary key,
    parent_id uuid references groups(id),
    -- not source of truth, this is a cached value
    -- see https://github.com/lth-fed/issues/issues/4#issuecomment-4296352880
    exclusive_top_parent_id uuid not null references groups(id),
    --
    name jsonb not null,
    description jsonb not null,
    logo_id uuid not null references images(id),
    type group_type not null,
    deleted boolean not null,
    unique (name, parent_id)
);

-- members from which other groups can ask to join this group?
create table groups_ask_to_join (
    target_id uuid not null references groups(id),
    joiner_id uuid not null references groups(id),
    primary key (target_id, joiner_id)
);

-- needs to be validate in backend that user is allowed before creating this
create table group_member_requests (
    member_id uuid not null references users(id),
    group_id uuid not null references groups(id),
    primary key (group_id, member_id)
);

create table group_members (
    member_id uuid not null references users(id),
    group_id uuid not null references groups(id),
    is_admin boolean not null,
    primary key (group_id, member_id, is_admin)
);

create type notification_level as enum ('none', 'personalized', 'all');
create table group_notifications (
    user_id uuid not null references users(id),
    group_id uuid not null references groups(id),
    --
    level notification_level not null,
    primary key (group_id, user_id)
);
