create table ticket_addon_options (
    id uuid primary key,
    ticket_addon_id uuid not null references ticket_addons(id),
    idx integer not null,
    --
    name jsonb not null,
    price bigint not null check (price >= 0),
    -- for the books, if e.g. 20SEK went to spirits, 15SEK to wine etc.
    -- add price here & category below. The index of the item maps it to it's price / category

    -- this is not optimal but composite types & sum constraints seemed very sus
    bookkeeping_prices bigint[] not null,
    bookkeeping_price_categories text[] not null,
    constraint bookkeeping_prices_add_up check (SUM(bookkeeping_prices) = price),
    constraint bookkeeping_lengths_consistent check (LEN(bookkeeping_prices) = LEN(bookkeeping_price_categories))
);

create table ticket_addon (
    id uuid primary key,
    ticket_kind_id uuid not null references ticket_kinds(id),
    -- text array
    multiple_alternatives boolean not null,
    has_text_field boolean not null,
    required boolean not null
);
-- matpref: options tom, has_text_field = true, required= false
-- dryckespaket: options: ["alkohol", "alkoholfritt", "inget"], has_text_field = false, required = true, multiple_alternatives = false
-- matpref val: options ["vego", "vegan", "nötter"], has_text_field = true, required=false, multiple_alternatives=true

create table ticket_kinds (
    id uuid primary key,
    activity_id uuid not null references activities(id),
    --
    name jsonb not null,
    -- in ören
    price bigint not null check (price >= 0),
    purchasing_available tsrange not null,
    max_tickets integer not null check (max_tickets > 0), -- default MAX_INT
    min_tickets integer not null check (min_tickets > 0), -- default MAX_INT
    purchased_tickets integer not null check (purchased_tickets >= 0),
    reserved_tickets integer not null check (reserved_tickets >= 0),
    -- when this is set nothing is allowed to be changed, except the bookkeeping on table:options & purchasing_available
    has_been_purchased boolean not null
);
-- which groups are allowed to buy this ticket kind
create table ticket_kind_allowed_groups (
    id uuid primary key,
    ticket_kind_id uuid not null references ticket_kinds(id),
    group_id uuid not null references groups(id)
);

create table purchased_ticket_addons (
    id uuid primary key,
    addon_id uuid not null references ticket_addons(id),
    ticket_id uuid not null references purchased_ticket(id),
    ---
    selected_options integer[] not null,
    selected_text text not null
);
-- clear if timestamp is more than 1 day old
create table purchased_ticket_validations (
    id uuid primary key,
    purchased_ticket_id uuid not null references purchased_ticket(id),
    timestamp timestamp not null
);
-- TODO: krav 12
-- fram tills 2 dagar före etc.
-- ONE PERSON CAN ONLY HAVE ONE TICKET KIND PER ACTIVITY
create table purchased_ticket (
    id uuid primary key,
    ticket_kind_id uuid not null references ticket_kinds(id),
    -- if these are not the same the ticket is clearly transferred
    purchaser_id uuid not null references users(id),
    owner_id uuid not null references users(id)
    -- could have price here, since it's not allowed to be changed once a ticket has been purchased, but it's just unnecessary because it's easy to calculate
);

-- people who have started queuing to buy a ticket
create table ticket_queuers (
    id uuid primary key,
    -- a biljettsläpp is only per ticket_kind ~~() is shared among all ticket kinds, so get a list of all unique user ids for all of these where ticket_id->activity_id is the activity in question~~
    ticket_id uuid not null references ticket_kinds(id),
    user_id uuid not null references users(id),
    -- remove after 20 minutes, should refresh after 15 minutes
    started_queueing timestamp not null,
    constraint not_queuing_for_multiple_ticket_types unique (ticket_id, user_id)
);

-- TODO: store who got accepted by random
-- TODO: store list & rank of people who didn't get tickets
-- worker process, when to start and exit?

-- people who have reserved a ticket
create table ticket_reservations (
    id uuid primary key,
    ticket_id uuid not null references ticket_kinds(id),
    user_id uuid not null references users(id),
    -- remove after this!
    -- or if transaction is currently happening and not cancellable wait for max an hour or smth
    timeout timestamp not null
);
