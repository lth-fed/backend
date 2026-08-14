create table client_ids (
    client_id text primary key,

    swish_cert text not null,
    swish_key text not null,
    swish_number text not null,

    stripe_secret text,
    -- all stripe callbacks should go to the same URL, so keeping one per client_id is reasonable
    stripe_endpoint_secret text,

    -- Fortnox service-account OAuth and bookkeeping settings. Keeping these nullable makes the
    -- integration opt-in per transactions client.
    fortnox_client_id text,
    fortnox_client_secret text,
    fortnox_tenant_id text,
    fortnox_voucher_series text,
    fortnox_bank_account integer check (fortnox_bank_account > 0),

    -- for receipts
    name text not null,
    email text not null,
    address text not null,
    organization_number text not null,
    svg_icon text,

    constraint fortnox_configuration_complete check (
        num_nonnulls(
            fortnox_client_id,
            fortnox_client_secret,
            fortnox_tenant_id,
            fortnox_voucher_series,
            fortnox_bank_account
        ) in (0, 5)
    )
);

-- VAT rates are stored as basis points: 2500 means 25%, 1200 means 12%, and 0 means exempt.
create table fortnox_tax_accounts (
    client_id text not null references client_ids(client_id) on delete cascade,
    vat_basis_points integer not null check (vat_basis_points >= 0),
    revenue_account integer not null check (revenue_account > 0),
    vat_account integer check (vat_account > 0),

    primary key (client_id, vat_basis_points),
    constraint fortnox_vat_account_required check (
        (vat_basis_points = 0 and vat_account is null)
        or (vat_basis_points > 0 and vat_account is not null)
    )
);
create table api_tokens (
    token text primary key,
    client_id text not null references client_ids(client_id),
    callback_url_v1 text not null
);
create type provider as enum (
    'swish',
    'stripe',
    'free'
);
-- for swish all these uuids are to be formatted as uppercase "simple"
-- this datamodel is Swish-first. Stripe will not be used nearly as often and also Swish has a way
-- nicer API. For example, swish uses UUIDs for IDs, while stripe uses text. I want the downstream
-- clients of `transactions` to get UUIDs and not some random text.
create table transactions (
    -- instructionUUID in swish
    id uuid primary key,
    -- used for stripe saving card details
    customer_id text,
    client_id text not null references client_ids(client_id),
    callback_url_v1 text not null,
    created timestamptz not null default now(),
    -- signifies if this is paid (if it's paid we'll have a payment reference!)
    -- stripe: it's the stripe checkout ID
    -- swish: the payment_reference we get
    payment_reference text,
    paid_at timestamptz,
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
    transaction_id uuid not null references transactions(id) on delete cascade,

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

create table transaction_reserved_ids (
    id uuid primary key,
    created timestamptz not null default now()
);

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

-- A durable outbox keeps Fortnox unavailable/retried callbacks from losing bookkeeping work.
-- `manual_review` is intentionally terminal: retrying an ambiguous voucher POST could create a
-- duplicate voucher in Fortnox, which documents no idempotency key for this endpoint.
create table fortnox_voucher_jobs (
    transaction_id uuid primary key references transactions(id),
    state text not null default 'pending'
        check (state in ('pending', 'processing', 'manual_review', 'completed')),
    attempts integer not null default 0 check (attempts >= 0),
    next_attempt_at timestamptz not null default now(),
    started_at timestamptz,
    last_error text,

    voucher_series text,
    voucher_number integer,
    voucher_year integer,
    file_id text,
    completed_at timestamptz,

    constraint fortnox_voucher_identity_complete check (
        num_nonnulls(voucher_series, voucher_number, voucher_year) in (0, 3)
    )
);
create index fortnox_voucher_jobs_pending
    on fortnox_voucher_jobs (next_attempt_at)
    where state = 'pending';
