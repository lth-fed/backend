with notification_allowed_groups as (
    select distinct
        allowed_group.id,
        allowed_group.path
    from ticket_kind_notifications
    inner join ticket_kind_allowed_groups
        on ticket_kind_allowed_groups.ticket_kind_id
            = ticket_kind_notifications.ticket_kind_id
    inner join groups as allowed_group
        on allowed_group.id = ticket_kind_allowed_groups.group_id
    where ticket_kind_notifications.notification_id = $1
),
visible_users as (
    select distinct
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
        visible_users.user_id,
        visible_users.allowed_group_id,
        user_group_settings.notification_level,
        row_number() over (
            partition by
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
),
recipient_users as (
    select distinct user_id
    from ranked_settings
    where precedence = 1
    and notification_level in (
        'all'::notification_level,
        'personalized'::notification_level
    )
)
select
    push_devices.user_id as "user_id!",
    push_devices.device_id as "device_id!",
    push_devices.push_token,
    push_devices.platform::push_platform as "platform!: crate::push_notifications::PushPlatform",
    users.language,
    users.nonce
from recipient_users
inner join push_devices
    on push_devices.user_id = recipient_users.user_id
inner join users
    on users.id = recipient_users.user_id
