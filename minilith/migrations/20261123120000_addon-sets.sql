-- Addons used to be owned by an activity (`product_addons.activity_id`). Sets let a product own
-- addons without an activity, which is what questionnaire products use. Activity sets reuse the
-- activity id, so callers that already have an activity can keep passing it as the set id.
create table "public"."product_addon_sets" (
    "id" uuid not null,
    "activity_id" uuid,
    primary key ("id")
);

-- One set per activity.
create unique index "product_addon_sets_by_activity" on "public"."product_addon_sets" ("activity_id");

alter table "public"."product_addon_sets" add constraint "product_addon_sets_activity_id_fkey"
    FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") ON DELETE CASCADE;

insert into "public"."product_addon_sets" ("id", "activity_id")
select distinct activity_id, activity_id from "public"."product_addons";

alter table "public"."product_addons" add column "set_id" uuid;
update "public"."product_addons" set "set_id" = "activity_id";
alter table "public"."product_addons" alter column "set_id" set not null;
alter table "public"."product_addons" drop constraint "product_addons_activity_id_fkey";
alter table "public"."product_addons" drop column "activity_id";
alter table "public"."product_addons" add constraint "product_addons_set_id_fkey"
    FOREIGN KEY ("set_id") REFERENCES "public"."product_addon_sets"("id") ON DELETE CASCADE;

-- Deleting an addon (or its set/activity) now cleans up its options and their bookkeeping.
alter table "public"."product_addon_options" drop constraint "product_addon_options_product_addon_id_fkey";
alter table "public"."product_addon_options" add constraint "product_addon_options_product_addon_id_fkey"
    FOREIGN KEY ("product_addon_id") REFERENCES "public"."product_addons"("id") ON DELETE CASCADE;

alter table "public"."product_addon_option_bookkeepings"
    drop constraint "product_addon_option_bookkeepings_option_id_fkey";
alter table "public"."product_addon_option_bookkeepings"
    add constraint "product_addon_option_bookkeepings_option_id_fkey"
    FOREIGN KEY ("option_id") REFERENCES "public"."product_addon_options"("id") ON DELETE CASCADE;
