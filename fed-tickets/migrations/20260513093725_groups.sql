create extension ltree;

create table groups (
    id uuid primary key default uuidv4(),
    path ltree not null unique,

    -- This generated column and fk constraint ensures that no non-root groups
    -- are orphans, i.e. that they have existing parents.
    --
    -- Source: https://dba.stackexchange.com/a/337753
    parent_path ltree generated always as (
        case
            when nlevel("path") > 1 then subpath("path", 0, nlevel("path") - 1)
            else null
        end
    ) stored,
    foreign key (parent_path) references groups(path),

    limit_membership_visibility boolean not null,
    --
    name jsonb not null,
    description jsonb not null,
    -- logo_id uuid not null references images(id),
    deleted boolean not null default false
);

-- GiST index for ltree ancestor/descendant operators (@>, <@). The b-tree
-- created by the unique constraint on `path` does not accelerate these.
create index groups_path_gist on groups using gist (path);

-- members from which other groups can ask to join this group?
create table groups_ask_to_join (
    target_path ltree not null references groups(path) on update cascade,
    joiner_path ltree not null references groups(path) on update cascade,
    primary key (target_path, joiner_path)
);

-- -- needs to be validate in backend that user is allowed before creating this
-- create table group_member_requests (
--     member_id uuid not null references users(id),
--     group_id uuid not null references groups(id),
--     primary key (group_id, member_id)
-- );

create table group_memberships (
    user_id text not null references users(id),
    group_path ltree not null references groups(path) on update cascade,
    primary key (user_id, group_path)
);

create table group_adminships (
    user_id text not null references users(id),
    group_path ltree not null references groups(path) on update cascade,
    primary key (user_id, group_path),
    -- ensure there exists a group membership with the same user_id and group_path
    constraint group_adminships_group_membership_fk foreign key (user_id, group_path)
        references group_memberships (user_id, group_path)
        on update cascade
        on delete cascade
);

-- create type notification_level as enum ('none', 'personalized', 'all');
-- create table group_notifications (
--     user_id uuid not null references users(id),
--     group_id uuid not null references groups(id),
--     --
--     level notification_level not null,
--     primary key (group_id, user_id)
-- );
