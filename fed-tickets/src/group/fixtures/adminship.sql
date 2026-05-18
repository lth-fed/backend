insert into users (id, name, language, latest_refresh, nonce) values
    ('user_a', ''::bytea, ''::bytea, now(), ''::bytea);

insert into groups (path, name, description, limit_membership_visibility) values
    ('tlth',         '{}'::jsonb, '{}'::jsonb, false),
    ('tlth.e',       '{}'::jsonb, '{}'::jsonb, false),
    ('tlth.e.nolla', '{}'::jsonb, '{}'::jsonb, false);
