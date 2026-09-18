alter table "public"."ticket_kind_addons" drop constraint "ticket_kind_addons_addon_id_activity_id_fkey";

alter table "public"."ticket_kind_addons" drop constraint "ticket_kind_addons_ticket_kind_id_activity_id_fkey";

alter table "public"."ticket_kinds" drop constraint "ticket_kinds_id_activity_id_key";

alter table "public"."ticket_addons" drop constraint "ticket_addons_id_activity_id_key";

drop index if exists "public"."ticket_addons_id_activity_id_key";

drop index if exists "public"."ticket_kinds_id_activity_id_key";

alter table "public"."ticket_kind_addons" drop column "activity_id";

alter table "public"."ticket_kind_addons" add constraint "ticket_kind_addons_addon_id_fkey" FOREIGN KEY ("addon_id") REFERENCES "public"."ticket_addons"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."ticket_kind_addons" validate constraint "ticket_kind_addons_addon_id_fkey";

alter table "public"."ticket_kind_addons" add constraint "ticket_kind_addons_ticket_kind_id_fkey" FOREIGN KEY ("ticket_kind_id") REFERENCES "public"."ticket_kinds"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."ticket_kind_addons" validate constraint "ticket_kind_addons_ticket_kind_id_fkey";


