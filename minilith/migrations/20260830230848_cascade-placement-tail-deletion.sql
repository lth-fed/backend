alter table "public"."ticket_reservation_placement_tails" drop constraint "ticket_reservation_placement_tails_ticket_kind_id_fkey";

alter table "public"."ticket_reservation_placement_tails" add constraint "ticket_reservation_placement_tails_ticket_kind_id_fkey" FOREIGN KEY ("ticket_kind_id") REFERENCES "public"."ticket_kinds"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."ticket_reservation_placement_tails" validate constraint "ticket_reservation_placement_tails_ticket_kind_id_fkey";


