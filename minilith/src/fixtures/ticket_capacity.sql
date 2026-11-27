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
    id, group_id, kind, name, description, sales_organization,
    sales_stock, stock_real, stock_unit, stock_price, max_uses,
    sales_start, sales_stop, resale_start, resale_stop
)
select
    '00000000-0000-0000-0000-000000000008'::uuid, target.group_id, 'activity_visibility', '{}'::jsonb, '{}'::jsonb,
    target.id, 0, 0, 'st', 0,
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
    id, group_id, kind, name, description, sales_organization,
    sales_stock, stock_real, stock_unit, stock_price, max_uses,
    sales_start, sales_stop, resale_start, resale_stop
) values
(
    '00000000-0000-0000-0000-000000000004',
    '00000000-0000-0000-0000-000000000002', 'ticket', '{}'::jsonb, '{}'::jsonb,
    '00000000-0000-0000-0000-000000000006', 3, 3, 'st', 0, 1,
    now(), now() + interval '1 hour', now(), now() + interval '1 hour'
),
(
    '00000000-0000-0000-0000-000000000005',
    '00000000-0000-0000-0000-000000000002', 'ticket', '{}'::jsonb, '{}'::jsonb,
    '00000000-0000-0000-0000-000000000006', 3, 3, 'st', 0, 1,
    now(), now() + interval '1 hour', now(), now() + interval '1 hour'
);

insert into product_activity_assignments (activity_id, product_id) values
    ('00000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000008'),
    ('00000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000004'),
    ('00000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000005');


-- Every product's price is its base-price add-on (see `product_base_prices`).
insert into product_addon_sets (id, product_id) select id, id from products;
insert into product_addons (id, set_id, idx, name, multiple_alternatives, has_text_field, required, is_base_price)
select uuidv4(), products.id, -1, '{"en":"Price","sv":"Pris"}'::jsonb, false, false, true, true
from products;
insert into product_addon_links (product_id, addon_id)
select products.id, addon.id from products
inner join product_addons addon on addon.set_id = products.id and addon.is_base_price;
insert into product_addon_options (id, product_addon_id, idx, name, price)
select uuidv4(), addon.id, 0, products.name, 0::money
from products
inner join product_addons addon on addon.set_id = products.id and addon.is_base_price;
insert into product_addon_option_bookkeepings (id, option_id, price, category)
select uuidv4(), options.id, options.price,
    (select tag.id from product_tags tag where tag.group_id = products.group_id order by tag.id limit 1)
from products
inner join product_addons addon on addon.set_id = products.id and addon.is_base_price
inner join product_addon_options options on options.product_addon_id = addon.id;
