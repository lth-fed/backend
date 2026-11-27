-- All prices live on add-ons. Every product gets a mandatory, customer-invisible base-price add-on
-- in a product-owned set (`id` = the product id) with exactly one option named after the product, so
-- `products.sales_price` and `purchases.product_price` become redundant.
alter table "public"."product_addons" add column "is_base_price" boolean not null default false;

-- One base add-on per set, and sets are per product, so one per product.
create unique index "product_addons_one_base_per_set" on "public"."product_addons" ("set_id")
    where "is_base_price";

-- Every product owns a set (questionnaire products already did).
insert into "public"."product_addon_sets" ("id", "product_id")
select "products"."id", "products"."id" from "public"."products"
on conflict ("id") do nothing;

insert into "public"."product_addons"
    ("id", "set_id", "idx", "name", "multiple_alternatives", "has_text_field", "required",
        "is_base_price")
select uuidv4(), "products"."id", -1, '{"en":"Price","sv":"Pris"}'::jsonb, false, false, true, true
from "public"."products";

-- The base add-on is linked like any other, so add-on loading finds it for that product.
insert into "public"."product_addon_links" ("product_id", "addon_id")
select "products"."id", "addon"."id"
from "public"."products"
inner join "public"."product_addons" "addon"
    on "addon"."set_id" = "products"."id" and "addon"."is_base_price"
on conflict do nothing;

insert into "public"."product_addon_options" ("id", "product_addon_id", "idx", "name", "price")
select uuidv4(), "addon"."id", 0, "products"."name", "products"."sales_price"
from "public"."products"
inner join "public"."product_addons" "addon"
    on "addon"."set_id" = "products"."id" and "addon"."is_base_price";

-- The whole base price is booked on the product's bookkeeping tag.
insert into "public"."product_addon_option_bookkeepings" ("id", "option_id", "price", "category")
select uuidv4(), "option"."id", "option"."price", "products"."bookkeeping_tag"
from "public"."products"
inner join "public"."product_addons" "addon"
    on "addon"."set_id" = "products"."id" and "addon"."is_base_price"
inner join "public"."product_addon_options" "option" on "option"."product_addon_id" = "addon"."id";

-- Keep the base price of historical purchases as a purchased base add-on, so reports and receipts
-- still know what was paid for the product itself.
insert into "public"."purchased_product_addons"
    ("addon_id", "purchase_id", "selected_options", "selected_options_prices", "selected_text")
select "addon"."id", "purchases"."id", '{0}'::integer[], array["purchases"."product_price"], ''
from "public"."purchases"
inner join "public"."product_addons" "addon"
    on "addon"."set_id" = "purchases"."product_id" and "addon"."is_base_price";

alter table "public"."purchases" drop column "product_price";
alter table "public"."products" drop column "sales_price";

-- The base price of a product, as an easily joined table.
create view "public"."product_base_prices" as
select
    "addon"."set_id" as "product_id",
    "option"."id" as "option_id",
    "option"."idx" as "option_idx",
    "option"."name" as "name",
    "option"."price" as "price"
from "public"."product_addons" "addon"
inner join "public"."product_addon_options" "option" on "option"."product_addon_id" = "addon"."id"
where "addon"."is_base_price";

-- The price range a product can be sold for: its base price plus the most expensive option of
-- every add-on a customer may pick (the activity's set and the product's own set).
create view "public"."product_price_ranges" as
select
    "product"."id" as "product_id",
    "base"."price" as "price_min",
    "base"."price" + coalesce((
        select sum((select max("option"."price")
            from "public"."product_addon_options" "option"
            where "option"."product_addon_id" = "addon"."id"))
        from "public"."product_addon_links" "link"
        inner join "public"."product_addons" "addon"
            on "addon"."id" = "link"."addon_id" and not "addon"."is_base_price"
        where "link"."product_id" = "product"."id"
        and "addon"."set_id" in (
            "product"."id",
            (select "assignment"."activity_id" from "public"."product_activity_assignments" "assignment"
                where "assignment"."product_id" = "product"."id"
                order by "assignment"."activity_id" limit 1)
        )
    ), '0'::money) as "price_max"
from "public"."products" "product"
inner join "public"."product_base_prices" "base" on "base"."product_id" = "product"."id";
