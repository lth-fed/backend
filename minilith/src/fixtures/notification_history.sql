insert into images (id, size, url)
values ('10000000-0000-0000-0000-000000000001', 0, 'https://example.com/logo.png');

insert into users (id, name, language) values
    ('eligible', ''::bytea, ''::bytea),
    ('muted', ''::bytea, ''::bytea),
    ('private-member', ''::bytea, ''::bytea),
    ('no-settings', ''::bytea, ''::bytea),
    ('owner-muted', ''::bytea, ''::bytea),
    ('owner-overridden', ''::bytea, ''::bytea);

insert into groups (id, path, name, description, logo_id) values
    ('20000000-0000-0000-0000-000000000001', 'tlth', '{}'::jsonb, '{}'::jsonb, '10000000-0000-0000-0000-000000000001'),
    ('20000000-0000-0000-0000-000000000002', 'tlth.e', '{}'::jsonb, '{}'::jsonb, '10000000-0000-0000-0000-000000000001'),
    ('20000000-0000-0000-0000-000000000003', 'tlth.e.open', '{}'::jsonb, '{}'::jsonb, '10000000-0000-0000-0000-000000000001'),
    ('20000000-0000-0000-0000-000000000004', 'tlth.e.private', '{}'::jsonb, '{}'::jsonb, '10000000-0000-0000-0000-000000000001');

insert into group_memberships (user_id, group_id) values
    ('eligible', '20000000-0000-0000-0000-000000000003'),
    ('muted', '20000000-0000-0000-0000-000000000003'),
    ('private-member', '20000000-0000-0000-0000-000000000004'),
    ('no-settings', '20000000-0000-0000-0000-000000000003');

update group_memberships set specific = true
where group_id = '20000000-0000-0000-0000-000000000004';

insert into sales_targets (id, group_id, email, iban) values
    ('20000000-0000-0000-0000-000000000005', '20000000-0000-0000-0000-000000000002', 'seed@invalid', '');

insert into product_tags (id, group_id, hidden, name) values
    ('20000000-0000-0000-0000-000000000006', '20000000-0000-0000-0000-000000000002', false, '{"en":"Ticket"}'::jsonb);

insert into product_tags (id, group_id, hidden, name)
select '20000000-0000-0000-0000-000000000006'::uuid, target.group_id, false, '{"en":"Ticket"}'::jsonb
from sales_targets target where target.id = '20000000-0000-0000-0000-000000000005'::uuid
and not exists (select 1 from product_tags where group_id = target.group_id);

insert into products (
    id, group_id, kind, name, description, sales_price, sales_organization,
    sales_stock, stock_real, stock_unit, stock_price, bookkeeping_tag, max_uses,
    sales_start, sales_stop, resale_start, resale_stop
)
select
    '50000000-0000-0000-0000-000000000007'::uuid, target.group_id, 'activity_visibility', '{}'::jsonb, '{}'::jsonb,
    0::money, target.id, 0, 0, 'st', 0,
    (select id from product_tags where group_id = target.group_id order by id limit 1),
    1, now(), now() + interval '1 hour', now() + interval '1 hour', now() + interval '1 hour'
from sales_targets target where target.id = '20000000-0000-0000-0000-000000000005'::uuid;

insert into activities (
    id, responsible_name, responsible_contact, sales_target_id,
    title, description, location, time_start, time_end, image_id,
    is_hidden, is_hidden_for_other_admins, max_tickets_per_user, base_ticket
) values (
    '50000000-0000-0000-0000-000000000001', 'Responsible', 'mailto:test@example.com',
    '20000000-0000-0000-0000-000000000005', '{}', '{}',
    row(null, null, null, null)::location, now(), now() + interval '1 hour',
    '10000000-0000-0000-0000-000000000001', false, false, 100,
    '50000000-0000-0000-0000-000000000007'
);

insert into activity_hosts (activity_id, group_id) values
    ('50000000-0000-0000-0000-000000000001', '20000000-0000-0000-0000-000000000003');

insert into products (
    id, group_id, kind, name, description, sales_price, sales_organization,
    sales_stock, stock_real, stock_unit, stock_price, bookkeeping_tag, max_uses,
    sales_start, sales_stop, resale_start, resale_stop, been_purchased
) values (
    '50000000-0000-0000-0000-000000000002', '20000000-0000-0000-0000-000000000002',
    'ticket', '{}', '{}', 0::money, '20000000-0000-0000-0000-000000000005',
    100, 100, 'st', 0, '20000000-0000-0000-0000-000000000006', 1,
    now(), now() + interval '1 hour', now(), now() + interval '1 hour', true
);

insert into product_activity_assignments (activity_id, product_id) values
    ('50000000-0000-0000-0000-000000000001', '50000000-0000-0000-0000-000000000007'),
    ('50000000-0000-0000-0000-000000000001', '50000000-0000-0000-0000-000000000002');

insert into product_allowed_groups (product_id, group_id) values
    ('50000000-0000-0000-0000-000000000002', '20000000-0000-0000-0000-000000000002');

insert into purchases (id, product_id, product_price, paid_price, activity_id, purchaser_id, owner_id, purchased_at) values
    ('50000000-0000-0000-0000-000000000003', '50000000-0000-0000-0000-000000000002',
     0::money, 0::money,
     '50000000-0000-0000-0000-000000000001', 'owner-muted', 'owner-muted', now()),
    ('50000000-0000-0000-0000-000000000004', '50000000-0000-0000-0000-000000000002',
     0::money, 0::money,
     '50000000-0000-0000-0000-000000000001', 'owner-overridden', 'owner-overridden', now());

insert into purchase_transactions (purchase_id, seller_id, buyer_id, completed_at, transaction_id) values
    ('50000000-0000-0000-0000-000000000003', null, 'owner-muted', now(), '50000000-0000-0000-0000-000000000005'),
    ('50000000-0000-0000-0000-000000000004', null, 'owner-overridden', now(), '50000000-0000-0000-0000-000000000006');

insert into push_notifications (id, kind, sender, title, content, send_at, sent) values
    ('30000000-0000-0000-0000-000000000001', 'news', '{"en":"Group"}', '{"en":"Recent"}', '{"en":"Visible"}', now() - interval '1 day', true),
    ('30000000-0000-0000-0000-000000000002', 'news', '{"en":"Group"}', '{"en":"Scheduled"}', '{"en":"Hidden"}', now() + interval '1 day', false),
    ('30000000-0000-0000-0000-000000000003', 'news', '{"en":"Group"}', '{"en":"Old"}', '{"en":"Hidden"}', now() - interval '7 months', true),
    ('30000000-0000-0000-0000-000000000004', 'news', '{"en":"Group"}', '{"en":"Private"}', '{"en":"Direct members"}', now() - interval '1 hour', true),
    ('30000000-0000-0000-0000-000000000005', 'activity', '{"en":"Activity"}', '{"en":"Owners"}', '{"en":"Ticket holders"}', now() - interval '30 minutes', true),
    ('30000000-0000-0000-0000-000000000006', 'activity', '{"en":"Activity"}', '{"en":"Buyers"}', '{"en":"Eligible buyers"}', now() + interval '1 hour', false);

insert into news (id, target_group_id, title, content, published_at) values
    ('40000000-0000-0000-0000-000000000001', '20000000-0000-0000-0000-000000000002', '{"en":"Recent"}', '{"en":"Visible"}', now() - interval '1 day'),
    ('40000000-0000-0000-0000-000000000002', '20000000-0000-0000-0000-000000000002', '{"en":"Scheduled"}', '{"en":"Hidden"}', now() + interval '1 day'),
    ('40000000-0000-0000-0000-000000000003', '20000000-0000-0000-0000-000000000002', '{"en":"Old"}', '{"en":"Hidden"}', now() - interval '7 months'),
    ('40000000-0000-0000-0000-000000000004', '20000000-0000-0000-0000-000000000004', '{"en":"Private"}', '{"en":"Direct members"}', now() - interval '1 hour');

insert into news_push_notifications (id, news_id, notification_id) values
    ('40000000-0000-0000-0000-000000000001', '40000000-0000-0000-0000-000000000001', '30000000-0000-0000-0000-000000000001'),
    ('40000000-0000-0000-0000-000000000002', '40000000-0000-0000-0000-000000000002', '30000000-0000-0000-0000-000000000002'),
    ('40000000-0000-0000-0000-000000000003', '40000000-0000-0000-0000-000000000003', '30000000-0000-0000-0000-000000000003'),
    ('40000000-0000-0000-0000-000000000004', '40000000-0000-0000-0000-000000000004', '30000000-0000-0000-0000-000000000004');

insert into purchased_ticket_push_notifications (id, activity_id, notification_id) values
    ('40000000-0000-0000-0000-000000000005', '50000000-0000-0000-0000-000000000001', '30000000-0000-0000-0000-000000000005'),
    ('40000000-0000-0000-0000-000000000006', '50000000-0000-0000-0000-000000000001', '30000000-0000-0000-0000-000000000006');

insert into activity_followers (user_id, activity_id, follow) values
    ('owner-overridden', '50000000-0000-0000-0000-000000000001', false);
