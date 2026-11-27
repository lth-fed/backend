-- Every price is an add-on option, and each option carries its own bookkeeping split in
-- `product_addon_option_bookkeepings`, so the product-level tag is redundant.
alter table "public"."products" drop constraint "products_bookkeeping_tag_fkey";

alter table "public"."products" drop column "bookkeeping_tag";
