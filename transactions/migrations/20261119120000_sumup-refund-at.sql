alter table "public"."stripe_checkouts" drop constraint "stripe_checkouts_transaction_id_fkey";

alter table "public"."transaction_wares" drop constraint "transaction_wares_transaction_id_fkey";

alter table "public"."transactions" drop constraint "transactions_client_id_fkey";

alter type "public"."provider" rename to "provider__old_version_to_be_dropped";

create type "public"."provider" as enum ('swish', 'stripe', 'free', 'sumup');

alter table "public"."transactions" alter column provider type "public"."provider" using provider::text::"public"."provider";

drop type "public"."provider__old_version_to_be_dropped";

alter table "public"."transactions" add column "refund_at" timestamp with time zone;

alter table "public"."stripe_checkouts" add constraint "stripe_checkouts_transaction_id_fkey" FOREIGN KEY ("transaction_id") REFERENCES "public"."transactions"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."stripe_checkouts" validate constraint "stripe_checkouts_transaction_id_fkey";

alter table "public"."transaction_wares" add constraint "transaction_wares_transaction_id_fkey" FOREIGN KEY ("transaction_id") REFERENCES "public"."transactions"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."transaction_wares" validate constraint "transaction_wares_transaction_id_fkey";

alter table "public"."transactions" add constraint "transactions_client_id_fkey" FOREIGN KEY ("client_id") REFERENCES "public"."client_ids"("client_id") NOT VALID;

alter table "public"."transactions" validate constraint "transactions_client_id_fkey";



alter table "public"."client_ids" drop column "stripe_secret";
alter table "public"."client_ids" drop column "stripe_endpoint_secret";
alter table "public"."client_ids" add column "sumup_api_key" text;
alter table "public"."client_ids" add column "sumup_merchant_code" text;

alter table "public"."transactions" add column "sumup_checkout_id" text;

drop table "public"."stripe_checkouts";
drop table "public"."stripe_customers";

alter table "public"."client_ids" add column "sumup_online_fee_fraction" double precision not null default 0;
alter table "public"."client_ids" add column "sumup_physical_fee_fraction" double precision not null default 0;

alter table "public"."transactions" add column "cancelled_at" timestamp with time zone;
alter table "public"."transactions" add column "sumup_reader_id" text;

drop table "public"."transaction_reserved_ids";
