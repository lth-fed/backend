CREATE UNIQUE INDEX ticket_reservations_transaction_id_key ON public.ticket_reservations USING btree (transaction_id);

alter table "public"."ticket_reservations" add constraint "ticket_reservations_transaction_id_key" UNIQUE using index "ticket_reservations_transaction_id_key";


