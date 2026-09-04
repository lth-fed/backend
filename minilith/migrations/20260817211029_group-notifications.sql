create table "public"."group_notifications" (
    "id" uuid not null,
    "group_id" uuid not null,
    "notification_id" uuid not null
);


CREATE UNIQUE INDEX group_notifications_pkey ON public.group_notifications USING btree (id);

alter table "public"."group_notifications" add constraint "group_notifications_pkey" PRIMARY KEY using index "group_notifications_pkey";

alter table "public"."group_notifications" add constraint "group_notifications_group_id_fkey" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") NOT VALID;

alter table "public"."group_notifications" validate constraint "group_notifications_group_id_fkey";

alter table "public"."group_notifications" add constraint "group_notifications_notification_id_fkey" FOREIGN KEY ("notification_id") REFERENCES "public"."notifications"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."group_notifications" validate constraint "group_notifications_notification_id_fkey";


