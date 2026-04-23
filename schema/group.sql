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
    deleted boolean not null
);

-- members from which other groups can ask to join this group?
create table groups_ask_to_join (
    id uuid primary key,
    target_id uuid not null references groups(id),
    joiner_id uuid not null references groups(id)
);

-- needs to be validate in backend that user is allowed before creating this
create table group_member_requests (
    id uuid primary key,
    member_id uuid not null references users(id),
    group_id uuid not null references groups(id)
);

create table group_members (
    id uuid primary key,
    member_id uuid not null references users(id),
    group_id uuid not null references groups(id),
    is_admin boolean not null
);

create type notification_level as enum ('none', 'personalized', 'all');
create table group_notifications (
    id uuid primary key,
    user_id uuid not null references users(id),
    group_id uuid not null references groups(id),
    --
    level notification_level not null
);
