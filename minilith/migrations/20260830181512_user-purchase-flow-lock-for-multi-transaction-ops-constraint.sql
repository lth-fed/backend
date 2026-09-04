alter table "public"."users_in_purchase_flow" add constraint "lock_matches" CHECK (((lock_id IS NULL) = (locked_at IS NULL))) not valid;

alter table "public"."users_in_purchase_flow" validate constraint "lock_matches";


