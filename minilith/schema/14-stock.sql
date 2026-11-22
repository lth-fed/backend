create table stock_transactions (
    id uuid primary key,
    name jsonb not null,
    group_id uuid not null references groups(id),
    -- auto add items to this activity too when items are taken out (if it's for sale)
    activity_id uuid references activities(id),
    -- purchase / refund if all records are of that
    time timestamptz not null default now()
);

create type stock_record_kind as enum ('purchase', 'refund', 'take-out', 'put-back', 'balance');
create table stock_transaction_records (
    id uuid primary key,
    -- these both MUST have the same non-null group associated
    transaction_id uuid not null references stock_transactions(id),
    product_id uuid not null references products(id),
    amount numeric not null,
    kind stock_record_kind not null
);
