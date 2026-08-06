insert into images (id, size, url)
values ('10000000-0000-0000-0000-000000000001', 0, 'https://example.invalid/logo.png');

insert into users (id, name, language) values
    ('email:creator@example.com', ''::bytea, ''::bytea),
    ('email:cohost@example.com', ''::bytea, ''::bytea),
    ('email:invited@example.com', ''::bytea, ''::bytea),
    ('email:other@example.com', ''::bytea, ''::bytea),
    ('lund-university:worker', ''::bytea, ''::bytea);

insert into groups (id, path, name, description, logo_id, limit_membership_visibility) values
    ('10000000-0000-0000-0000-000000000002', 'creator', '{"en":"Creator"}', '{}'::jsonb, '10000000-0000-0000-0000-000000000001', false),
    ('10000000-0000-0000-0000-000000000003', 'cohost', '{"en":"Cohost"}', '{}'::jsonb, '10000000-0000-0000-0000-000000000001', false),
    ('10000000-0000-0000-0000-000000000004', 'invited', '{"en":"Invited"}', '{}'::jsonb, '10000000-0000-0000-0000-000000000001', false),
    ('10000000-0000-0000-0000-000000000005', 'other', '{"en":"Other"}', '{}'::jsonb, '10000000-0000-0000-0000-000000000001', false);

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
    id, responsible_name, responsible_contact, creator_id,
    title, description, location, time_start, time_end, image_id,
    is_hidden, is_hidden_for_other_admins, max_tickets
) values (
    '10000000-0000-0000-0000-000000000006',
    'Responsible',
    'mailto:responsible@example.com',
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
    ('10000000-0000-0000-0000-000000000006', '10000000-0000-0000-0000-000000000002'),
    ('10000000-0000-0000-0000-000000000006', '10000000-0000-0000-0000-000000000003');
