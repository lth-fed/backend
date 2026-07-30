insert into images (id, size, url)
values ('00000000-0000-0000-0000-000000000001', 0, 'https://example.invalid/image');

insert into users (id, name, language, nonce)
values ('email:responsible@example.com', ''::bytea, ''::bytea, ''::bytea);

insert into groups (id, path, name, description, logo_id, limit_membership_visibility)
values (
    '00000000-0000-0000-0000-000000000002',
    'root',
    '{}'::jsonb,
    '{}'::jsonb,
    '00000000-0000-0000-0000-000000000001',
    false
);

insert into activities (
    id, responsible_id, responsible_contact, creator_id,
    title, description, location, time_start, time_end, image_id,
    is_hidden, is_hidden_for_other_admins, max_tickets
) values (
    '00000000-0000-0000-0000-000000000003',
    'email:responsible@example.com',
    'mailto:responsible@example.com',
    '00000000-0000-0000-0000-000000000002',
    '{}'::jsonb,
    '{}'::jsonb,
    row(null, null, null, null)::location,
    now(),
    now() + interval '1 hour',
    '00000000-0000-0000-0000-000000000001',
    false,
    false,
    3
);

insert into ticket_kinds (
    id, activity_id, name, price,
    purchasing_available_start, purchasing_available_stop,
    max_tickets, min_tickets, reserved_or_purchased_tickets,
    allow_transfer_ticket_start, allow_transfer_ticket_stop,
    allow_transfer_ticket_bypass_allowed_groups,
    has_been_purchased, has_been_released
) values
(
    '00000000-0000-0000-0000-000000000004',
    '00000000-0000-0000-0000-000000000003',
    '{}'::jsonb, 0::money, now(), now() + interval '1 hour',
    3, 0, 0, now(), now() + interval '1 hour', false, false, false
),
(
    '00000000-0000-0000-0000-000000000005',
    '00000000-0000-0000-0000-000000000003',
    '{}'::jsonb, 0::money, now(), now() + interval '1 hour',
    3, 0, 0, now(), now() + interval '1 hour', false, false, false
);
