insert into images (id, size, url)
values ('10000000-0000-0000-0000-000000000001', 0, 'https://example.com/logo.png');

insert into users (id, name, language) values
    ('eligible', ''::bytea, ''::bytea),
    ('muted', ''::bytea, ''::bytea),
    ('private-member', ''::bytea, ''::bytea);

insert into groups (id, path, name, description, logo_id, limit_membership_visibility) values
    ('20000000-0000-0000-0000-000000000001', 'tlth', '{}'::jsonb, '{}'::jsonb, '10000000-0000-0000-0000-000000000001', false),
    ('20000000-0000-0000-0000-000000000002', 'tlth.e', '{}'::jsonb, '{}'::jsonb, '10000000-0000-0000-0000-000000000001', false),
    ('20000000-0000-0000-0000-000000000003', 'tlth.e.open', '{}'::jsonb, '{}'::jsonb, '10000000-0000-0000-0000-000000000001', false),
    ('20000000-0000-0000-0000-000000000004', 'tlth.e.private', '{}'::jsonb, '{}'::jsonb, '10000000-0000-0000-0000-000000000001', true);

insert into group_memberships (user_id, group_id) values
    ('eligible', '20000000-0000-0000-0000-000000000003'),
    ('muted', '20000000-0000-0000-0000-000000000003'),
    ('private-member', '20000000-0000-0000-0000-000000000004');

insert into user_group_settings (user_id, group_id, visible, notification_level) values
    ('eligible', '20000000-0000-0000-0000-000000000002', true, 'all'),
    ('muted', '20000000-0000-0000-0000-000000000002', true, 'none'),
    ('private-member', '20000000-0000-0000-0000-000000000002', true, 'all');

insert into notifications (id, title, content, send_at, sent) values
    ('30000000-0000-0000-0000-000000000001', '{"en":"Recent"}', '{"en":"Visible"}', now() - interval '1 day', true),
    ('30000000-0000-0000-0000-000000000002', '{"en":"Scheduled"}', '{"en":"Hidden"}', now() + interval '1 day', false),
    ('30000000-0000-0000-0000-000000000003', '{"en":"Old"}', '{"en":"Hidden"}', now() - interval '7 months', true),
    ('30000000-0000-0000-0000-000000000004', '{"en":"Private"}', '{"en":"Direct members"}', now() - interval '1 hour', true);

insert into group_notifications (id, group_id, notification_id) values
    ('40000000-0000-0000-0000-000000000001', '20000000-0000-0000-0000-000000000002', '30000000-0000-0000-0000-000000000001'),
    ('40000000-0000-0000-0000-000000000002', '20000000-0000-0000-0000-000000000002', '30000000-0000-0000-0000-000000000002'),
    ('40000000-0000-0000-0000-000000000003', '20000000-0000-0000-0000-000000000002', '30000000-0000-0000-0000-000000000003'),
    ('40000000-0000-0000-0000-000000000004', '20000000-0000-0000-0000-000000000004', '30000000-0000-0000-0000-000000000004');
