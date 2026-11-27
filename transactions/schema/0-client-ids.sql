create table client_ids (
    client_id text primary key,

    swish_cert text not null,
    swish_key text not null,
    swish_number text not null,
    swish_payment_fee_fixed money not null,
    swish_payment_fee_fraction double precision not null,
    swish_payment_fee_max money not null,
    swish_refund_fee money not null,

    -- SumUp card payments. Both are null when the client has no SumUp merchant.
    sumup_api_key text,
    sumup_merchant_code text,
    -- Fraction of the amount charged as our transaction fee, e.g. 0.015 for 1.5 %.
    sumup_online_fee_fraction double precision not null default 0.0149,
    sumup_physical_fee_fraction double precision not null default 0.0079,

    -- for receipts
    name text not null,
    email text not null,
    address text not null,
    organization_number text not null,
    svg_icon text
);
