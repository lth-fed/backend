create type push_platform as enum ('ios', 'android');

create table push_devices (
    device_id text primary key,
    user_id text not null references users (id) on delete cascade,
    push_token text not null,
    platform push_platform not null,
    updated_at timestamptz not null default now(),
    unique (platform, push_token)
);

create table notifications (
    id uuid primary key,
    sender jsonb not null,
    title jsonb not null,
    content jsonb not null,
    send_at timestamptz not null,
    sent boolean not null default false
);
-- specific tables for specific kind of notifications, so we can at send time look up which users should get them
create table activity_notifications (
    id uuid not null,
    activity_id uuid not null references activities (id),
    primary key (id, activity_id),
    notification_id uuid not null references notifications (id) on delete cascade
);
create table activity_buyers_notifications (
    -- Uuid::nil() for the automatically created ticket-release notification.
    id uuid not null,
    activity_id uuid not null references activities (id),
    primary key (id, activity_id),
    notification_id uuid not null references notifications (id) on delete cascade
);
create table purchased_ticket_notifications (
    id uuid not null,
    activity_id uuid not null references activities (id),
    primary key (id, activity_id),
    notification_id uuid not null references notifications (id) on delete cascade
);

create table group_notifications (
    id uuid primary key,
    group_id uuid not null references groups (id),
    notification_id uuid not null references notifications (id) on delete cascade
);

create table activity_notification_overrides (
    user_id text not null references users(id),
    activity_id uuid not null references activities(id) on delete cascade,
    primary key(user_id, activity_id),
    follow boolean not null
);

-- Keep eligibility separate from notification preferences. Activity eligibility follows ticket
-- visibility, purchasing access, or ownership, while preferences follow the activity hosts.
create view notification_recipients as
with eligible_users as (
    -- Everyone who can see the activity.
    select
        activity_notifications.notification_id,
        group_memberships.user_id,
        activity_notifications.activity_id,
        null::uuid as group_id,
        true as use_group_settings
    from activity_notifications
    inner join ticket_kinds kind
        on kind.activity_id = activity_notifications.activity_id
    inner join ticket_kind_allowed_groups
        on ticket_kind_allowed_groups.ticket_kind_id
            = kind.id
    inner join groups allowed_group
        on allowed_group.id = ticket_kind_allowed_groups.group_id
    inner join groups member_group
        on allowed_group.path @> member_group.path
    inner join group_memberships
        on group_memberships.group_id = member_group.id
    where member_group.limit_membership_visibility = false
        or member_group.id = allowed_group.id

    union

    -- Everyone who can buy a non-visibility ticket kind.
    select
        activity_buyers_notifications.notification_id,
        group_memberships.user_id,
        activity_buyers_notifications.activity_id,
        null::uuid as group_id,
        true as use_group_settings
    from activity_buyers_notifications
    inner join ticket_kinds kind
        on kind.activity_id = activity_buyers_notifications.activity_id
    inner join ticket_kind_allowed_groups
        on ticket_kind_allowed_groups.ticket_kind_id
            = kind.id
    inner join groups allowed_group
        on allowed_group.id = ticket_kind_allowed_groups.group_id
    inner join groups member_group
        on allowed_group.path @> member_group.path
    inner join group_memberships
        on group_memberships.group_id = member_group.id
    where kind.max_tickets > 0
        and (
            member_group.limit_membership_visibility = false
            or member_group.id = allowed_group.id
        )

    union

    -- Everyone who owns a ticket to the activity.
    select
        purchased_ticket_notifications.notification_id,
        purchased_tickets.owner_id,
        purchased_ticket_notifications.activity_id,
        null::uuid as group_id,
        false as use_group_settings
    from purchased_ticket_notifications
    inner join ticket_kinds
        on ticket_kinds.activity_id = purchased_ticket_notifications.activity_id
    inner join purchased_tickets
        on purchased_tickets.ticket_kind_id = ticket_kinds.id

    union

    -- Direct and descendant members of a group notification target.
    select
        group_notifications.notification_id,
        group_memberships.user_id,
        null::uuid as activity_id,
        groups.id as group_id,
        true as use_group_settings
    from group_notifications
    inner join groups on groups.id = group_notifications.group_id
    inner join groups member_group on groups.path @> member_group.path
    inner join group_memberships
        on group_memberships.group_id = member_group.id
    where member_group.limit_membership_visibility = false
        or member_group.id = groups.id
),
settings_by_host as (
    select
        eligible_users.notification_id,
        eligible_users.user_id,
        eligible_users.use_group_settings,
        activity_override.follow,
        coalesce(closest_setting.visible, false) as visible,
        coalesce(
            closest_setting.notification_level,
            'none'::notification_level
        ) as notification_level
    from eligible_users
    inner join lateral (
        select activity_hosts.group_id
        from activity_hosts
        where activity_hosts.activity_id = eligible_users.activity_id
        union all
        select eligible_users.group_id
        where eligible_users.activity_id is null
    ) preference_target on true
    inner join groups target_group on target_group.id = preference_target.group_id
    left join lateral (
        select settings.visible, settings.notification_level
        from user_group_settings settings
        inner join groups settings_group on settings_group.id = settings.group_id
        where settings.user_id = eligible_users.user_id
            and settings_group.path @> target_group.path
        order by nlevel(settings_group.path) desc
        limit 1
    ) closest_setting on true
    left join activity_notification_overrides activity_override
        on activity_override.user_id = eligible_users.user_id
        and activity_override.activity_id = eligible_users.activity_id
    where eligible_users.activity_id is null
        or exists (
            select 1
            from activities
            where activities.id = eligible_users.activity_id
            and activities.is_hidden = false
        )
)
select
    notification_id,
    user_id,
    case when bool_or(
        case
            when follow is true then true
            when follow is false then false
            when use_group_settings = false then true
            else notification_level = 'all'::notification_level
        end
    ) then 'all'::notification_level else 'none'::notification_level end
        as notification_level,
    bool_or(
        case
            when follow is true then true
            when follow is false then false
            when use_group_settings = false then true
            else visible
        end
    ) as visible
from settings_by_host
group by notification_id, user_id;
