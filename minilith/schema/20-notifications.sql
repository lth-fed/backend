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

-- Per-user read state for the notification history (the inbox). A notification is read when its row
-- says so, otherwise when it was sent at or before the user's watermark — which "mark all as read"
-- moves, and which defaults to the account's creation so a new account doesn't start with months of
-- unread history. `read` is null when the row only records a dismissal.
create table notification_read_watermarks (
    user_id text primary key references users (id) on delete cascade,
    read_up_to timestamptz not null
);
create table notification_read_states (
    user_id text not null references users (id) on delete cascade,
    notification_id uuid not null references notifications (id) on delete cascade,
    primary key (user_id, notification_id),
    read boolean,
    -- Dismissed notifications leave the inbox but stay in their activity's updates.
    dismissed boolean not null default false
);

-- What a notification is about, resolved once for every reader of it.
create view notification_details as
select
    notifications.id,
    case when group_notifications.group_id is null then 'activity' else 'group' end as kind,
    case
        when activity_notifications.activity_id is not null then 'all'
        when activity_buyers_notifications.activity_id is not null then 'buyers'
        when purchased_ticket_notifications.activity_id is not null then 'ticket_holders'
    end as audience,
    coalesce(
        activity_notifications.activity_id,
        activity_buyers_notifications.activity_id,
        purchased_ticket_notifications.activity_id
    ) as activity_id,
    group_notifications.group_id,
    coalesce(activity_image.url, group_logo.url) as image_url,
    notifications.sender,
    notifications.title,
    notifications.content,
    notifications.send_at,
    notifications.sent
from notifications
left join activity_notifications on activity_notifications.notification_id = notifications.id
left join activity_buyers_notifications
    on activity_buyers_notifications.notification_id = notifications.id
left join purchased_ticket_notifications
    on purchased_ticket_notifications.notification_id = notifications.id
left join group_notifications on group_notifications.notification_id = notifications.id
left join activities on activities.id = coalesce(
    activity_notifications.activity_id,
    activity_buyers_notifications.activity_id,
    purchased_ticket_notifications.activity_id
)
left join images activity_image on activity_image.id = activities.image_id
left join groups on groups.id = group_notifications.group_id
left join images group_logo on group_logo.id = groups.logo_id;

-- Whether a user wants notifications from a group: its nearest configured setting, or everything
-- when nothing is configured. Notification levels are independent of the home feed's visibility
-- filter; hiding a group's activities doesn't mute it.
create function wants_group_notifications(target_user text, target_group uuid) returns boolean
language sql stable as $$
    select coalesce(
        (
            select settings.notification_level = 'all'::notification_level
            from user_group_settings settings
            inner join groups settings_group on settings_group.id = settings.group_id
            inner join groups target on target.id = target_group
            where settings.user_id = target_user
                and settings_group.path @> target.path
            order by nlevel(settings_group.path) desc
            limit 1
        ),
        true
    )
$$;

-- Whether a user wants an activity's news: their own choice for the activity (the follow bell),
-- otherwise what they want from any of its hosts.
create function follows_activity(target_user text, target_activity uuid) returns boolean
language sql stable as $$
    select coalesce(
        (
            select follow from activity_notification_overrides
            where user_id = target_user and activity_id = target_activity
        ),
        (
            select bool_or(wants_group_notifications(target_user, activity_hosts.group_id))
            from activity_hosts
            where activity_hosts.activity_id = target_activity
        ),
        true
    )
$$;

-- Every user each notification may reach (eligibility), and whether it does (`receives`): pushes go
-- to receivers, and the history lists what was received. Ticket-holder news always reaches ticket
-- holders — muting an activity you hold a ticket for only mutes its general news.
create view notification_recipients as
with eligible_users as (
    -- Everyone who can see the activity.
    select
        activity_notifications.notification_id,
        group_memberships.user_id,
        activity_notifications.activity_id,
        null::uuid as group_id,
        false as ticket_holder
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
        false as ticket_holder
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
    where not kind.for_visibility
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
        true as ticket_holder
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
        false as ticket_holder
    from group_notifications
    inner join groups on groups.id = group_notifications.group_id
    inner join groups member_group on groups.path @> member_group.path
    inner join group_memberships
        on group_memberships.group_id = member_group.id
    where member_group.limit_membership_visibility = false
        or member_group.id = groups.id
)
select
    notification_id,
    user_id,
    bool_or(
        case
            when ticket_holder then true
            when activity_id is not null then follows_activity(user_id, activity_id)
            else wants_group_notifications(user_id, group_id)
        end
    ) as receives
from eligible_users
where activity_id is null
    or exists (
        select 1
        from activities
        where activities.id = eligible_users.activity_id
        and activities.is_hidden = false
    )
group by notification_id, user_id;
