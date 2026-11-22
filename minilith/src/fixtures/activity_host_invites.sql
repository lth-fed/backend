insert into images (id, size, url)
values ('10000000-0000-0000-0000-000000000001', 0, 'https://example.invalid/logo.png');

insert into users (id, name, language) values
    ('email:creator@example.com', ''::bytea, ''::bytea),
    ('email:cohost@example.com', ''::bytea, ''::bytea),
    ('email:invited@example.com', ''::bytea, ''::bytea),
    ('email:other@example.com', ''::bytea, ''::bytea),
    ('lund-university:worker', ''::bytea, ''::bytea);

insert into groups (id, path, name, description, logo_id) values
    ('10000000-0000-0000-0000-000000000002', 'creator', '{"en":"Creator"}', '{}'::jsonb, '10000000-0000-0000-0000-000000000001'),
    ('10000000-0000-0000-0000-000000000003', 'cohost', '{"en":"Cohost"}', '{}'::jsonb, '10000000-0000-0000-0000-000000000001'),
    ('10000000-0000-0000-0000-000000000004', 'invited', '{"en":"Invited"}', '{}'::jsonb, '10000000-0000-0000-0000-000000000001'),
    ('10000000-0000-0000-0000-000000000005', 'other', '{"en":"Other"}', '{}'::jsonb, '10000000-0000-0000-0000-000000000001');

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

insert into sales_targets (id, group_id, email, iban) values
    ('10000000-0000-0000-0000-000000000007',
     '10000000-0000-0000-0000-000000000002', 'seed@invalid', '');

insert into product_tags (id, group_id, hidden, name)
select '10000000-0000-0000-0000-000000000009'::uuid, target.group_id, false, '{"en":"Ticket"}'::jsonb
from sales_targets target where target.id = '10000000-0000-0000-0000-000000000007'::uuid
and not exists (select 1 from product_tags where group_id = target.group_id);

insert into products (
    id, group_id, kind, name, description, sales_price, sales_organization,
    sales_stock, stock_real, stock_unit, stock_price, bookkeeping_tag, max_uses,
    sales_start, sales_stop, resale_start, resale_stop
)
select
    '10000000-0000-0000-0000-000000000008'::uuid, target.group_id, 'activity_visibility', '{}'::jsonb, '{}'::jsonb,
    0::money, target.id, 0, 0, 'st', 0,
    (select id from product_tags where group_id = target.group_id order by id limit 1),
    1, now(), now() + interval '1 hour', now() + interval '1 hour', now() + interval '1 hour'
from sales_targets target where target.id = '10000000-0000-0000-0000-000000000007'::uuid;

insert into activities (
    id, responsible_name, responsible_contact, sales_target_id,
    title, description, location, time_start, time_end, image_id,
    is_hidden, is_hidden_for_other_admins, max_tickets_per_user, base_ticket
) values (
    '10000000-0000-0000-0000-000000000006',
    'Responsible',
    'mailto:responsible@example.com',
    '10000000-0000-0000-0000-000000000007',
    '{"en":"Activity"}'::jsonb,
    '{}'::jsonb,
    row(null, null, null, null)::location,
    now(),
    now() + interval '1 hour',
    '10000000-0000-0000-0000-000000000001',
    false,
    false,
    100,
    '10000000-0000-0000-0000-000000000008'
);

insert into activity_hosts (activity_id, group_id) values
    ('10000000-0000-0000-0000-000000000006', '10000000-0000-0000-0000-000000000002'),
    ('10000000-0000-0000-0000-000000000006', '10000000-0000-0000-0000-000000000003');
