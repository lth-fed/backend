insert into images (id, size, url) values ('e2a92dcc-06cf-4d47-9865-d33f59d0261f'::uuid, 0, 'https://icelk.dev/logo.png');

insert into users (id, name, language, latest_refresh, nonce) values
    ('email:user_a', ''::bytea, ''::bytea, now(), ''::bytea);

insert into groups (path, name, description, logo_id, limit_membership_visibility) values
    ('tlth',         '{}'::jsonb, '{}'::jsonb, 'e2a92dcc-06cf-4d47-9865-d33f59d0261f'::uuid, false),
    ('tlth.e',       '{}'::jsonb, '{}'::jsonb, 'e2a92dcc-06cf-4d47-9865-d33f59d0261f'::uuid, false),
    ('tlth.e.nolla', '{}'::jsonb, '{}'::jsonb, 'e2a92dcc-06cf-4d47-9865-d33f59d0261f'::uuid, false);
