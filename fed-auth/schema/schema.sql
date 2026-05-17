create table auth_refresh_tokens (
    user_id text not null,
    domain text not null,
    refresh_token uuid not null,
    primary key (refresh_token, domain)
);
