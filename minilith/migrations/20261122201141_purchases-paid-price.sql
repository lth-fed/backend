alter table "public"."purchases" add column "paid_price" money;

-- Existing rows: the current owner paid the original price plus the addon prices. Rows that were
-- already resold lose the exact amount (it was never stored); the original price + addons is the
-- upper bound a refund was previously allowed to reach.
update purchases
set paid_price = product_price + coalesce((
    select sum(option_price)
    from purchased_product_addons addon
    cross join lateral unnest(addon.selected_options_prices) option_price
    where addon.purchase_id = purchases.id
), '0.00'::money);

alter table "public"."purchases" alter column "paid_price" set not null;
