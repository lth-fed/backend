create or replace view "public"."notification_recipients" as  WITH notification_allowed_groups AS (
         SELECT DISTINCT ticket_kind_notifications.notification_id,
            allowed_group.id,
            allowed_group.path
           FROM ticket_kind_notifications
             JOIN ticket_kind_allowed_groups ON ticket_kind_allowed_groups.ticket_kind_id = ticket_kind_notifications.ticket_kind_id
             JOIN groups allowed_group ON allowed_group.id = ticket_kind_allowed_groups.group_id
        UNION
         SELECT DISTINCT group_notifications.notification_id,
            groups.id,
            groups.path
           FROM group_notifications
             JOIN groups ON groups.id = group_notifications.group_id
        ), visible_users AS (
         SELECT DISTINCT notification_allowed_groups.notification_id,
            group_memberships.user_id,
            notification_allowed_groups.id AS allowed_group_id,
            notification_allowed_groups.path AS allowed_group_path
           FROM notification_allowed_groups
             JOIN groups member_group ON notification_allowed_groups.path @> member_group.path
             JOIN group_memberships ON group_memberships.group_id = member_group.id
          WHERE member_group.limit_membership_visibility = false OR member_group.id = notification_allowed_groups.id
        ), ranked_settings AS (
         SELECT visible_users.notification_id,
            visible_users.user_id,
            visible_users.allowed_group_id,
            user_group_settings.notification_level,
            row_number() OVER (PARTITION BY visible_users.notification_id, visible_users.user_id, visible_users.allowed_group_id ORDER BY (nlevel(settings_group.path)) DESC) AS precedence
           FROM visible_users
             JOIN user_group_settings ON user_group_settings.user_id = visible_users.user_id
             JOIN groups settings_group ON settings_group.id = user_group_settings.group_id AND settings_group.path @> visible_users.allowed_group_path
        )
 SELECT DISTINCT notification_id,
    user_id
   FROM ranked_settings
  WHERE precedence = 1 AND (notification_level = ANY (ARRAY['all'::notification_level, 'personalized'::notification_level]));



