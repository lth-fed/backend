alter table "public"."ticket_reservation_addons" drop constraint "ticket_reservation_addons_ticket_id_fkey";

set check_function_bodies = off;

CREATE OR REPLACE FUNCTION public.ensure_purchase_flow_has_state()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
declare
    checked_user text;
begin
    if tg_op = 'DELETE' then
        checked_user := old.user_id;
    else
        checked_user := new.user_id;
    end if;
    if exists (
        select 1 from users_in_purchase_flow flow
        where flow.user_id = checked_user
        and (
            num_nonnulls(release_queue, reservation_queue, reservation) != 1
            or (release_queue is not null) != exists (
                select 1 from ticket_release_queuers
                where user_id = flow.user_id and ticket_kind_id = flow.ticket_kind_id
            )
            or (reservation_queue is not null) != exists (
                select 1 from ticket_reservation_queuers
                where user_id = flow.user_id and ticket_kind_id = flow.ticket_kind_id
            )
            or (reservation is not null) != exists (
                select 1 from ticket_reservations
                where user_id = flow.user_id and ticket_kind_id = flow.ticket_kind_id
            )
        )
    ) then
        raise exception 'purchase flow for user % has inconsistent state', checked_user;
    end if;
    return null;
end;
$function$
;

create table "public"."users_in_purchase_flow" (
    "user_id" text not null,
    "ticket_kind_id" uuid not null,
    "release_queue" text,
    "reservation_queue" text,
    "reservation" text
);


alter table "public"."ticket_reservations" alter column "user_id" set not null;

CREATE CONSTRAINT TRIGGER release_queuer_has_matching_flow AFTER INSERT OR DELETE OR UPDATE ON public.ticket_release_queuers DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION ensure_purchase_flow_has_state();

CREATE CONSTRAINT TRIGGER reservation_queuer_has_matching_flow AFTER INSERT OR DELETE OR UPDATE ON public.ticket_reservation_queuers DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION ensure_purchase_flow_has_state();

CREATE CONSTRAINT TRIGGER reservation_has_matching_flow AFTER INSERT OR DELETE OR UPDATE ON public.ticket_reservations DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION ensure_purchase_flow_has_state();

CREATE CONSTRAINT TRIGGER purchase_flow_has_state AFTER INSERT OR UPDATE ON public.users_in_purchase_flow DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION ensure_purchase_flow_has_state();

CREATE UNIQUE INDEX users_in_purchase_flow_pkey ON public.users_in_purchase_flow USING btree (user_id);

CREATE UNIQUE INDEX users_in_purchase_flow_user_id_ticket_kind_id_key ON public.users_in_purchase_flow USING btree (user_id, ticket_kind_id);

alter table "public"."users_in_purchase_flow" add constraint "users_in_purchase_flow_pkey" PRIMARY KEY using index "users_in_purchase_flow_pkey";

alter table "public"."ticket_release_queuers" add constraint "ticket_release_queuers_user_id_ticket_kind_id_fkey" FOREIGN KEY ("user_id", "ticket_kind_id") REFERENCES "public"."users_in_purchase_flow"("user_id", "ticket_kind_id") NOT VALID;

alter table "public"."ticket_release_queuers" validate constraint "ticket_release_queuers_user_id_ticket_kind_id_fkey";

alter table "public"."ticket_reservation_queuers" add constraint "ticket_reservation_queuers_user_id_ticket_kind_id_fkey" FOREIGN KEY ("user_id", "ticket_kind_id") REFERENCES "public"."users_in_purchase_flow"("user_id", "ticket_kind_id") NOT VALID;

alter table "public"."ticket_reservation_queuers" validate constraint "ticket_reservation_queuers_user_id_ticket_kind_id_fkey";

alter table "public"."ticket_reservations" add constraint "ticket_reservations_user_id_ticket_kind_id_fkey" FOREIGN KEY ("user_id", "ticket_kind_id") REFERENCES "public"."users_in_purchase_flow"("user_id", "ticket_kind_id") NOT VALID;

alter table "public"."ticket_reservations" validate constraint "ticket_reservations_user_id_ticket_kind_id_fkey";

alter table "public"."users_in_purchase_flow" add constraint "purchase_flow_has_at_most_one_state" CHECK ((num_nonnulls(release_queue, reservation_queue, reservation) <= 1)) not valid;

alter table "public"."users_in_purchase_flow" validate constraint "purchase_flow_has_at_most_one_state";

alter table "public"."users_in_purchase_flow" add constraint "purchase_flow_references_own_user" CHECK ((((release_queue IS NULL) OR (release_queue = user_id)) AND ((reservation_queue IS NULL) OR (reservation_queue = user_id)) AND ((reservation IS NULL) OR (reservation = user_id)))) not valid;

alter table "public"."users_in_purchase_flow" validate constraint "purchase_flow_references_own_user";

alter table "public"."users_in_purchase_flow" add constraint "users_in_purchase_flow_ticket_kind_id_fkey" FOREIGN KEY ("ticket_kind_id") REFERENCES "public"."ticket_kinds"("id") NOT VALID;

alter table "public"."users_in_purchase_flow" validate constraint "users_in_purchase_flow_ticket_kind_id_fkey";

alter table "public"."users_in_purchase_flow" add constraint "users_in_purchase_flow_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") NOT VALID;

alter table "public"."users_in_purchase_flow" validate constraint "users_in_purchase_flow_user_id_fkey";

alter table "public"."users_in_purchase_flow" add constraint "users_in_purchase_flow_user_id_ticket_kind_id_key" UNIQUE using index "users_in_purchase_flow_user_id_ticket_kind_id_key";

alter table "public"."ticket_reservation_addons" add constraint "ticket_reservation_addons_ticket_id_fkey" FOREIGN KEY ("ticket_id") REFERENCES "public"."ticket_reservations"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."ticket_reservation_addons" validate constraint "ticket_reservation_addons_ticket_id_fkey";

