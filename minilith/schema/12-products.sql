-- this file is ordered according to how a user interacts with the system:)

create type product_kind as enum ('ticket', 'normal', 'questionnaire', 'activity_visibility');

create table product_tags (
    id uuid primary key,
    group_id uuid not null references groups(id),
    -- hide instead of deleting for bookkeeping legacy
    hidden bool not null default false,
    name jsonb not null
);
create table products (
    id uuid primary key default uuidv4(),
    group_id uuid not null references groups(id),
    image_id uuid references images(id),
    -- immutable!
    -- don't show questionnaire in store
    -- ticket can be showed in store (for the activity) for e.g. sales in door
    kind product_kind not null,

    -- addons ticket_addons[] not null,
    -- allowed_grous set<ticket_kind_allowed_groups> not null,

    name jsonb not null,
    description jsonb not null,
    -- NOTE: gone. The product's price is its base-price add-on (see `product_base_prices`).
    -- immutable!
    sales_organization uuid not null references sales_targets(id),
    -- what is calculated based on checkouts
    -- we need this as a lock so we don't make too many
    -- https://github.com/lth-fed/backend/pull/7#discussion_r3145792223
    -- null when item isn't managed automatically (e.g. manually: shots or not at all: the spirits used for shots & drinks)
    sales_stock integer check (sales_stock is null or sales_stock >= 0),
    -- what is manually entered using stock transactions.
    stock_real integer not null,
    stock_unit text not null,
    stock_price numeric(18, 6) not null,
    -- NOTE: no bookkeeping tag here; every price (the base price is an add-on too) is split over
    -- categories in `product_addon_option_bookkeepings`.
    max_uses integer not null default 1 check (max_uses > 0),

    sales_start timestamptz not null,
    sales_stop timestamptz not null,
    -- to disable, make the range empty. Also applies to transfers
    resale_start timestamptz not null,
    resale_stop timestamptz not null,
    been_purchased boolean not null default false,

    -- for products available to be sold without an activity; e.g. sandwitches in a café
    -- if false, this product can only be sold at a certain activity and doesn't show up
    -- when linking a sales device to a group
    store_general_sales boolean not null default false,
    -- Available in the app to all users who has access to this group
    store_online_self_serve boolean not null default false,
    -- Can be used to keep store items unique. For alcohol, I strongly recommend forcing the admin
    -- to input the Systembolaget ID here, to avoid creating multiple products.
    store_internal_id text,
    -- stored as parts per million
    store_alcohol_by_volume integer check (store_alcohol_by_volume is null or store_alcohol_by_volume >= 0),

    tickets_been_released boolean not null default false,
    -- When looking this up, keep in mind it's:
    -- `!tickets_guaranteed ? 0 : stock_effective +
    --   count(*) from product_guaranteed_buyers where product_id = this`
    tickets_guaranteed bool not null default false,
    -- This product can only be bought if the user owns a ticket product to this activity
    -- ONLY makes sense for products which are specific to an activity and isn't sold otherwise
    -- (i.e. other products or sitting specific products)
    tickets_require_other bool not null default false
);
create table product_addon_sets (
    -- same as the owner's id (the `activities.id` or `products.id` below)
    id uuid primary key,
    -- exactly one of these owns the set (uniqueness is enforced by indexes in 40-indexes.sql)
    activity_id uuid references activities(id) on delete cascade,
    -- item products (and questionnaires) own their addons through this instead
    product_id uuid references products(id) on delete cascade,
    constraint product_addon_sets_single_owner check (activity_id is null or product_id is null)
);
-- these are examples for how this table can be used:
-- matpref: options tom, has_text_field = true, required= false
-- dryckespaket: options: ["alkohol", "alkoholfritt", "inget"], has_text_field = false, required = true, multiple_alternatives = false
-- matpref val: options ["vego", "vegan", "nötter"], has_text_field = true, required=false, multiple_alternatives=true
create table product_addons (
    id uuid primary key,
    set_id uuid not null references product_addon_sets(id) on delete cascade,
    -- for sorting
    idx integer not null,
    name jsonb not null,
    -- <virtual foreign key> product_addon_options[] not null,
    --
    multiple_alternatives boolean not null,
    has_text_field boolean not null,
    required boolean not null,
    -- the mandatory, customer-invisible add-on holding the product's own price
    -- (one per set, sets are per product, so one per product)
    is_base_price boolean not null default false
);
-- which addons a product offers
create table product_addon_links (
    product_id uuid not null references products(id) on delete cascade,
    addon_id uuid not null references product_addons(id) on delete cascade,
    primary key (product_id, addon_id)
);
-- this is basically an array
create table product_addon_options (
    id uuid primary key,
    product_addon_id uuid not null references product_addons(id) on delete cascade,
    idx integer not null,
    --
    name jsonb not null,
    -- ONLY base price, for the actual sales price check `purchased_product_addons->selected_options_prices`
    price money not null check (price >= 0::money)
    -- for the books, if e.g. 20SEK went to spirits, 15SEK to wine etc.
    -- add price here & category below. The index of the item maps it to it's price / category
);
-- these HAVE to add up to `product_addon_options->price
-- and are distributed over `purchased_product_addons->selected_options_prices`
create table product_addon_option_bookkeepings (
    id uuid primary key,
    option_id uuid not null references product_addon_options(id) on delete cascade,
    price money not null,
    category uuid not null references product_tags(id)
);
create table product_assigned_tags (
    tag_id uuid not null references product_tags(id),
    product_id uuid not null references products(id),
    primary key (tag_id, product_id)
);
-- add products (including tickets) to an activity!
create table product_activity_assignments (
    -- the activity_id->creator_id MUST be the same as product_id->group_id
    activity_id uuid not null references activities(id),
    product_id uuid not null references products(id),
    primary key (activity_id, product_id)
);
-- which groups are allowed to buy this product
create table product_allowed_groups (
    product_id uuid not null references products(id),
    group_id uuid not null references groups(id),
    primary key (group_id, product_id)
);
-- which users are guaranteed one copy of this product OR specifically invited to buy
-- (most relevant for tickets)
create table product_buyers (
    product_id uuid not null references products(id),
    activity_id uuid not null references activities(id),
    user_id text not null references users(id),
    guaranteed bool not null,
    primary key (activity_id, product_id, user_id),
    foreign key (activity_id, product_id) references product_activity_assignments(activity_id, product_id)
);
create index product_buyers_product on product_buyers using hash (product_id);

create table physical_payment_terminals (
    id text primary key,
    group_id uuid not null references groups(id),
    name text not null
);
create table physical_sales_clients (
    -- random uuid generated by the client, shown as a QR code
    id uuid primary key,
    name text not null,
    group_id uuid references groups(id),
    activity_id uuid references activities(id),
    until timestamptz not null,
    constraint has_reference check ((group_id is null) != (activity_id is null))
);

-- A counter/physical sale started on a card reader. The header is kept after completion (for
-- status polling and auditing); the item lines are deleted once they become purchases.
create table physical_sales (
    -- transactions service id, generated by minilith before the provider call
    transaction_id uuid primary key,
    -- null for a regular purchase paid on a reader (no device involved)
    sales_device_id uuid references physical_sales_clients(id),
    buyer_id text not null references users(id),
    total money not null,
    created_at timestamptz not null default now()
);
create table physical_sale_items (
    -- doubles as the purchase id when the payment completes
    id uuid primary key default uuidv4(),
    transaction_id uuid not null references physical_sales(transaction_id) on delete cascade,
    product_id uuid not null references products(id),
    product_price money not null,
    activity_id uuid references activities(id)
);
create index physical_sale_items_transaction on physical_sale_items using hash (transaction_id);
create table physical_sale_item_addons (
    item_id uuid not null references physical_sale_items(id) on delete cascade,
    addon_id uuid not null references product_addons(id),
    selected_options integer[] not null,
    selected_options_prices money[] not null,
    selected_text text not null,
    primary key (item_id, addon_id),
    constraint sale_addon_lengths_consistent check (
        cardinality(selected_options) = cardinality(selected_options_prices)
    )
);

create table purchases (
    id uuid primary key default uuidv4(),
    product_id uuid not null references products(id),
    -- what the current owner paid for the ticket + addons: the addon prices when first bought,
    -- the resale price when resold. A resale may not be listed for more, and we can
    -- never refund more than this (the provider would reject refunding more than was captured).
    paid_price money not null,
    -- if purchased for an activity
    activity_id uuid references activities(id),
    -- if these are not the same the ticket is clearly transferred
    -- todo: remove purchaser_id
    purchaser_id text not null references users(id),
    -- not a FK to users: historical refunds moved the owner to the sentinel `refunded:`, which
    -- is no longer written (refunds only set `refunded_id`).
    owner_id text not null,
    purchased_at timestamptz not null,

    -- this whole item is refunded; not to be confused with transactions API's refunded,
    -- which is simply that a transaction is refunded, which happens for example when
    -- tickets are resold (the old owner is refunded).
    --
    -- the purchase is refunded exactly when this is not null. Only an admin refund sets it
    -- (ownership is left unchanged); resales never set it, nor do refunds of an older
    -- transaction that was part of a resale.
    -- when this is true, don't count the ticket price as income,
    -- only count first purchase_transactions & this id (which refers to transactions id)
    refunded_id uuid,
    -- MUST be <= original price (product_price + sum addons->selected_options_prices)
    -- non-null if for resale
    for_resale money,
    -- true if owner signals they will not use product (i.e. ticket)
    no_show_hint bool not null default false,
    seller_payout_completed bool not null default false,
    -- not null when payout process has been started
    seller_payout_id uuid,
    -- pending resale: the buyer's payment must be completed before ownership moves,
    -- so the callback needs this mapping. Unique = one resale at a time.
    resale_transaction_id uuid unique,
    resale_buyer_id text references users(id)
);
-- there MUST be at least one of these per purchase
-- customer can get receipt (new field) if and only if a row exists with
-- buyer_id = user.id & transaction_id != null
create table purchase_transactions (
    purchase_id uuid not null references purchases(id),
    seller_id text references users(id),
    -- `internal:anonymous-physical-sales` for physical sales without owner ID in request
    buyer_id text not null references users(id),
    completed_at timestamptz not null,
    -- null for transfers
    -- for resale: this is the new buy transaction, the old refunded transaction exists
    -- further back in the history.
    transaction_id uuid,
    constraint first_transaction_exists check (not ((seller_id is null) and (transaction_id is null)))
);
-- COUNT(*) from purchase_uses < purchase_id->product_id->max_uses
create table purchase_uses (
    id uuid primary key,
    purchase_id uuid not null references purchases(id),
    timestamp timestamptz not null default now()
);
create table purchase_use_validations (
    id uuid primary key,
    use_id uuid not null references purchase_uses(id),
    timestamp timestamptz not null default now()
);
create table purchased_product_addons (
    addon_id uuid not null references product_addons(id),
    purchase_id uuid not null references purchases(id),
    ---
    selected_options integer[] not null,
    selected_options_prices money[] not null,
    constraint lengths_consistent check (cardinality(selected_options) = cardinality(selected_options_prices)),
    selected_text text not null,
    primary key (purchase_id, addon_id)
);

-- see the bottom of this document below for the triggers which make sure one reference is valid
create table users_in_purchase_flow (
    user_id text primary key references users(id),
    product_id uuid not null references products(id),
    -- denormalized since the "child" tables also have this, but it's very convenient and enforced by DB
    unique (user_id, product_id),
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
    ),

    lock_id uuid,
    locked_at timestamptz,
    constraint lock_matches check ((lock_id is null) = (locked_at is null))
);

-- People queuing for a ticket kind before it is released (`products.sales_start`). One queue per
-- ticket kind, because every kind has its own stock. At the release (`product/release.rs`) the
-- queuers are shuffled, everybody that fits gets a reservation, and the rest are pushed down into
-- the reservation queue of the activity's base ticket (`original_product_id` keeps the kind they
-- wanted). Queuers expire after 20 minutes.
create table product_release_queuers (
    user_id text primary key references users(id),
    -- a biljettsläpp is per product kind, not per activity
    product_id uuid not null references products(id),
    started_queueing timestamptz not null,
    foreign key (user_id, product_id)
        references users_in_purchase_flow(user_id, product_id)
);

-- Queue positions are only kept for the base ticket of an activity (`activities.base_ticket`), so
-- only that product gets a row here. It is the next placement to hand out; `queue()` and the
-- misplaced-queuer recovery append to it.
create table product_reservation_placement_tails (
    product_id uuid primary key references products(id) on delete cascade,
    placement_tail integer not null
);

-- People waiting for a reservation, i.e. for somebody to cancel or time out a payment. Only the
-- activity's base ticket (`kind = 'ticket'`) has a reservation queue: `product_id` is always that
-- base ticket, and `original_product_id` is the kind they actually want (null = the base ticket).
-- Positions belong to the one queue of the activity, the lowest `placement` is served first; a
-- unit of their original kind that frees up goes to the first in line that wants it, a base ticket
-- unit to the first in line.
create table product_reservation_queuers (
    user_id text primary key references users(id),
    product_id uuid not null references products(id),
    original_product_id uuid references products(id),
    placement integer not null check (placement > 0),
    unique (product_id, placement),
    foreign key (user_id, product_id)
        references users_in_purchase_flow(user_id, product_id)
);

-- people who have reserved a product
create table product_reservations (
    id uuid primary key default uuidv4(),
    user_id text not null unique references users(id),
    product_id uuid not null references products(id),
    product_price money not null,
    -- could be null before transaction is initiated
    transaction_id uuid unique,
    -- remove after this!
    -- or if transaction is currently happening and not cancellable wait for max an hour or smth
    timeout timestamptz not null,
    foreign key (user_id, product_id)
        references users_in_purchase_flow(user_id, product_id)
);
-- the addons for the reserved ticket
create table product_reservation_addons (
    addon_id uuid not null references product_addons(id),
    reservation_id uuid not null references product_reservations(id) on delete cascade,
    ---
    selected_options integer[] not null,
    selected_options_prices money[] not null,
    constraint lengths_consistent check (cardinality(selected_options) = cardinality(selected_options_prices)),
    selected_text text not null,
    primary key (reservation_id, addon_id)
);

-- recipients may be members of these groups or any of their descendants
create table product_transfer_groups (
    product_id uuid not null references products(id) on delete cascade,
    group_id uuid not null references groups(id),
    primary key (group_id, product_id)
);

------------
-- Nerd stuff
------------

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
                select 1 from product_release_queuers
                where user_id = flow.user_id and product_id = flow.product_id
            )
            or (reservation_queue is not null) != exists (
                select 1 from product_reservation_queuers
                where user_id = flow.user_id and product_id = flow.product_id
            )
            or (reservation is not null) != exists (
                select 1 from product_reservations
                where user_id = flow.user_id and product_id = flow.product_id
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
after insert or update or delete on product_release_queuers
deferrable initially deferred
for each row execute function ensure_purchase_flow_has_state();

create constraint trigger reservation_queuer_has_matching_flow
after insert or update or delete on product_reservation_queuers
deferrable initially deferred
for each row execute function ensure_purchase_flow_has_state();

create constraint trigger reservation_has_matching_flow
after insert or update or delete on product_reservations
deferrable initially deferred
for each row execute function ensure_purchase_flow_has_state();

-- The base price of a product, as an easily joined table.
create view product_base_prices as
select
    addon.set_id as product_id,
    option.id as option_id,
    option.idx as option_idx,
    option.name as name,
    option.price as price
from product_addons addon
inner join product_addon_options option on option.product_addon_id = addon.id
where addon.is_base_price;

-- The price range a product can be sold for: its base price plus the most expensive option of
-- every add-on a customer may pick (the activity's set and the product's own set).
create view product_price_ranges as
select
    product.id as product_id,
    base.price as price_min,
    base.price + coalesce((
        select sum((select max(option.price)
            from product_addon_options option
            where option.product_addon_id = addon.id))
        from product_addon_links link
        inner join product_addons addon
            on addon.id = link.addon_id and not addon.is_base_price
        where link.product_id = product.id
        and addon.set_id in (
            product.id,
            (select assignment.activity_id from product_activity_assignments assignment
                where assignment.product_id = product.id
                order by assignment.activity_id limit 1)
        )
    ), '0'::money) as price_max
from products product
inner join product_base_prices base on base.product_id = product.id;
