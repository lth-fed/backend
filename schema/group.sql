create table groups (
    admin_path ltree primary key,
    limit_membership_visibility boolean not null,
    --
    name jsonb not null,
    description jsonb not null,
    logo_id uuid not null references images(id),
    type group_type not null,
    deleted boolean not null
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
