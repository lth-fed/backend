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
    user_id text not null,
    client_id text not null
);

create table sessions (
    id text primary key,
    redirect_uri text not null,
    client_id text not null,
    state text,
    nonce text,
    callback_url_v1 text,
    -- PKCE
    code_challenge text not null,

    datasharing_confirmed boolean not null default false,
    redirect_requires_datasharing boolean not null,

    created timestamptz not null default now()
);
create table session_validated_users (
    session_id text primary key references sessions(id) on delete cascade,
    sub text not null,
    email text,
    full_name text,
    -- lowercase, single letter or close (e, doct)
    lth_guild text
);

create table saml2_request_id_cache (
    id text primary key,
    created timestamptz not null default now()
);
create table email_token_holding (
    id uuid primary key,
    email text not null,
    code text not null,

    created timestamptz not null default now()
);
