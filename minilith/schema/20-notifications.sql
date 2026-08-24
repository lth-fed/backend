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
    send_at timestamptz not null,
    sent boolean not null default false
);
create index notifications_unsent_send_time on notifications using btree (send_at)
where sent = false;

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

create table group_notifications (
    id uuid primary key,
    group_id uuid not null references groups (id),
    notification_id uuid not null references notifications (id) on delete cascade
);

-- Add each new notification kind's allowed groups to this first CTE. The view's stable
-- (notification_id, user_id) contract keeps delivery and history independent of notification kind.
create view notification_recipients as
with notification_allowed_groups as (
    select distinct
        ticket_kind_notifications.notification_id,
        allowed_group.id,
        allowed_group.path
    from ticket_kind_notifications
    inner join ticket_kind_allowed_groups
        on ticket_kind_allowed_groups.ticket_kind_id
            = ticket_kind_notifications.ticket_kind_id
    inner join groups allowed_group
        on allowed_group.id = ticket_kind_allowed_groups.group_id

    union

    select distinct
        group_notifications.notification_id,
        groups.id,
        groups.path
    from group_notifications
    inner join groups on groups.id = group_notifications.group_id
),
visible_users as (
    select distinct
        notification_allowed_groups.notification_id,
        group_memberships.user_id,
        notification_allowed_groups.id as allowed_group_id,
        notification_allowed_groups.path as allowed_group_path
    from notification_allowed_groups
    inner join groups as member_group
        on notification_allowed_groups.path @> member_group.path
    inner join group_memberships
        on group_memberships.group_id = member_group.id
    where (
        member_group.limit_membership_visibility = false
        or member_group.id = notification_allowed_groups.id
    )
),
ranked_settings as (
    select
        visible_users.notification_id,
        visible_users.user_id,
        visible_users.allowed_group_id,
        user_group_settings.notification_level,
        row_number() over (
            partition by
                visible_users.notification_id,
                visible_users.user_id,
                visible_users.allowed_group_id
            order by nlevel(settings_group.path) desc
        ) as precedence
    from visible_users
    inner join user_group_settings
        on user_group_settings.user_id = visible_users.user_id
    inner join groups as settings_group
        on settings_group.id = user_group_settings.group_id
        and settings_group.path @> visible_users.allowed_group_path
)
select distinct notification_id, user_id
from ranked_settings
where precedence = 1
and notification_level in (
    'all'::notification_level,
    'personalized'::notification_level
);
