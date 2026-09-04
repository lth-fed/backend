alter table "public"."users_in_purchase_flow" add column "lock_id" uuid;

alter table "public"."users_in_purchase_flow" add column "locked_at" timestamp with time zone;


