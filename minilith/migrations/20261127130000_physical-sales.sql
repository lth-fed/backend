-- Physical (counter) sales paid on a card reader. The header survives completion so a device can
-- poll the result; the item lines become purchases and are deleted in the provider callback.
create table "public"."physical_sales" (
    "transaction_id" uuid not null,
    "sales_device_id" uuid,
    "buyer_id" text not null,
    "total" money not null,
    "created_at" timestamp with time zone not null default now()
);
create table "public"."physical_sale_items" (
    "id" uuid not null default uuidv4(),
    "transaction_id" uuid not null,
    "product_id" uuid not null,
    "product_price" money not null,
    "activity_id" uuid
);
create index physical_sale_items_transaction on public.physical_sale_items using hash (transaction_id);
create table "public"."physical_sale_item_addons" (
    "item_id" uuid not null,
    "addon_id" uuid not null,
    "selected_options" integer[] not null,
    "selected_options_prices" money[] not null,
    "selected_text" text not null
);

create unique index physical_sales_pkey on public.physical_sales using btree (transaction_id);
create unique index physical_sale_items_pkey on public.physical_sale_items using btree (id);
create unique index physical_sale_item_addons_pkey
    on public.physical_sale_item_addons using btree (item_id, addon_id);

alter table "public"."physical_sales" add constraint physical_sales_pkey PRIMARY KEY using index physical_sales_pkey;
alter table "public"."physical_sale_items" add constraint physical_sale_items_pkey PRIMARY KEY using index physical_sale_items_pkey;
alter table "public"."physical_sale_item_addons" add constraint physical_sale_item_addons_pkey PRIMARY KEY using index physical_sale_item_addons_pkey;

alter table "public"."physical_sales" add constraint physical_sales_sales_device_id_fkey FOREIGN KEY ("sales_device_id") REFERENCES "public"."physical_sales_clients"("id") NOT VALID;
alter table "public"."physical_sales" add constraint physical_sales_buyer_id_fkey FOREIGN KEY ("buyer_id") REFERENCES "public"."users"("id") NOT VALID;
alter table "public"."physical_sale_items" add constraint physical_sale_items_transaction_id_fkey FOREIGN KEY ("transaction_id") REFERENCES "public"."physical_sales"("transaction_id") on delete cascade NOT VALID;
alter table "public"."physical_sale_items" add constraint physical_sale_items_product_id_fkey FOREIGN KEY ("product_id") REFERENCES "public"."products"("id") NOT VALID;
alter table "public"."physical_sale_items" add constraint physical_sale_items_activity_id_fkey FOREIGN KEY ("activity_id") REFERENCES "public"."activities"("id") NOT VALID;
alter table "public"."physical_sale_item_addons" add constraint physical_sale_item_addons_item_id_fkey FOREIGN KEY ("item_id") REFERENCES "public"."physical_sale_items"("id") on delete cascade NOT VALID;
alter table "public"."physical_sale_item_addons" add constraint physical_sale_item_addons_addon_id_fkey FOREIGN KEY ("addon_id") REFERENCES "public"."product_addons"("id") NOT VALID;
alter table "public"."physical_sale_item_addons" add constraint sale_addon_lengths_consistent check (
    cardinality(selected_options) = cardinality(selected_options_prices)
) not valid;

alter table "public"."physical_sales" validate constraint physical_sales_sales_device_id_fkey;
alter table "public"."physical_sales" validate constraint physical_sales_buyer_id_fkey;
alter table "public"."physical_sale_items" validate constraint physical_sale_items_transaction_id_fkey;
alter table "public"."physical_sale_items" validate constraint physical_sale_items_product_id_fkey;
alter table "public"."physical_sale_items" validate constraint physical_sale_items_activity_id_fkey;
alter table "public"."physical_sale_item_addons" validate constraint physical_sale_item_addons_item_id_fkey;
alter table "public"."physical_sale_item_addons" validate constraint physical_sale_item_addons_addon_id_fkey;
alter table "public"."physical_sale_item_addons" validate constraint sale_addon_lengths_consistent;
