alter table "public"."ticket_addons" drop constraint "ticket_addons_ticket_kind_id_fkey";

alter table "public"."ticket_kinds" drop constraint "ticket_kinds_check";

alter table "public"."ticket_kinds" drop constraint "ticket_kinds_check1";

create table "public"."admin_personal_accounts" (
    "admin_id" text not null,
    "user_id" text not null
);


create table "public"."ticket_kind_addons" (
    "ticket_kind_id" uuid not null,
    "addon_id" uuid not null,
    "activity_id" uuid not null
);


alter table "public"."groups" alter column "propogate_member_visibility_access" set default false;

alter table "public"."ticket_addons" add column "activity_id" uuid;
update ticket_addons addon set activity_id = kind.activity_id
from ticket_kinds kind where kind.id = addon.ticket_kind_id;
insert into ticket_kind_addons (ticket_kind_id, addon_id, activity_id)
select ticket_kind_id, id, activity_id from ticket_addons;
alter table "public"."ticket_addons" alter column "activity_id" set not null;
alter table "public"."ticket_addons" drop column "ticket_kind_id";

alter table "public"."ticket_kinds" add column "for_visibility" boolean not null default false;
update ticket_kinds set for_visibility = true where max_tickets = 0;

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
          WHERE NOT kind.for_visibility AND (member_group.limit_membership_visibility = false OR member_group.id = allowed_group.id)
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
          WHERE eligible_users.activity_id IS NULL OR (EXISTS ( SELECT 1
                   FROM activities
                  WHERE activities.id = eligible_users.activity_id AND activities.is_hidden = false))
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


CREATE UNIQUE INDEX admin_personal_accounts_pkey ON public.admin_personal_accounts USING btree (admin_id);

CREATE INDEX admin_personal_accounts_user ON public.admin_personal_accounts USING btree (user_id);

CREATE UNIQUE INDEX ticket_addons_id_activity_id_key ON public.ticket_addons USING btree (id, activity_id);

CREATE UNIQUE INDEX ticket_kind_addons_pkey ON public.ticket_kind_addons USING btree (ticket_kind_id, addon_id);

CREATE UNIQUE INDEX ticket_kinds_id_activity_id_key ON public.ticket_kinds USING btree (id, activity_id);

alter table "public"."admin_personal_accounts" add constraint "admin_personal_accounts_pkey" PRIMARY KEY using index "admin_personal_accounts_pkey";

alter table "public"."ticket_kind_addons" add constraint "ticket_kind_addons_pkey" PRIMARY KEY using index "ticket_kind_addons_pkey";

alter table "public"."admin_personal_accounts" add constraint "admin_personal_accounts_admin_id_check" CHECK ((admin_id ~~ 'email:%'::text)) not valid;

alter table "public"."admin_personal_accounts" validate constraint "admin_personal_accounts_admin_id_check";

alter table "public"."admin_personal_accounts" add constraint "admin_personal_accounts_admin_id_fkey" FOREIGN KEY ("admin_id") REFERENCES "public"."users"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."admin_personal_accounts" validate constraint "admin_personal_accounts_admin_id_fkey";

alter table "public"."admin_personal_accounts" add constraint "admin_personal_accounts_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."admin_personal_accounts" validate constraint "admin_personal_accounts_user_id_fkey";

alter table "public"."ticket_addons" add constraint "ticket_addons_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") NOT VALID;

alter table "public"."ticket_addons" validate constraint "ticket_addons_activity_id_fkey";

alter table "public"."ticket_addons" add constraint "ticket_addons_id_activity_id_key" UNIQUE using index "ticket_addons_id_activity_id_key";

alter table "public"."ticket_kind_addons" add constraint "ticket_kind_addons_addon_id_activity_id_fkey" FOREIGN KEY ("addon_id", "activity_id") REFERENCES "public"."ticket_addons"("id", "activity_id") NOT VALID;

alter table "public"."ticket_kind_addons" validate constraint "ticket_kind_addons_addon_id_activity_id_fkey";

alter table "public"."ticket_kind_addons" add constraint "ticket_kind_addons_ticket_kind_id_activity_id_fkey" FOREIGN KEY ("ticket_kind_id", "activity_id") REFERENCES "public"."ticket_kinds"("id", "activity_id") ON DELETE CASCADE NOT VALID;

alter table "public"."ticket_kind_addons" validate constraint "ticket_kind_addons_ticket_kind_id_activity_id_fkey";

alter table "public"."ticket_kinds" add constraint "ticket_kinds_check2" CHECK (((reserved_or_purchased_tickets >= 0) AND (reserved_or_purchased_tickets <= max_tickets))) not valid;

alter table "public"."ticket_kinds" validate constraint "ticket_kinds_check2";

alter table "public"."ticket_kinds" add constraint "ticket_kinds_id_activity_id_key" UNIQUE using index "ticket_kinds_id_activity_id_key";

alter table "public"."ticket_kinds" add constraint "ticket_kinds_check" CHECK (((NOT for_visibility) OR (max_tickets = 0))) not valid;

alter table "public"."ticket_kinds" validate constraint "ticket_kinds_check";

alter table "public"."ticket_kinds" add constraint "ticket_kinds_check1" CHECK ((((min_tickets = 0) AND (max_tickets = 0)) OR (min_tickets < max_tickets))) not valid;

alter table "public"."ticket_kinds" validate constraint "ticket_kinds_check1";

