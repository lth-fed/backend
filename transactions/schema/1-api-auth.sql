create table api_tokens (
    token text primary key,
    client_id text not null references client_ids(client_id),
    callback_url_v1 text not null
);
