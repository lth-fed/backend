-- `schema/12-products.sql` always declared one buyer per (activity, product, user) with a composite
-- key, but no migration created it. Align the database with the schema.
alter table "public"."product_buyers" drop constraint "product_buyers_pkey";

drop index if exists "public"."product_buyers_pkey";

CREATE UNIQUE INDEX product_buyers_pkey ON public.product_buyers USING btree (activity_id, product_id, user_id);

alter table "public"."product_buyers" add constraint "product_buyers_pkey" PRIMARY KEY using index "product_buyers_pkey";

alter table "public"."product_buyers" add constraint "product_buyers_activity_id_product_id_fkey" FOREIGN KEY ("activity_id", "product_id") REFERENCES "public"."product_activity_assignments"("activity_id", "product_id") NOT VALID;

alter table "public"."product_buyers" validate constraint "product_buyers_activity_id_product_id_fkey";
