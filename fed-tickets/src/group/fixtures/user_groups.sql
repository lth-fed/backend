insert into images (id, size, url) values ('e2a92dcc-06cf-4d47-9865-d33f59d0261f'::uuid, 0, 'https://icelk.dev/logo.png');

insert into users (id, name, language, latest_refresh, nonce) values
    ('user_a', ''::bytea, ''::bytea, now(), ''::bytea),
    ('user_b', ''::bytea, ''::bytea, now(), ''::bytea),
    ('user_c', ''::bytea, ''::bytea, now(), ''::bytea);

insert into groups (path, name, description, logo_id, limit_membership_visibility) values
    ('tlth',             '{}'::jsonb, '{}'::jsonb, 'e2a92dcc-06cf-4d47-9865-d33f59d0261f'::uuid, false),
    ('tlth.e',           '{}'::jsonb, '{}'::jsonb, 'e2a92dcc-06cf-4d47-9865-d33f59d0261f'::uuid, false),
    ('tlth.e.styrelsen', '{}'::jsonb, '{}'::jsonb, 'e2a92dcc-06cf-4d47-9865-d33f59d0261f'::uuid, false),
    ('tlth.e.nolla',     '{}'::jsonb, '{}'::jsonb, 'e2a92dcc-06cf-4d47-9865-d33f59d0261f'::uuid, true),
    ('tlth.d',           '{}'::jsonb, '{}'::jsonb, 'e2a92dcc-06cf-4d47-9865-d33f59d0261f'::uuid, false),
    ('tlth.d.styrelsen', '{}'::jsonb, '{}'::jsonb, 'e2a92dcc-06cf-4d47-9865-d33f59d0261f'::uuid, false),
    ('tlth.d.nolla',     '{}'::jsonb, '{}'::jsonb, 'e2a92dcc-06cf-4d47-9865-d33f59d0261f'::uuid, true),
    ('tlth.f',           '{}'::jsonb, '{}'::jsonb, 'e2a92dcc-06cf-4d47-9865-d33f59d0261f'::uuid, false),
    ('tlth.f.styrelsen', '{}'::jsonb, '{}'::jsonb, 'e2a92dcc-06cf-4d47-9865-d33f59d0261f'::uuid, false),
    ('tlth.f.nolla',     '{}'::jsonb, '{}'::jsonb, 'e2a92dcc-06cf-4d47-9865-d33f59d0261f'::uuid, true);

insert into group_memberships (user_id, group_id) values
    -- guild member, no privileged subgroups
    ('user_a', (select id from groups where path = 'tlth.e')),
    -- direct nolla membership only
    ('user_b', (select id from groups where path = 'tlth.d.nolla')),
    -- overlapping memberships at the federation root and a guild
    ('user_c', (select id from groups where path = 'tlth')),
    ('user_c', (select id from groups where path = 'tlth.f'));
