-- Addon sets are owned by an activity or by a product (item products and questionnaires).
-- The owner's id doubles as the set id, so callers keep passing the activity id or product id.
alter table "public"."product_addon_sets" add column "product_id" uuid;

-- One set per product.
create unique index "product_addon_sets_by_product" on "public"."product_addon_sets" ("product_id");

alter table "public"."product_addon_sets" add constraint "product_addon_sets_product_id_fkey"
    FOREIGN KEY ("product_id") REFERENCES "public"."products"("id") ON DELETE CASCADE;

-- A set belongs to at most one owner.
alter table "public"."product_addon_sets" add constraint "product_addon_sets_single_owner"
    CHECK ("activity_id" is null or "product_id" is null);

-- Questionnaire sets were created without an activity; they own the post's product.
update "public"."product_addon_sets"
set "product_id" = "id"
where "activity_id" is null
    and exists (select 1 from "public"."products" where "products"."id" = "product_addon_sets"."id");
