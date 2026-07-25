create type provider as enum (
    'swish',
    'stripe'
);
-- for swish all these uuids are to be formatted as uppercase "simple"
-- this datamodel is Swish-first. Stripe will not be used nearly as often and also Swish has a way
-- nicer API. For example, swish uses UUIDs for IDs, while stripe uses text. I want the downstream
-- clients of `transactions` to get UUIDs and not some random text.
create table transactions (
    -- instructionUUID in swish
    id uuid primary key,
    -- used for stripe saving card details
    customer_id text not null,
    client_id text not null references client_ids(client_id),
    callback_url_v1 text not null,
    created timestamptz not null default now(),
    -- signifies if this is paid (if it's paid we'll have a payment reference!)
    -- stripe: it's the stripe checkout ID
    -- swish: the payment_reference we get
    payment_reference text,
    timeout timestamptz not null,
    -- for refund
    provider provider not null,

    -- to be added once a transaction is started & incremented once a refund is submitted
    total_transaction_fee money not null,
    total_transaction_fee_currency text not null default 'SEK'
        check (total_transaction_fee_currency = 'SEK'),

    -- our_reference uuid not null, -- this is not necessary as we have the iid
    -- used to verify that the callbacks from e.g. Swish are actually from them
    callback_identifier uuid not null,

    refund_reference text,
    refund_id uuid,
    -- used to verify that the callbacks from e.g. Swish are actually from them
    refund_callback_identifier uuid
);
create index transactions_timeout on transactions using btree (timeout);
create table transaction_wares (
    idx integer not null,
    transaction_id uuid not null references transactions(id),

    name text not null,
    -- incl. tax
    amount money not null,
    currency text not null default 'SEK'
        check (currency = 'SEK'),
    -- e.g. 1.25 for 25% moms in Sweden.
    -- on receipts, round to closet öre
    -- for e.g. E-sektionen this will be 1, since we pay moms when buying stuff, not selling tickets
    tax double precision not null check (tax >= 1.0),

    primary key (idx, transaction_id)
);
create index transaction_wares_transaction_id on transaction_wares using hash (transaction_id);

create table stripe_customers (
    customer_id text primary key,
    stripe_id text not null
);
create table stripe_checkouts (
    -- if transaction is removed, remove this too
    transaction_id uuid primary key references transactions(id) on delete cascade,
    stripe_id text not null
);
create index stripe_checkouts_stripe_id on stripe_checkouts using hash (stripe_id);
