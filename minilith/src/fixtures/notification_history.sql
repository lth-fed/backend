insert into images (id, size, url)
values ('10000000-0000-0000-0000-000000000001', 0, 'https://example.com/logo.png');

insert into users (id, name, language) values
    ('eligible', ''::bytea, ''::bytea),
    ('muted', ''::bytea, ''::bytea),
    ('private-member', ''::bytea, ''::bytea),
    ('no-settings', ''::bytea, ''::bytea),
    ('owner-muted', ''::bytea, ''::bytea),
    ('owner-overridden', ''::bytea, ''::bytea);

insert into groups (id, path, name, description, logo_id, limit_membership_visibility) values
    ('20000000-0000-0000-0000-000000000001', 'tlth', '{}'::jsonb, '{}'::jsonb, '10000000-0000-0000-0000-000000000001', false),
    ('20000000-0000-0000-0000-000000000002', 'tlth.e', '{}'::jsonb, '{}'::jsonb, '10000000-0000-0000-0000-000000000001', false),
    ('20000000-0000-0000-0000-000000000003', 'tlth.e.open', '{}'::jsonb, '{}'::jsonb, '10000000-0000-0000-0000-000000000001', false),
    ('20000000-0000-0000-0000-000000000004', 'tlth.e.private', '{}'::jsonb, '{}'::jsonb, '10000000-0000-0000-0000-000000000001', true);

insert into group_memberships (user_id, group_id) values
    ('eligible', '20000000-0000-0000-0000-000000000003'),
    ('muted', '20000000-0000-0000-0000-000000000003'),
    ('private-member', '20000000-0000-0000-0000-000000000004'),
    ('no-settings', '20000000-0000-0000-0000-000000000003');

insert into user_group_settings (user_id, group_id, visible, notification_level) values
    ('eligible', '20000000-0000-0000-0000-000000000002', true, 'all'),
    ('muted', '20000000-0000-0000-0000-000000000001', true, 'all'),
    ('muted', '20000000-0000-0000-0000-000000000002', true, 'none'),
    ('private-member', '20000000-0000-0000-0000-000000000002', true, 'all'),
    ('owner-muted', '20000000-0000-0000-0000-000000000002', false, 'none'),
    ('owner-overridden', '20000000-0000-0000-0000-000000000002', true, 'all');

insert into activities (
    id, responsible_name, responsible_contact, creator_id,
    title, description, location, time_start, time_end, image_id,
    is_hidden, is_hidden_for_other_admins, max_tickets
) values (
    '50000000-0000-0000-0000-000000000001', 'Responsible', 'mailto:test@example.com',
    '20000000-0000-0000-0000-000000000003', '{}', '{}',
    row(null, null, null, null)::location, now(), now() + interval '1 hour',
    '10000000-0000-0000-0000-000000000001', false, false, 100
);

insert into activity_hosts (activity_id, group_id) values
    ('50000000-0000-0000-0000-000000000001', '20000000-0000-0000-0000-000000000003');

insert into ticket_kinds (
    id, activity_id, name, price,
    purchasing_available_start, purchasing_available_stop,
    max_tickets, min_tickets, reserved_or_purchased_tickets,
    allow_transfer_ticket_start, allow_transfer_ticket_stop,
    has_been_purchased, has_been_released
) values (
    '50000000-0000-0000-0000-000000000002',
    '50000000-0000-0000-0000-000000000001', '{}', 0::money,
    now(), now() + interval '1 hour', 100, 0, 2,
    now(), now() + interval '1 hour', true, true
);

insert into ticket_kind_allowed_groups (ticket_kind_id, group_id) values
    ('50000000-0000-0000-0000-000000000002', '20000000-0000-0000-0000-000000000002');

insert into purchased_tickets (id, ticket_kind_id, purchaser_id, owner_id, transaction_id) values
    ('50000000-0000-0000-0000-000000000003', '50000000-0000-0000-0000-000000000002', 'owner-muted', 'owner-muted', '50000000-0000-0000-0000-000000000005'),
    ('50000000-0000-0000-0000-000000000004', '50000000-0000-0000-0000-000000000002', 'owner-overridden', 'owner-overridden', '50000000-0000-0000-0000-000000000006');

insert into notifications (id, sender, title, content, send_at, sent) values
    ('30000000-0000-0000-0000-000000000001', '{"en":"Group"}', '{"en":"Recent"}', '{"en":"Visible"}', now() - interval '1 day', true),
    ('30000000-0000-0000-0000-000000000002', '{"en":"Group"}', '{"en":"Scheduled"}', '{"en":"Hidden"}', now() + interval '1 day', false),
    ('30000000-0000-0000-0000-000000000003', '{"en":"Group"}', '{"en":"Old"}', '{"en":"Hidden"}', now() - interval '7 months', true),
    ('30000000-0000-0000-0000-000000000004', '{"en":"Group"}', '{"en":"Private"}', '{"en":"Direct members"}', now() - interval '1 hour', true),
    ('30000000-0000-0000-0000-000000000005', '{"en":"Activity"}', '{"en":"Owners"}', '{"en":"Ticket holders"}', now() - interval '30 minutes', true),
    ('30000000-0000-0000-0000-000000000006', '{"en":"Activity"}', '{"en":"Buyers"}', '{"en":"Eligible buyers"}', now() + interval '1 hour', false);

insert into group_notifications (id, group_id, notification_id) values
    ('40000000-0000-0000-0000-000000000001', '20000000-0000-0000-0000-000000000002', '30000000-0000-0000-0000-000000000001'),
    ('40000000-0000-0000-0000-000000000002', '20000000-0000-0000-0000-000000000002', '30000000-0000-0000-0000-000000000002'),
    ('40000000-0000-0000-0000-000000000003', '20000000-0000-0000-0000-000000000002', '30000000-0000-0000-0000-000000000003'),
    ('40000000-0000-0000-0000-000000000004', '20000000-0000-0000-0000-000000000004', '30000000-0000-0000-0000-000000000004');

insert into purchased_ticket_notifications (id, activity_id, notification_id) values
    ('40000000-0000-0000-0000-000000000005', '50000000-0000-0000-0000-000000000001', '30000000-0000-0000-0000-000000000005');

insert into activity_buyers_notifications (id, activity_id, notification_id) values
    ('40000000-0000-0000-0000-000000000006', '50000000-0000-0000-0000-000000000001', '30000000-0000-0000-0000-000000000006');

insert into activity_notification_overrides (user_id, activity_id, follow) values
    ('owner-overridden', '50000000-0000-0000-0000-000000000001', false);
