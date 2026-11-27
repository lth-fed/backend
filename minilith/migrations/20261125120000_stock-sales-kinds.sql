-- Consumer sales now have their own kinds; `purchase`/`refund` mean us buying from / returning to a
-- supplier and move the physical stock in the store. The type is recreated (instead of appended to)
-- so the value order matches `schema/14-stock.sql`.
alter type "public"."stock_record_kind" rename to "stock_record_kind__old_version_to_be_dropped";

create type "public"."stock_record_kind" as enum ('purchase', 'refund', 'sales', 'sales-refund', 'take-out', 'put-back', 'balance');

alter table "public"."stock_transaction_records"
    alter column kind type "public"."stock_record_kind" using kind::text::"public"."stock_record_kind";

drop type "public"."stock_record_kind__old_version_to_be_dropped";
