-- Reservation queues remember which ticket kind a queuer originally wanted. At a release, the
-- leftovers of the non-base kinds are pushed down into the activity's base ticket queue; a freed
-- non-base unit is offered back to the highest placed queuer that originally wanted it.

alter table "public"."product_reservation_queuers"
    add column "original_product_id" uuid;

alter table "public"."product_reservation_queuers"
    add constraint "product_reservation_queuers_original_product_id_fkey"
    FOREIGN KEY ("original_product_id") REFERENCES "public"."products"("id") NOT VALID;

alter table "public"."product_reservation_queuers"
    validate constraint "product_reservation_queuers_original_product_id_fkey";

create index "product_queuer_original_product" on "public"."product_reservation_queuers"
    using btree ("original_product_id");
