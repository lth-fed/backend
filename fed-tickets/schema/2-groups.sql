create extension ltree;

create table groups (
    admin_path ltree primary key,
    limit_membership_visibility boolean not null,
    --
    name jsonb not null,
    description jsonb not null,
    logo_id uuid not null references images(id),
    deleted boolean not null
);

-- members from which other groups can ask to join this group?
create table groups_ask_to_join (
    target_id ltree not null references groups(admin_path),
    joiner_id ltree not null references groups(admin_path),
    primary key (target_id, joiner_id)
);

-- needs to be validate in backend that user is allowed before creating this
create table group_member_requests (
    member_id text not null references users(id),
    group_id ltree not null references groups(admin_path),
    primary key (group_id, member_id)
);

create table group_members (
    member_id text not null references users(id),
    group_id ltree not null references groups(admin_path),
    is_admin boolean not null,
    primary key (group_id, member_id, is_admin)
);

create type notification_level as enum ('none', 'personalized', 'all');
create table user_group_settings (
    user_id text not null references users(id),
    group_id ltree not null references groups(admin_path),
    --
    notification_level notification_level not null,
    visible boolean not null,
    primary key (group_id, user_id)
);
