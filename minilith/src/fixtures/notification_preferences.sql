-- On top of `notification_history`: activity news to everyone who can see activity ...01 (members of
-- tlth.e and below may buy its ticket), its tags, and a tagged news post.
insert into users (id, name, language) values
    ('tags-off', ''::bytea, ''::bytea),
    ('tags-off-follows', ''::bytea, ''::bytea),
    ('important-off', ''::bytea, ''::bytea);

insert into group_memberships (user_id, group_id) values
    ('tags-off', '20000000-0000-0000-0000-000000000003'),
    ('tags-off-follows', '20000000-0000-0000-0000-000000000003'),
    ('important-off', '20000000-0000-0000-0000-000000000003');

insert into activity_tags (id, creator_id, title) values
    ('60000000-0000-0000-0000-000000000001', '20000000-0000-0000-0000-000000000002', '{"en":"Important"}'),
    ('60000000-0000-0000-0000-000000000002', '20000000-0000-0000-0000-000000000002', '{"en":"Party"}');

insert into activity_assigned_tags (activity_id, tag_id) values
    ('50000000-0000-0000-0000-000000000001', '60000000-0000-0000-0000-000000000001'),
    ('50000000-0000-0000-0000-000000000001', '60000000-0000-0000-0000-000000000002');

insert into activity_opt_out_tags (user_id, tag_id) values
    ('tags-off', '60000000-0000-0000-0000-000000000001'),
    ('tags-off', '60000000-0000-0000-0000-000000000002'),
    ('tags-off-follows', '60000000-0000-0000-0000-000000000001'),
    ('tags-off-follows', '60000000-0000-0000-0000-000000000002'),
    ('important-off', '60000000-0000-0000-0000-000000000001');

insert into activity_followers (user_id, activity_id, follow) values
    ('muted', '50000000-0000-0000-0000-000000000001', false),
    ('tags-off-follows', '50000000-0000-0000-0000-000000000001', true);

insert into push_notifications (id, kind, sender, title, content, send_at, sent) values
    ('30000000-0000-0000-0000-000000000007', 'activity', '{"en":"Activity"}', '{"en":"Everyone"}', '{"en":"Who can see it"}', now() - interval '10 minutes', true),
    ('30000000-0000-0000-0000-000000000008', 'news', '{"en":"Group"}', '{"en":"Tagged"}', '{"en":"News"}', now() - interval '5 minutes', true);

insert into activity_push_notifications (id, activity_id, notification_id) values
    ('40000000-0000-0000-0000-000000000007', '50000000-0000-0000-0000-000000000001', '30000000-0000-0000-0000-000000000007');

insert into news (id, target_group_id, title, content, published_at) values
    ('40000000-0000-0000-0000-000000000008', '20000000-0000-0000-0000-000000000002', '{"en":"Tagged"}', '{"en":"News"}', now() - interval '5 minutes');

insert into news_tags (id, creator_id, title) values
    ('70000000-0000-0000-0000-000000000001', '20000000-0000-0000-0000-000000000002', '{"en":"Sports"}');

insert into news_assigned_tags (news_id, tag_id) values
    ('40000000-0000-0000-0000-000000000008', '70000000-0000-0000-0000-000000000001');

insert into news_push_opt_out_tags (user_id, tag_id) values
    ('eligible', '70000000-0000-0000-0000-000000000001');

insert into news_push_notifications (id, news_id, notification_id) values
    ('40000000-0000-0000-0000-000000000009', '40000000-0000-0000-0000-000000000008', '30000000-0000-0000-0000-000000000008');

-- Accounts older than every notification here, so nothing starts out read.
update users set creation = now() - interval '1 year';
