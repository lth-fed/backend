create or replace view "public"."notification_recipients" as  WITH eligible_users AS (
         SELECT activity_notifications.notification_id,
            group_memberships.user_id,
            activity_notifications.activity_id,
            NULL::uuid AS group_id,
            true AS use_group_settings
           FROM activity_notifications
             JOIN ticket_kinds kind ON kind.activity_id = activity_notifications.activity_id
             JOIN ticket_kind_allowed_groups ON ticket_kind_allowed_groups.ticket_kind_id = kind.id
             JOIN groups allowed_group ON allowed_group.id = ticket_kind_allowed_groups.group_id
             JOIN groups member_group ON allowed_group.path @> member_group.path
             JOIN group_memberships ON group_memberships.group_id = member_group.id
          WHERE member_group.limit_membership_visibility = false OR member_group.id = allowed_group.id
        UNION
         SELECT activity_buyers_notifications.notification_id,
            group_memberships.user_id,
            activity_buyers_notifications.activity_id,
            NULL::uuid AS group_id,
            true AS use_group_settings
           FROM activity_buyers_notifications
             JOIN ticket_kinds kind ON kind.activity_id = activity_buyers_notifications.activity_id
             JOIN ticket_kind_allowed_groups ON ticket_kind_allowed_groups.ticket_kind_id = kind.id
             JOIN groups allowed_group ON allowed_group.id = ticket_kind_allowed_groups.group_id
             JOIN groups member_group ON allowed_group.path @> member_group.path
             JOIN group_memberships ON group_memberships.group_id = member_group.id
          WHERE kind.max_tickets > 0 AND (member_group.limit_membership_visibility = false OR member_group.id = allowed_group.id)
        UNION
         SELECT purchased_ticket_notifications.notification_id,
            purchased_tickets.owner_id,
            purchased_ticket_notifications.activity_id,
            NULL::uuid AS group_id,
            false AS use_group_settings
           FROM purchased_ticket_notifications
             JOIN ticket_kinds ON ticket_kinds.activity_id = purchased_ticket_notifications.activity_id
             JOIN purchased_tickets ON purchased_tickets.ticket_kind_id = ticket_kinds.id
        UNION
         SELECT group_notifications.notification_id,
            group_memberships.user_id,
            NULL::uuid AS activity_id,
            groups.id AS group_id,
            true AS use_group_settings
           FROM group_notifications
             JOIN groups ON groups.id = group_notifications.group_id
             JOIN groups member_group ON groups.path @> member_group.path
             JOIN group_memberships ON group_memberships.group_id = member_group.id
          WHERE member_group.limit_membership_visibility = false OR member_group.id = groups.id
        ), settings_by_host AS (
         SELECT eligible_users.notification_id,
            eligible_users.user_id,
            eligible_users.use_group_settings,
            activity_override.follow,
            COALESCE(closest_setting.visible, false) AS visible,
            COALESCE(closest_setting.notification_level, 'none'::notification_level) AS notification_level
           FROM eligible_users
             JOIN LATERAL ( SELECT activity_hosts.group_id
                   FROM activity_hosts
                  WHERE activity_hosts.activity_id = eligible_users.activity_id
                UNION ALL
                 SELECT eligible_users.group_id
                  WHERE eligible_users.activity_id IS NULL) preference_target ON true
             JOIN groups target_group ON target_group.id = preference_target.group_id
             LEFT JOIN LATERAL ( SELECT settings.visible,
                    settings.notification_level
                   FROM user_group_settings settings
                     JOIN groups settings_group ON settings_group.id = settings.group_id
                  WHERE settings.user_id = eligible_users.user_id AND settings_group.path @> target_group.path
                  ORDER BY (nlevel(settings_group.path)) DESC
                 LIMIT 1) closest_setting ON true
             LEFT JOIN activity_notification_overrides activity_override ON activity_override.user_id = eligible_users.user_id AND activity_override.activity_id = eligible_users.activity_id
        )
 SELECT notification_id,
    user_id,
        CASE
            WHEN bool_or(
            CASE
                WHEN follow IS TRUE THEN true
                WHEN follow IS FALSE THEN false
                WHEN use_group_settings = false THEN true
                ELSE notification_level = 'all'::notification_level
            END) THEN 'all'::notification_level
            ELSE 'none'::notification_level
        END AS notification_level,
    bool_or(
        CASE
            WHEN follow IS TRUE THEN true
            WHEN follow IS FALSE THEN false
            WHEN use_group_settings = false THEN true
            ELSE visible
        END) AS visible
   FROM settings_by_host
  GROUP BY notification_id, user_id;



