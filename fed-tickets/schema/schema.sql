create table users (
    -- `lund-university:<stil-id>` or `email:<email>`
    id text primary key,
    -- needs to be pseudoanonymised according to GDPR, therefore we have to store the encrypted bytes
    name bytea not null,
    -- same as for name, also remember to salt so the encrypted data isn't the same for all users with the same language
    language bytea not null,
    -- used by encryption
    nonce bytea not null,
    -- GDPR remove all accounts older than 2 years after this: this is a GDPR requirement
    latest_refresh timestamptz not null,
    creation timestamptz not null,
    -- don't remove name when becoming inactive!
    inactive_since timestamptz
    -- constraint no_name_when_deleted check ((not is_active) and name = null or is_active and name != null)
);
create table images (
    id uuid primary key,
    created timestamptz not null,
    size bigint not null,
    url text not null
);
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

create table activities (
    id uuid primary key,
    -- has to be user with email
    responsible_id text not null references users(id),
    creator_id ltree not null references groups(admin_path),
    --
    title jsonb not null,
    description jsonb not null,
    location location not null,
    time_start timestamptz not null,
    time_end timestamptz not null,
    image_id uuid not null references images(id),
    is_hidden boolean not null,
    max_tickets integer not null check (max_tickets > 0) -- default MAX_INT
);

-- co-hosts of event. should not include activities.creator_id
create table activity_hosts (
    activity_id uuid not null references activities(id),
    group_id ltree not null references groups(admin_path),
    primary key (activity_id, group_id)
);
-- invites to be `activity_hosts`
create table activity_host_invites (
    activity_id uuid not null references activities(id),
    group_id ltree not null references groups(admin_path),
    primary key (activity_id, group_id)
);
-- this file is ordered according to how a user interacts with the system:)

CREATE FUNCTION array_sum_money(money[]) RETURNS money
   LANGUAGE sql IMMUTABLE STRICT AS
'SELECT sum(e) FROM unnest($1) AS a(e)';

create table ticket_kinds (
    id uuid primary key,
    activity_id uuid not null references activities(id),
    -- addons ticket_addons[] not null,
    -- allowed_grous set<ticket_kind_allowed_groups> not null,
    --
    name jsonb not null,
    -- in ören
    price money not null check (price >= 0::money),
    purchasing_available_start timestamptz not null,
    purchasing_available_stop timestamptz not null,
    max_tickets integer not null check (max_tickets > 0), -- default MAX_INT
    min_tickets integer not null check (min_tickets > 0), -- default MAX_INT
    check (min_tickets < max_tickets),
    -- we need this as a lock so we don't make too many
    -- https://github.com/lth-fed/backend/pull/7#discussion_r3145792223
    reserved_or_purchased_tickets integer not null
    check (reserved_or_purchased_tickets >= 0 and reserved_or_purchased_tickets <= max_tickets),
    -- to disable, make the range empty
    -- to allow transfer without bounds, just set this to a REALLY long interval
    allow_transfer_ticket_start timestamptz not null,
    allow_transfer_ticket_stop timestamptz not null,
    -- allows tickets to be transferred to users which do not pass the `ticket_kind_allowed_groups` check
    allow_transfer_ticket_bypass_allowed_groups boolean not null,
    -- when this is set nothing is allowed to be changed, except the bookkeeping on table:options & purchasing_available
    has_been_purchased boolean not null
);
-- which groups are allowed to buy this ticket kind
create table ticket_kind_allowed_groups (
    ticket_kind_id uuid not null references ticket_kinds(id),
    group_id ltree not null references groups(admin_path),
    primary key (group_id, ticket_kind_id)
);
-- these are examples for how this table can be used:
-- matpref: options tom, has_text_field = true, required= false
-- dryckespaket: options: ["alkohol", "alkoholfritt", "inget"], has_text_field = false, required = true, multiple_alternatives = false
-- matpref val: options ["vego", "vegan", "nötter"], has_text_field = true, required=false, multiple_alternatives=true
create table ticket_addons (
    id uuid primary key,
    ticket_kind_id uuid not null references ticket_kinds(id),
    idx integer not null,
    -- <virtual foreign key> ticket_addon_options[] not null,
    --
    multiple_alternatives boolean not null,
    has_text_field boolean not null,
    required boolean not null
);
-- this is basically an array
create table ticket_addon_options (
    id uuid primary key,
    ticket_addon_id uuid not null references ticket_addons(id),
    idx integer not null,
    --
    name jsonb not null,
    price money not null check (price >= 0::money),
    -- for the books, if e.g. 20SEK went to spirits, 15SEK to wine etc.
    -- add price here & category below. The index of the item maps it to it's price / category

    -- this is not optimal but composite types & sum constraints seemed very sus
    bookkeeping_prices money[] not null,
    bookkeeping_price_categories text[] not null,
    constraint bookkeeping_prices_add_up check (array_sum_money(bookkeeping_prices) = price),
    constraint bookkeeping_lengths_consistent check (cardinality(bookkeeping_prices) = cardinality(bookkeeping_price_categories))
);

-- people who have started queuing to buy a ticket
-- only applicable at biljettsläpp
-- once the ticket is released for purchase, convert all the queuers to `ticket_queue` with random placements
-- then start a worker which pops the user with the highest placement as long as there are tickets available
-- but for efficiency, don't put the N people with highest placement in `ticket_queue`, just move all of them to
-- `ticket_reservations` directly & initiate transactions
create table ticket_queuers (
    id uuid primary key,
    -- a biljettsläpp is only per ticket_kind, not per activity
    ticket_id uuid not null references ticket_kinds(id),
    user_id text not null references users(id),
    -- remove after 20 minutes, should refresh after 15 minutes
    started_queueing timestamptz not null

    -- can't check this since it requires an INNER JOIN on ticket_kinds (BACKEND HAS TO CHECK!)
    -- constraint not_queuing_for_multiple_ticket_types unique (ticket_id->activity_id, user_id)
);
-- there should be a worker that every second or so checks if there are any available tickets,
-- then pop the user with the best placement & converts it into a reservation & decrements available
-- tickets (transaction)
--
-- I HAVE NOT VALIDATED THAT THINGS WILL BE CONSISTENT IF THERE ARE SEVERAL WORKERS PER TICKET_KIND
--
-- If there are no reservations & no available tickets, clear this table and (notify users?) and stop worker
-- Upon server startup, check if there are any people in this queue, for every ticket_kind, and if there is,
--     start the worker.
-- The worker should start after the biljettsläpp if there were more people interested than there was tickets
create table ticket_queue (
    id uuid primary key,
    ticket_id uuid not null references ticket_kinds(id),
    user_id text not null references users(id),
    placement integer not null check (placement >= 0)
);

-- people who have reserved a ticket
create table ticket_reservations (
    id uuid primary key,
    ticket_id uuid not null references ticket_kinds(id),
    user_id text not null references users(id),
    -- remove after this!
    -- or if transaction is currently happening and not cancellable wait for max an hour or smth
    timeout timestamptz not null

    -- can't check this since it requires an INNER JOIN on ticket_kinds (BACKEND HAS TO CHECK!)
    --constraint not_reserve_multiple_ticket_kinds unique (ticket_id->activity_id, user_id)
);

-- ONE PERSON CAN ONLY OWN ONE TICKET KIND PER ACTIVITY
-- the backend has to check this when both buying and transferring a ticket!
create table purchased_tickets (
    id uuid primary key,
    ticket_kind_id uuid not null references ticket_kinds(id),
    -- if these are not the same the ticket is clearly transferred
    purchaser_id text not null references users(id),
    owner_id text not null references users(id),
    -- could have price here, since it's not allowed to be changed once a ticket has been purchased, but it's just unnecessary because it's easy to calculate
    constraint max_one_ticket_per_person_per_activity unique (ticket_kind_id, owner_id)
);
create table purchased_ticket_addons (
    id uuid primary key,
    addon_id uuid not null references ticket_addons(id),
    ticket_id uuid not null references purchased_tickets(id),
    ---
    selected_options integer[] not null,
    selected_text text not null
);
-- clear if timestamptz is more than 1 day old
create table purchased_ticket_validations (
    id uuid primary key,
    purchased_ticket_id uuid not null references purchased_tickets(id),
    timestamptz timestamptz not null
);
