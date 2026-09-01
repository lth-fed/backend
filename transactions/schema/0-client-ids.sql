create table client_ids (
    client_id text primary key,

    swish_cert text not null,
    swish_key text not null,
    swish_number text not null,
    swish_payment_fee_fixed money not null,
    swish_payment_fee_fraction double precision not null,
    swish_payment_fee_max money not null,
    swish_refund_fee money not null,

    stripe_secret text,
    -- all stripe callbacks should go to the same URL, so keeping one per client_id is reasonable
    stripe_endpoint_secret text,

    -- for receipts
    name text not null,
    email text not null,
    address text not null,
    organization_number text not null,
    svg_icon text
);
