create extension ltree;

create table groups (
    id uuid primary key default uuidv4(),
    path ltree not null unique,

    -- This generated column and fk constraint ensures that no non-root groups
    -- are orphans, i.e. that they have existing parents. The self-reference
    -- stays path-based because parent_path is derived from path via subpath();
    -- you can't compute the parent's uuid from a child's path.
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
    logo_id uuid not null references images(id),
    deleted boolean not null default false
);

-- GiST index for ltree ancestor/descendant operators (@>, <@). The b-tree
-- created by the unique constraint on `path` does not accelerate these.
create index groups_path_gist on groups using gist (path);

-- members from which other groups can ask to join this group?
create table groups_ask_to_join (
    target_id uuid not null references groups(id),
    joiner_id uuid not null references groups(id),
    primary key (target_id, joiner_id)
);

-- needs to be validate in backend that user is allowed before creating this
create table group_member_requests (
    member_id text not null references users(id),
    group_id uuid not null references groups(id),
    primary key (group_id, member_id)
);

create table group_memberships (
    user_id text not null references users(id),
    group_id uuid not null references groups(id),
    primary key (user_id, group_id)
);

-- a view of all group memberships, including inherited memberships
create view transitive_group_memberships as
    select user_id, group_id
    from group_memberships
    inner join groups member_groups on (
        group_memberships.group_id = member_groups.id
    )
    inner join groups on (
        member_groups.path @> groups.path
    )
    where (
        member_groups.limit_membership_visibility = false
        or groups.id = member_groups.id
    );

create table group_adminships (
    user_id text not null references users(id),
    group_id uuid not null references groups(id),
    primary key (user_id, group_id),
    -- ensure there exists a group membership with the same user_id and group_id
    constraint group_adminships_group_membership_fk foreign key (user_id, group_id)
        references group_memberships (user_id, group_id)
        on delete cascade
);

create type notification_level as enum ('none', 'personalized', 'all');
create table user_group_settings (
    user_id text not null references users(id),
    group_id uuid not null references groups(id),
    --
    visible boolean not null,
    notification_level notification_level not null,
    primary key (group_id, user_id)
);
