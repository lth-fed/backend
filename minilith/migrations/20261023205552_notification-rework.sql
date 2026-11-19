drop view if exists "public"."notification_recipients";

set check_function_bodies = off;

CREATE OR REPLACE FUNCTION public.follows_activity(target_user text, target_activity uuid)
 RETURNS boolean
 LANGUAGE sql
 STABLE
AS $function$
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
$function$
;

CREATE OR REPLACE FUNCTION public.wants_group_notifications(target_user text, target_group uuid)
 RETURNS boolean
 LANGUAGE sql
 STABLE
AS $function$
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
$function$
;

create table "public"."notification_read_states" (
    "user_id" text not null,
    "notification_id" uuid not null,
    "read" boolean,
    "dismissed" boolean not null default false
);


create table "public"."notification_read_watermarks" (
    "user_id" text not null,
    "read_up_to" timestamp with time zone not null
);


create or replace view "public"."notification_details" as  SELECT notifications.id,
        CASE
            WHEN group_notifications.group_id IS NULL THEN 'activity'::text
            ELSE 'group'::text
        END AS kind,
        CASE
            WHEN activity_notifications.activity_id IS NOT NULL THEN 'all'::text
            WHEN activity_buyers_notifications.activity_id IS NOT NULL THEN 'buyers'::text
            WHEN purchased_ticket_notifications.activity_id IS NOT NULL THEN 'ticket_holders'::text
            ELSE NULL::text
        END AS audience,
    COALESCE(activity_notifications.activity_id, activity_buyers_notifications.activity_id, purchased_ticket_notifications.activity_id) AS activity_id,
    group_notifications.group_id,
    COALESCE(activity_image.url, group_logo.url) AS image_url,
    notifications.sender,
    notifications.title,
    notifications.content,
    notifications.send_at,
    notifications.sent
   FROM notifications
     LEFT JOIN activity_notifications ON activity_notifications.notification_id = notifications.id
     LEFT JOIN activity_buyers_notifications ON activity_buyers_notifications.notification_id = notifications.id
     LEFT JOIN purchased_ticket_notifications ON purchased_ticket_notifications.notification_id = notifications.id
     LEFT JOIN group_notifications ON group_notifications.notification_id = notifications.id
     LEFT JOIN activities ON activities.id = COALESCE(activity_notifications.activity_id, activity_buyers_notifications.activity_id, purchased_ticket_notifications.activity_id)
     LEFT JOIN images activity_image ON activity_image.id = activities.image_id
     LEFT JOIN groups ON groups.id = group_notifications.group_id
     LEFT JOIN images group_logo ON group_logo.id = groups.logo_id;


create or replace view "public"."notification_recipients" as  WITH eligible_users AS (
         SELECT activity_notifications.notification_id,
            group_memberships.user_id,
            activity_notifications.activity_id,
            NULL::uuid AS group_id,
            false AS ticket_holder
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
            false AS ticket_holder
           FROM activity_buyers_notifications
             JOIN ticket_kinds kind ON kind.activity_id = activity_buyers_notifications.activity_id
             JOIN ticket_kind_allowed_groups ON ticket_kind_allowed_groups.ticket_kind_id = kind.id
             JOIN groups allowed_group ON allowed_group.id = ticket_kind_allowed_groups.group_id
             JOIN groups member_group ON allowed_group.path @> member_group.path
             JOIN group_memberships ON group_memberships.group_id = member_group.id
          WHERE NOT kind.for_visibility AND (member_group.limit_membership_visibility = false OR member_group.id = allowed_group.id)
        UNION
         SELECT purchased_ticket_notifications.notification_id,
            purchased_tickets.owner_id,
            purchased_ticket_notifications.activity_id,
            NULL::uuid AS group_id,
            true AS ticket_holder
           FROM purchased_ticket_notifications
             JOIN ticket_kinds ON ticket_kinds.activity_id = purchased_ticket_notifications.activity_id
             JOIN purchased_tickets ON purchased_tickets.ticket_kind_id = ticket_kinds.id
        UNION
         SELECT group_notifications.notification_id,
            group_memberships.user_id,
            NULL::uuid AS activity_id,
            groups.id AS group_id,
            false AS ticket_holder
           FROM group_notifications
             JOIN groups ON groups.id = group_notifications.group_id
             JOIN groups member_group ON groups.path @> member_group.path
             JOIN group_memberships ON group_memberships.group_id = member_group.id
          WHERE member_group.limit_membership_visibility = false OR member_group.id = groups.id
        )
 SELECT notification_id,
    user_id,
    bool_or(
        CASE
            WHEN ticket_holder THEN true
            WHEN activity_id IS NOT NULL THEN follows_activity(user_id, activity_id)
            ELSE wants_group_notifications(user_id, group_id)
        END) AS receives
   FROM eligible_users
  WHERE activity_id IS NULL OR (EXISTS ( SELECT 1
           FROM activities
          WHERE activities.id = eligible_users.activity_id AND activities.is_hidden = false))
  GROUP BY notification_id, user_id;


CREATE UNIQUE INDEX notification_read_states_pkey ON public.notification_read_states USING btree (user_id, notification_id);

CREATE UNIQUE INDEX notification_read_watermarks_pkey ON public.notification_read_watermarks USING btree (user_id);

alter table "public"."notification_read_states" add constraint "notification_read_states_pkey" PRIMARY KEY using index "notification_read_states_pkey";

alter table "public"."notification_read_watermarks" add constraint "notification_read_watermarks_pkey" PRIMARY KEY using index "notification_read_watermarks_pkey";

alter table "public"."notification_read_states" add constraint "notification_read_states_notification_id_fkey" FOREIGN KEY ("notification_id") REFERENCES "public"."notifications"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."notification_read_states" validate constraint "notification_read_states_notification_id_fkey";

alter table "public"."notification_read_states" add constraint "notification_read_states_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."notification_read_states" validate constraint "notification_read_states_user_id_fkey";

alter table "public"."notification_read_watermarks" add constraint "notification_read_watermarks_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."notification_read_watermarks" validate constraint "notification_read_watermarks_user_id_fkey";



-- Data carry-over: read state starts now. Everything already in a user's history counts as read, so
-- nobody opens the app to six months of unread notifications.
insert into "public"."notification_read_watermarks" (user_id, read_up_to)
select id, now() from "public"."users";
