insert into images (id, size, url)
values ('00000000-0000-0000-0000-000000000001', 0, 'https://example.invalid/image');

insert into users (id, name, language)
values
    ('test:purchase-flow-1', ''::bytea, ''::bytea),
    ('test:purchase-flow-2', ''::bytea, ''::bytea),
    ('test:purchase-flow-3', ''::bytea, ''::bytea);

insert into groups (id, path, name, description, logo_id)
values (
    '00000000-0000-0000-0000-000000000002',
    'root',
    '{}'::jsonb,
    '{}'::jsonb,
    '00000000-0000-0000-0000-000000000001'
);

insert into sales_targets (id, group_id, email, iban) values (
    '00000000-0000-0000-0000-000000000006',
    '00000000-0000-0000-0000-000000000002',
    'seed@invalid',
    ''
);

insert into product_tags (id, group_id, hidden, name) values (
    '00000000-0000-0000-0000-000000000007',
    '00000000-0000-0000-0000-000000000002',
    false,
    '{"en":"Ticket"}'::jsonb
);

insert into product_tags (id, group_id, hidden, name)
select '00000000-0000-0000-0000-000000000007'::uuid, target.group_id, false, '{"en":"Ticket"}'::jsonb
from sales_targets target where target.id = '00000000-0000-0000-0000-000000000006'::uuid
and not exists (select 1 from product_tags where group_id = target.group_id);

insert into products (
    id, group_id, kind, name, description, sales_price, sales_organization,
    sales_stock, stock_real, stock_unit, stock_price, bookkeeping_tag, max_uses,
    sales_start, sales_stop, resale_start, resale_stop
)
select
    '00000000-0000-0000-0000-000000000008'::uuid, target.group_id, 'activity_visibility', '{}'::jsonb, '{}'::jsonb,
    0::money, target.id, 0, 0, 'st', 0,
    (select id from product_tags where group_id = target.group_id order by id limit 1),
    1, now(), now() + interval '1 hour', now() + interval '1 hour', now() + interval '1 hour'
from sales_targets target where target.id = '00000000-0000-0000-0000-000000000006'::uuid;

insert into activities (
    id, responsible_name, responsible_contact, sales_target_id,
    title, description, location, time_start, time_end, image_id,
    is_hidden, is_hidden_for_other_admins, max_tickets_per_user, base_ticket
) values (
    '00000000-0000-0000-0000-000000000003',
    'Responsible Person',
    'mailto:responsible@example.com',
    '00000000-0000-0000-0000-000000000006',
    '{}'::jsonb,
    '{}'::jsonb,
    row(null, null, null, null)::location,
    now(),
    now() + interval '1 hour',
    '00000000-0000-0000-0000-000000000001',
    false,
    false,
    3,
    '00000000-0000-0000-0000-000000000008'
);

insert into products (
    id, group_id, kind, name, description, sales_price, sales_organization,
    sales_stock, stock_real, stock_unit, stock_price, bookkeeping_tag, max_uses,
    sales_start, sales_stop, resale_start, resale_stop
) values
(
    '00000000-0000-0000-0000-000000000004',
    '00000000-0000-0000-0000-000000000002', 'ticket', '{}'::jsonb, '{}'::jsonb,
    0::money, '00000000-0000-0000-0000-000000000006', 3, 3, 'st', 0,
    '00000000-0000-0000-0000-000000000007', 1,
    now(), now() + interval '1 hour', now(), now() + interval '1 hour'
),
(
    '00000000-0000-0000-0000-000000000005',
    '00000000-0000-0000-0000-000000000002', 'ticket', '{}'::jsonb, '{}'::jsonb,
    0::money, '00000000-0000-0000-0000-000000000006', 3, 3, 'st', 0,
    '00000000-0000-0000-0000-000000000007', 1,
    now(), now() + interval '1 hour', now(), now() + interval '1 hour'
);

insert into product_activity_assignments (activity_id, product_id) values
    ('00000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000008'),
    ('00000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000004'),
    ('00000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000005');
