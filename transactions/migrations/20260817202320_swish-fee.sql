alter table "public"."client_ids" add column "swish_payment_fee_fixed" money;
alter table "public"."client_ids" add column "swish_payment_fee_fraction" double precision;
alter table "public"."client_ids" add column "swish_payment_fee_max" money;
alter table "public"."client_ids" add column "swish_refund_fee" money;

update client_ids set swish_refund_fee = '3'::money, swish_payment_fee_fixed = '3'::money,
    swish_payment_fee_fraction = 0.0, swish_payment_fee_max = '3'::money;

alter table "public"."client_ids" alter column "swish_payment_fee_fixed" set not null;
alter table "public"."client_ids" alter column "swish_payment_fee_fraction" set not null;
alter table "public"."client_ids" alter column "swish_payment_fee_max" set not null;
alter table "public"."client_ids" alter column "swish_refund_fee" set not null;
