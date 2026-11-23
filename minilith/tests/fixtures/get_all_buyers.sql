insert into images (id, size, url)
values (
    '10000000-0000-0000-0000-000000000001',
    0,
    'https://example.invalid/logo.png'
);

insert into users (id, name, language) values
    ('email:creator@example.com', ''::bytea, ''::bytea),
    ('email:cohost@example.com', ''::bytea, ''::bytea),
    ('email:invited@example.com', ''::bytea, ''::bytea),
    ('email:other@example.com', ''::bytea, ''::bytea),
    ('lund-university:worker', ''::bytea, ''::bytea);

insert into groups (
    id,
    path,
    name,
    description,
    logo_id,
    limit_membership_visibility
) values
    (
        '10000000-0000-0000-0000-000000000002',
        'creator',
        '{"en":"Creator"}',
        '{}'::jsonb,
        '10000000-0000-0000-0000-000000000001',
        false
    ),
    (
        '10000000-0000-0000-0000-000000000003',
        'cohost',
        '{"en":"Cohost"}',
        '{}'::jsonb,
        '10000000-0000-0000-0000-000000000001',
        false
    ),
    (
        '10000000-0000-0000-0000-000000000004',
        'invited',
        '{"en":"Invited"}',
        '{}'::jsonb,
        '10000000-0000-0000-0000-000000000001',
        false
    ),
    (
        '10000000-0000-0000-0000-000000000005',
        'other',
        '{"en":"Other"}',
        '{}'::jsonb,
        '10000000-0000-0000-0000-000000000001',
        false
    );

insert into group_memberships (user_id, group_id) values
    ('email:creator@example.com', '10000000-0000-0000-0000-000000000002'),
    ('email:cohost@example.com', '10000000-0000-0000-0000-000000000003'),
    ('email:invited@example.com', '10000000-0000-0000-0000-000000000004'),
    ('email:other@example.com', '10000000-0000-0000-0000-000000000005');

insert into group_adminships (user_id, group_id) values
    ('email:creator@example.com', '10000000-0000-0000-0000-000000000002'),
    ('email:cohost@example.com', '10000000-0000-0000-0000-000000000003'),
    ('email:invited@example.com', '10000000-0000-0000-0000-000000000004'),
    ('email:other@example.com', '10000000-0000-0000-0000-000000000005');

insert into activities (
    id,
    responsible_name,
    responsible_contact,
    creator_id,
    title,
    description,
    location,
    time_start,
    time_end,
    image_id,
    is_hidden,
    is_hidden_for_other_admins,
    max_tickets
) values (
    '10000000-0000-0000-0000-000000000006',
    'Responsible',
    'mailto:responsible@example.com',

    -- activities.creator_id references groups.id
    '10000000-0000-0000-0000-000000000002',

    '{"en":"Activity"}'::jsonb,
    '{}'::jsonb,
    row(null, null, null, null)::location,
    now(),
    now() + interval '1 hour',
    '10000000-0000-0000-0000-000000000001',
    false,
    false,
    100
);

insert into activity_hosts (activity_id, group_id) values
    (
        '10000000-0000-0000-0000-000000000006',
        '10000000-0000-0000-0000-000000000002'
    ),
    (
        '10000000-0000-0000-0000-000000000006',
        '10000000-0000-0000-0000-000000000003'
    );

insert into ticket_kinds (
    id,
    activity_id,
    name,
    price,
    purchasing_available_start,
    purchasing_available_stop,
    max_tickets,
    min_tickets,
    reserved_or_purchased_tickets,
    allow_transfer_ticket_start,
    allow_transfer_ticket_stop,
    has_been_purchased,
    has_been_released
) values (
    '10000000-0000-0000-0000-000000000007',
    '10000000-0000-0000-0000-000000000006',
    '{"en":"Ticket"}'::jsonb,
    100::money,
    now() - interval '1 hour',
    now() + interval '1 hour',
    100,
    0,
    2,
    now() - interval '1 hour',
    now() + interval '1 hour',
    true,
    false
);

insert into activity_verifiers (activity_id, user_id)
values (
    '10000000-0000-0000-0000-000000000006',
    'email:cohost@example.com'
);

insert into purchased_tickets (
    id,
    ticket_kind_id,
    purchaser_id,
    owner_id,
    transaction_id
) values
(
    '10000000-0000-0000-0000-000000000008',
    '10000000-0000-0000-0000-000000000007',
    'email:invited@example.com',
    'email:invited@example.com',
    '10000000-0000-0000-0000-000000000009'
),
(
    '10000000-0000-0000-0000-00000000000a',
    '10000000-0000-0000-0000-000000000007',
    'email:other@example.com',
    'email:invited@example.com',
    '10000000-0000-0000-0000-00000000000b'
);
insert into ticket_addons (
    id,
    ticket_kind_id,
    idx,
    name,
    multiple_alternatives,
    has_text_field,
    required
) values (
    '10000000-0000-0000-0000-00000000000c',
    '10000000-0000-0000-0000-000000000007',
    0,
    '{"en":"Lunch"}'::jsonb,
    false,
    true,
    true
);

insert into ticket_addon_options (
    id,
    ticket_addon_id,
    idx,
    name,
    price,
    bookkeeping_prices,
    bookkeeping_price_categories
) values
(
    '10000000-0000-0000-0000-00000000000d',
    '10000000-0000-0000-0000-00000000000c',
    0,
    '{"en":"Vegetarian"}'::jsonb,
    20::money,
    ARRAY[20::money],
    ARRAY['other']::text[]
),
(
    '10000000-0000-0000-0000-00000000000e',
    '10000000-0000-0000-0000-00000000000c',
    1,
    '{"en":"Meat"}'::jsonb,
    30::money,
    ARRAY[30::money],
    ARRAY['other']::text[]
);

insert into purchased_ticket_addons (
    ticket_id,
    addon_id,
    selected_options,
    selected_text
) values
(
    '10000000-0000-0000-0000-000000000008',
    '10000000-0000-0000-0000-00000000000c',
    ARRAY[0]::integer[],
    'No onions'
),
(
    '10000000-0000-0000-0000-00000000000a',
    '10000000-0000-0000-0000-00000000000c',
    ARRAY[1]::integer[],
    'Extra hungry'
);
