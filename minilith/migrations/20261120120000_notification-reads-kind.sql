create type "public"."notification_kind" as enum ('activity', 'news', 'ticket_release', 'transfer', 'reservation');

alter table "public"."push_notifications" add column "kind" "public"."notification_kind";

update "public"."push_notifications" as notification
set "kind" = case
    when exists (
        select 1 from "public"."news_push_notifications" link
        where link.notification_id = notification.id
    ) then 'news'::"public"."notification_kind"
    when exists (
        select 1 from "public"."ticket_release_push_notifications" link
        where link.notification_id = notification.id
    ) then 'ticket_release'::"public"."notification_kind"
    else 'activity'::"public"."notification_kind"
end;

alter table "public"."push_notifications" alter column "kind" set not null;

create table "public"."notification_reads" (
    "user_id" text not null,
    "notification_id" uuid not null,
    "read_at" timestamptz not null default now()
);

create table "public"."user_notification_state" (
    "user_id" text not null,
    "last_full_read" timestamptz not null default now()
);

alter table "public"."notification_reads" add constraint "notification_reads_pkey" PRIMARY KEY ("user_id", "notification_id");

alter table "public"."user_notification_state" add constraint "user_notification_state_pkey" PRIMARY KEY ("user_id");

create index "notification_reads_by_notification" on "public"."notification_reads" using btree ("notification_id");

alter table "public"."notification_reads" add constraint "notification_reads_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."notification_reads" validate constraint "notification_reads_user_id_fkey";

alter table "public"."notification_reads" add constraint "notification_reads_notification_id_fkey" FOREIGN KEY ("notification_id") REFERENCES "public"."push_notifications"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."notification_reads" validate constraint "notification_reads_notification_id_fkey";

alter table "public"."user_notification_state" add constraint "user_notification_state_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."user_notification_state" validate constraint "user_notification_state_user_id_fkey";
