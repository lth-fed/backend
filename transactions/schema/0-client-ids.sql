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
