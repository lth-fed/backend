-- this file is ordered according to how a user interacts with the system:)

create table ticket_kinds (
    id uuid primary key default uuidv4(),
    activity_id uuid not null references activities(id),
    -- addons ticket_addons[] not null,
    -- allowed_grous set<ticket_kind_allowed_groups> not null,
    --
    name jsonb not null,
    -- in ören
    price money not null check (price >= 0::money),
    purchasing_available_start timestamptz not null,
    purchasing_available_stop timestamptz not null,
    max_tickets integer not null default 2147483647 check (max_tickets >= 0),
    min_tickets integer not null check (min_tickets >= 0), -- default MAX_INT
    check (
        (min_tickets = 0 and max_tickets = 0)
        or min_tickets < max_tickets
    ),
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
    has_been_purchased boolean not null,
    has_been_released boolean not null
);
-- which groups are allowed to buy this ticket kind
create table ticket_kind_allowed_groups (
    ticket_kind_id uuid not null references ticket_kinds(id),
    group_id uuid not null references groups(id),
    primary key (group_id, ticket_kind_id)
);
-- these are examples for how this table can be used:
-- matpref: options tom, has_text_field = true, required= false
-- dryckespaket: options: ["alkohol", "alkoholfritt", "inget"], has_text_field = false, required = true, multiple_alternatives = false
-- matpref val: options ["vego", "vegan", "nötter"], has_text_field = true, required=false, multiple_alternatives=true
create table ticket_addons (
    id uuid primary key,
    ticket_kind_id uuid not null references ticket_kinds(id),
    -- for sorting
    idx integer not null,
    name jsonb not null,
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

-- see the bottom of this document below for the triggers which make sure one reference is valid
create table users_in_purchase_flow (
    user_id text primary key references users(id),
    ticket_kind_id uuid not null references ticket_kinds(id),
    -- denormalized since the "child" tables also have this, but it's very convenient and enforced by DB
    unique (user_id, ticket_kind_id),
    -- at least one of these need to be valid
    release_queue text,
    reservation_queue text,
    reservation text,
    constraint purchase_flow_references_own_user check (
        (release_queue is null or release_queue = user_id)
        and (reservation_queue is null or reservation_queue = user_id)
        and (reservation is null or reservation = user_id)
    ),
    constraint purchase_flow_has_at_most_one_state check (
        num_nonnulls(release_queue, reservation_queue, reservation) <= 1
    )
);

-- people who have started queuing to buy a ticket
-- only applicable at biljettsläpp
-- once the ticket is released for purchase, convert all the queuers to `ticket_queue` with random placements
-- then start a worker which pops the user with the highest placement as long as there are tickets available
-- but for efficiency, don't put the N people with highest placement in `ticket_queue`, just move all of them to
-- `ticket_reservations` directly & initiate transactions
create table ticket_release_queuers (
    user_id text primary key references users(id),
    -- a biljettsläpp is only per ticket_kind, not per activity
    ticket_kind_id uuid not null references ticket_kinds(id),
    -- remove after 20 minutes, should refresh after 15 minutes
    started_queueing timestamptz not null,
    foreign key (user_id, ticket_kind_id)
        references users_in_purchase_flow(user_id, ticket_kind_id)
);
-- there should be a worker that every second or so checks if there are any available tickets,
-- then pop the user with the best placement & converts it into a reservation & decrements available
-- tickets (transaction)
--
-- If there are no reservations & no available tickets, clear this table and (notify users?) and stop worker
-- Upon server startup, check if there are any people in this queue, for every ticket_kind, and if there is,
--     start the worker.
-- The worker should start after the biljettsläpp if there were more people interested than there was tickets
create table ticket_reservation_placement_tails (
    ticket_kind_id uuid primary key references ticket_kinds(id),
    placement_tail integer not null
);
create table ticket_reservation_queuers (
    user_id text primary key references users(id),
    ticket_kind_id uuid not null references ticket_kinds(id),
    -- this value is relative placement
    placement integer not null check (placement > 0),
    unique (ticket_kind_id, placement),
    foreign key (user_id, ticket_kind_id)
        references users_in_purchase_flow(user_id, ticket_kind_id)
);

-- people who have reserved a ticket
create table ticket_reservations (
    id uuid primary key default uuidv4(),
    user_id text not null unique references users(id),
    ticket_kind_id uuid not null references ticket_kinds(id),
    -- could be null before transaction is initiated
    transaction_id uuid,
    -- remove after this!
    -- or if transaction is currently happening and not cancellable wait for max an hour or smth
    timeout timestamptz not null,
    foreign key (user_id, ticket_kind_id)
        references users_in_purchase_flow(user_id, ticket_kind_id)
);
-- the addons for the reserved ticket
create table ticket_reservation_addons (
    addon_id uuid not null references ticket_addons(id),
    ticket_id uuid not null references ticket_reservations(id) on delete cascade,
    ---
    selected_options integer[] not null,
    selected_text text not null,
    primary key (ticket_id, addon_id)
);

create table purchased_tickets (
    id uuid primary key default uuidv4(),
    ticket_kind_id uuid not null references ticket_kinds(id),
    -- if these are not the same the ticket is clearly transferred
    purchaser_id text not null references users(id),
    owner_id text not null references users(id),
    -- could have price here, since it's not allowed to be changed once a ticket has been purchased, but it's just unnecessary because it's easy to calculate
    transaction_id uuid not null
);
create table purchased_ticket_addons (
    addon_id uuid not null references ticket_addons(id),
    ticket_id uuid not null references purchased_tickets(id),
    ---
    selected_options integer[] not null,
    selected_text text not null,
    primary key (ticket_id, addon_id)
);
-- clear if timestamptz is more than 1 day old
create table purchased_ticket_validations (
    id uuid primary key,
    purchased_ticket_id uuid not null references purchased_tickets(id),
    timestamp timestamptz not null default now()
);

create function ensure_purchase_flow_has_state() returns trigger
language plpgsql as $$
declare
    checked_user text;
begin
    if tg_op = 'DELETE' then
        checked_user := old.user_id;
    else
        checked_user := new.user_id;
    end if;
    if exists (
        select 1 from users_in_purchase_flow flow
        where flow.user_id = checked_user
        and (
            num_nonnulls(release_queue, reservation_queue, reservation) != 1
            or (release_queue is not null) != exists (
                select 1 from ticket_release_queuers
                where user_id = flow.user_id and ticket_kind_id = flow.ticket_kind_id
            )
            or (reservation_queue is not null) != exists (
                select 1 from ticket_reservation_queuers
                where user_id = flow.user_id and ticket_kind_id = flow.ticket_kind_id
            )
            or (reservation is not null) != exists (
                select 1 from ticket_reservations
                where user_id = flow.user_id and ticket_kind_id = flow.ticket_kind_id
            )
        )
    ) then
        raise exception 'purchase flow for user % has inconsistent state', checked_user;
    end if;
    return null;
end;
$$;

-- Transitions briefly have zero or two child rows, so consistency is checked
-- at transaction commit rather than after each statement.
create constraint trigger purchase_flow_has_state
after insert or update on users_in_purchase_flow
deferrable initially deferred
for each row execute function ensure_purchase_flow_has_state();

create constraint trigger release_queuer_has_matching_flow
after insert or update or delete on ticket_release_queuers
deferrable initially deferred
for each row execute function ensure_purchase_flow_has_state();

create constraint trigger reservation_queuer_has_matching_flow
after insert or update or delete on ticket_reservation_queuers
deferrable initially deferred
for each row execute function ensure_purchase_flow_has_state();

create constraint trigger reservation_has_matching_flow
after insert or update or delete on ticket_reservations
deferrable initially deferred
for each row execute function ensure_purchase_flow_has_state();
