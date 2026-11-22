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

    --
    name jsonb not null,
    description jsonb not null,
    logo_id uuid not null references images(id),
    -- only to be set for 
    -- - "area" groups, i.e. Lund (`lund`), where everybody can join the group
    --   (since parent is null, that means in this context everybody)
    -- - child groups where we want to allow members in parent group to join children without asking
    parent_members_can_join bool not null default false,
    -- opt-in for stock movements / stock UI
    stock_tracking bool not null default false,
    -- only acts as a barrier "upwards" when admin, i.e. if I'm admin of E-guild
    -- & this is set for E-guild, I still see members in subgroups, but their
    -- admins don't see E-guild's members.
    propogate_member_visibility_access bool not null default false,
    deleted boolean not null default false
);

-- GiST index for ltree ancestor/descendant operators (@>, <@).
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
    -- The identity may be invited before its user row is created on first login.
    user_id text not null,
    group_id uuid not null references groups(id),
    -- if this is a time-limited membership:
    until timestamptz,
    -- limits this member's visibility for other parts of the tree
    specific boolean not null default false,
    primary key (user_id, group_id)
);

create table group_adminships (
    -- Admin email identities may be invited before their first login.
    user_id text not null,
    group_id uuid not null references groups(id),
    -- if this is a time-limited membership:
    until timestamptz,
    primary key (user_id, group_id),
    constraint group_adminships_email_account check (user_id like 'email:%' or user_id like 'api:%'),
    -- ensure there exists a group membership with the same user_id and group_id
    constraint group_adminships_group_membership_fk foreign key (user_id, group_id)
        references group_memberships (user_id, group_id)
);

-- grants the admins of `access_group_id` view of the activities where we are creators or co-hosts
create table allow_admins_from_group_view_activities (
    host_group_id uuid not null references groups(id) on delete cascade,
    access_group_id uuid not null references groups(id) on delete cascade,
    primary key (host_group_id, access_group_id)
);

create table sales_targets (
    id uuid primary key,
    group_id uuid not null references groups(id),
    email text not null,
    iban text not null
);
