create table auth_refresh_tokens (
    refresh_token uuid not null,
    client_id text not null,
    user_id text not null,
    nonce text,
    auth_time timestamptz not null,
    primary key (refresh_token, client_id)
);

create table api_keys (
    key uuid primary key,
    user_id text not null
);
