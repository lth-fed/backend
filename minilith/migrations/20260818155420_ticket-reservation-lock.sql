alter table "public"."ticket_reservation_queuers" drop constraint "ticket_reservation_queuers_placement_check";

create table "public"."ticket_reservation_placement_tails" (
    "ticket_kind_id" uuid not null,
    "placement_tail" integer not null
);


CREATE UNIQUE INDEX ticket_reservation_placement_tails_pkey ON public.ticket_reservation_placement_tails USING btree (ticket_kind_id);

alter table "public"."ticket_reservation_placement_tails" add constraint "ticket_reservation_placement_tails_pkey" PRIMARY KEY using index "ticket_reservation_placement_tails_pkey";

alter table "public"."ticket_reservation_placement_tails" add constraint "ticket_reservation_placement_tails_ticket_kind_id_fkey" FOREIGN KEY ("ticket_kind_id") REFERENCES "public"."ticket_kinds"("id") NOT VALID;

alter table "public"."ticket_reservation_placement_tails" validate constraint "ticket_reservation_placement_tails_ticket_kind_id_fkey";

alter table "public"."ticket_reservation_queuers" add constraint "ticket_reservation_queuers_placement_check" CHECK ((placement > 0)) not valid;

alter table "public"."ticket_reservation_queuers" validate constraint "ticket_reservation_queuers_placement_check";


