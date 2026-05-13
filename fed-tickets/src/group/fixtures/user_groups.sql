insert into users (id, name, language, latest_refresh) values
    ('user_a', ''::bytea, ''::bytea, now()),
    ('user_b', ''::bytea, ''::bytea, now()),
    ('user_c', ''::bytea, ''::bytea, now());

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

insert into group_memberships (user_id, group_path) values
    -- guild member, no privileged subgroups
    ('user_a', 'tlth.e'),
    -- direct nolla membership only
    ('user_b', 'tlth.d.nolla'),
    -- overlapping memberships at the federation root and a guild
    ('user_c', 'tlth'),
    ('user_c', 'tlth.f');
