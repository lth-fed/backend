insert into users (id, name, language, latest_refresh, nonce) values
    ('user_a', ''::bytea, ''::bytea, now(), ''::bytea),
    ('user_b', ''::bytea, ''::bytea, now(), ''::bytea),
    ('user_c', ''::bytea, ''::bytea, now(), ''::bytea);

insert into groups (path, name, description, limit_membership_visibility) values
    ('tlth',             '{}'::jsonb, '{}'::jsonb, false),
    ('tlth.e',           '{}'::jsonb, '{}'::jsonb, false),
    ('tlth.e.styrelsen', '{}'::jsonb, '{}'::jsonb, false),
    ('tlth.e.nolla',     '{}'::jsonb, '{}'::jsonb, true),
    ('tlth.d',           '{}'::jsonb, '{}'::jsonb, false),
    ('tlth.d.styrelsen', '{}'::jsonb, '{}'::jsonb, false),
    ('tlth.d.nolla',     '{}'::jsonb, '{}'::jsonb, true),
    ('tlth.f',           '{}'::jsonb, '{}'::jsonb, false),
    ('tlth.f.styrelsen', '{}'::jsonb, '{}'::jsonb, false),
    ('tlth.f.nolla',     '{}'::jsonb, '{}'::jsonb, true);

insert into group_memberships (user_id, group_id) values
    -- guild member, no privileged subgroups
    ('user_a', (select id from groups where path = 'tlth.e')),
    -- direct nolla membership only
    ('user_b', (select id from groups where path = 'tlth.d.nolla')),
    -- overlapping memberships at the federation root and a guild
    ('user_c', (select id from groups where path = 'tlth')),
    ('user_c', (select id from groups where path = 'tlth.f'));
