drop view if exists "public"."notification_recipients";

alter table "public"."user_group_settings"
alter column "notification_level" type text
using "notification_level"::text;

drop type "public"."notification_level";

create type "public"."notification_level" as enum ('none', 'all');

create table "public"."activity_buyers_notifications" (
    "id" uuid not null,
    "activity_id" uuid not null,
    "notification_id" uuid not null
);


create table "public"."activity_notification_overrides" (
    "user_id" text not null,
    "activity_id" uuid not null,
    "follow" boolean not null
);


create table "public"."activity_notifications" (
    "id" uuid not null,
    "activity_id" uuid not null,
    "notification_id" uuid not null
);


create table "public"."purchased_ticket_notifications" (
    "id" uuid not null,
    "activity_id" uuid not null,
    "notification_id" uuid not null
);


update "public"."user_group_settings" as settings
set "notification_level" = case
    when groups.path = 'tlth'::ltree then 'none'
    else 'all'
end
from "public"."groups"
where groups.id = settings.group_id;

alter table "public"."user_group_settings"
alter column "notification_level" type "public"."notification_level"
using "notification_level"::"public"."notification_level";

alter table "public"."notifications" add column "sender" jsonb;

update "public"."notifications" as notification
set sender = coalesce(
    (
        select activities.title
        from "public"."ticket_kind_notifications" links
        inner join "public"."ticket_kinds" kinds on kinds.id = links.ticket_kind_id
        inner join "public"."activities" activities on activities.id = kinds.activity_id
        where links.notification_id = notification.id
        limit 1
    ),
    (
        select groups.name
        from "public"."group_notifications" links
        inner join "public"."groups" groups on groups.id = links.group_id
        where links.notification_id = notification.id
        limit 1
    ),
    '{}'::jsonb
);

alter table "public"."notifications" alter column "sender" set not null;

create temporary table ticket_notifications_to_migrate on commit drop as
select
    links.notification_id,
    kinds.activity_id,
    row_number() over (
        partition by notifications.send_at, kinds.activity_id
        order by links.ticket_kind_id, links.id, links.notification_id
    ) as duplicate_rank
from "public"."ticket_kind_notifications" links
inner join "public"."ticket_kinds" kinds on kinds.id = links.ticket_kind_id
inner join "public"."notifications" notifications on notifications.id = links.notification_id;

insert into "public"."activity_buyers_notifications" (id, activity_id, notification_id)
select uuidv4(), activity_id, notification_id
from ticket_notifications_to_migrate
where duplicate_rank = 1;

delete from "public"."notifications"
where id in (
    select notification_id
    from ticket_notifications_to_migrate
    where duplicate_rank > 1
);

drop table "public"."ticket_kind_notifications";

create view "public"."notification_recipients" as
with eligible_users as (
    select activity_notifications.notification_id, group_memberships.user_id,
        activity_notifications.activity_id, null::uuid as group_id
    from activity_notifications
    inner join ticket_kinds kind on kind.activity_id = activity_notifications.activity_id
    inner join ticket_kind_allowed_groups on ticket_kind_allowed_groups.ticket_kind_id = kind.id
    inner join groups allowed_group on allowed_group.id = ticket_kind_allowed_groups.group_id
    inner join groups member_group on allowed_group.path @> member_group.path
    inner join group_memberships on group_memberships.group_id = member_group.id
    where member_group.limit_membership_visibility = false
        or member_group.id = allowed_group.id

    union

    select activity_buyers_notifications.notification_id, group_memberships.user_id,
        activity_buyers_notifications.activity_id, null::uuid as group_id
    from activity_buyers_notifications
    inner join ticket_kinds kind on kind.activity_id = activity_buyers_notifications.activity_id
    inner join ticket_kind_allowed_groups on ticket_kind_allowed_groups.ticket_kind_id = kind.id
    inner join groups allowed_group on allowed_group.id = ticket_kind_allowed_groups.group_id
    inner join groups member_group on allowed_group.path @> member_group.path
    inner join group_memberships on group_memberships.group_id = member_group.id
    where kind.max_tickets > 0
        and (member_group.limit_membership_visibility = false
            or member_group.id = allowed_group.id)

    union

    select purchased_ticket_notifications.notification_id, purchased_tickets.owner_id,
        purchased_ticket_notifications.activity_id, null::uuid as group_id
    from purchased_ticket_notifications
    inner join ticket_kinds
        on ticket_kinds.activity_id = purchased_ticket_notifications.activity_id
    inner join purchased_tickets on purchased_tickets.ticket_kind_id = ticket_kinds.id

    union

    select group_notifications.notification_id, group_memberships.user_id,
        null::uuid as activity_id, groups.id as group_id
    from group_notifications
    inner join groups on groups.id = group_notifications.group_id
    inner join groups member_group on groups.path @> member_group.path
    inner join group_memberships on group_memberships.group_id = member_group.id
    where member_group.limit_membership_visibility = false or member_group.id = groups.id
), settings_by_host as (
    select eligible_users.notification_id, eligible_users.user_id,
        activity_override.follow,
        coalesce(closest_setting.visible, true) as visible,
        coalesce(closest_setting.notification_level, 'all'::notification_level)
            as notification_level
    from eligible_users
    inner join lateral (
        select activity_hosts.group_id
        from activity_hosts
        where activity_hosts.activity_id = eligible_users.activity_id
        union all
        select eligible_users.group_id where eligible_users.activity_id is null
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
)
select notification_id, user_id,
    case when bool_or(
        case when follow is true then true when follow is false then false
            else notification_level = 'all'::notification_level end
    ) then 'all'::notification_level else 'none'::notification_level end
        as notification_level,
    bool_or(
        case when follow is true then true when follow is false then false else visible end
    ) as visible
from settings_by_host
group by notification_id, user_id;


CREATE UNIQUE INDEX activity_buyers_notifications_pkey ON public.activity_buyers_notifications USING btree (id, activity_id);

CREATE UNIQUE INDEX activity_notification_overrides_pkey ON public.activity_notification_overrides USING btree (user_id, activity_id);

CREATE UNIQUE INDEX activity_notifications_pkey ON public.activity_notifications USING btree (id, activity_id);

CREATE UNIQUE INDEX purchased_ticket_notifications_pkey ON public.purchased_ticket_notifications USING btree (id, activity_id);

alter table "public"."activity_buyers_notifications" add constraint "activity_buyers_notifications_pkey" PRIMARY KEY using index "activity_buyers_notifications_pkey";

alter table "public"."activity_notification_overrides" add constraint "activity_notification_overrides_pkey" PRIMARY KEY using index "activity_notification_overrides_pkey";

alter table "public"."activity_notifications" add constraint "activity_notifications_pkey" PRIMARY KEY using index "activity_notifications_pkey";

alter table "public"."purchased_ticket_notifications" add constraint "purchased_ticket_notifications_pkey" PRIMARY KEY using index "purchased_ticket_notifications_pkey";

alter table "public"."activity_buyers_notifications" add constraint "activity_buyers_notifications_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") NOT VALID;

alter table "public"."activity_buyers_notifications" validate constraint "activity_buyers_notifications_activity_id_fkey";

alter table "public"."activity_buyers_notifications" add constraint "activity_buyers_notifications_notification_id_fkey" FOREIGN KEY ("notification_id") REFERENCES "public"."notifications"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."activity_buyers_notifications" validate constraint "activity_buyers_notifications_notification_id_fkey";

alter table "public"."activity_notification_overrides" add constraint "activity_notification_overrides_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."activity_notification_overrides" validate constraint "activity_notification_overrides_activity_id_fkey";

alter table "public"."activity_notification_overrides" add constraint "activity_notification_overrides_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."activity_notification_overrides" validate constraint "activity_notification_overrides_user_id_fkey";

alter table "public"."activity_notifications" add constraint "activity_notifications_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") NOT VALID;

alter table "public"."activity_notifications" validate constraint "activity_notifications_activity_id_fkey";

alter table "public"."activity_notifications" add constraint "activity_notifications_notification_id_fkey" FOREIGN KEY ("notification_id") REFERENCES "public"."notifications"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."activity_notifications" validate constraint "activity_notifications_notification_id_fkey";

alter table "public"."purchased_ticket_notifications" add constraint "purchased_ticket_notifications_activity_id_fkey" FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") NOT VALID;

alter table "public"."purchased_ticket_notifications" validate constraint "purchased_ticket_notifications_activity_id_fkey";

alter table "public"."purchased_ticket_notifications" add constraint "purchased_ticket_notifications_notification_id_fkey" FOREIGN KEY ("notification_id") REFERENCES "public"."notifications"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."purchased_ticket_notifications" validate constraint "purchased_ticket_notifications_notification_id_fkey";

create index activity_notifications_by_activity on activity_notifications (activity_id);
create index activity_notifications_by_notification on activity_notifications (notification_id);
create index activity_buyers_notifications_by_activity
    on activity_buyers_notifications (activity_id);
create index activity_buyers_notifications_by_notification
    on activity_buyers_notifications (notification_id);
create index purchased_ticket_notifications_by_activity
    on purchased_ticket_notifications (activity_id);
create index purchased_ticket_notifications_by_notification
    on purchased_ticket_notifications (notification_id);
create index group_notifications_by_group on group_notifications (group_id);
create index group_notifications_by_notification on group_notifications (notification_id);
