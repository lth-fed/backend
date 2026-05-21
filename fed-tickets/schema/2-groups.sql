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

    membership_inherits_upward boolean not null default true,
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
create or replace view effective_group_memberships as
    select distinct
        m.user_id,
        g.id as group_id,
        g.path as group_path
    from group_memberships m
    join groups member_group on m.group_id = member_group.id
    join groups g on g.path @> member_group.path
    where not member_group.deleted
    and not g.deleted
    and (
        member_group.membership_inherits_upward = true
        or member_group.path = g.path
    );

create table group_adminships (
    user_id text not null references users(id),
    group_id uuid not null references groups(id),
    primary key (user_id, group_id)
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
