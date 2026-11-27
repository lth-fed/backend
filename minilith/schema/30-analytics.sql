create table activity_views (
    id uuid primary key,
    activity_id uuid not null references activities(id) on delete cascade,
    user_id text not null unique references users(id) on delete cascade,
    count integer not null
);

create table notification_stats (
    notification_id uuid primary key references push_notifications(id) on delete cascade,
    n_opened integer not null,
    n_dismissed integer not null,
    n_sent integer not null
);
-- every interaction with a notification (opened / dismissed / sent)
create type notification_event_kind as enum ('sent', 'opened', 'dismissed');
create table notification_events (
    id uuid primary key default uuidv4(),
    notification_id uuid not null references push_notifications(id) on delete cascade,
    user_id text not null references users(id),
    kind notification_event_kind not null,
    -- e.g. activity id or news id the notification pointed to
    target_id uuid,
    time timestamptz not null default now()
);
