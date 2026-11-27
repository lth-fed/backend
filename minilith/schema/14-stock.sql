create table stock_transactions (
    id uuid primary key,
    name jsonb not null,
    group_id uuid not null references groups(id),
    -- auto add items to this activity too when items are taken out (if it's for sale)
    activity_id uuid references activities(id),
    -- purchase / refund if all records are of that
    time timestamptz not null default now()
);

create type stock_record_kind as enum (
    -- us buying from a supplier / returning to the supplier: the physical stock in the store
    'purchase', 'refund',
    -- a consumer buying / returning at the counter: what is still available to sell
    'sales', 'sales-refund',
    -- moving stock between the store and an activity
    'take-out', 'put-back',
    -- losses entered by hand (spillage, a voided bottle)
    'balance',
    -- signed adjustment from a stocktake: positive when more was counted than the books said
    'inventory'
);
create table stock_transaction_records (
    id uuid primary key,
    -- these both MUST have the same non-null group associated
    transaction_id uuid not null references stock_transactions(id),
    product_id uuid not null references products(id),
    amount numeric not null,
    kind stock_record_kind not null
);
