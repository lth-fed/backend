drop index if exists "public"."notifications_send_time";

alter table "public"."notifications" add column "sent" boolean not null default false;

CREATE INDEX notifications_unsent_send_time ON public.notifications USING btree (send_at) WHERE (sent = false);


