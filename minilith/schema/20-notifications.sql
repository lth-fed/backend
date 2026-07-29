create type push_platform as enum ('ios', 'android');

create table push_devices (
    device_id uuid primary key,
    user_id text not null references users (id) on delete cascade,
    push_token text not null,
    platform push_platform not null,
    updated_at timestamptz not null default now(),
    unique (platform, push_token)
);

create table notifications (
    id uuid primary key,
    title jsonb not null,
    content jsonb not null,
    send_at timestamptz not null
);
create index notifications_send_time on notifications using btree (send_at);

-- specific tables for specific kind of notifications, so we can at send time look up which users should get them
create table ticket_kind_notifications (
    -- just so we can query it, example: "release" for ticket release
    -- then we can insert & edit using the ticket_kind_id & "release"
    -- and it's still possible to send additional messages
    id text default 'release',
    ticket_kind_id uuid references ticket_kinds (id),
    primary key (id, ticket_kind_id),
    notification_id uuid not null references notifications (id) on delete cascade
);
