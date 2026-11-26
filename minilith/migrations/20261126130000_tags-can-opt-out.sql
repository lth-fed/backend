-- Tags everyone gets, such as "Important", can't be opted out of.
alter table "public"."activity_tags" add column "can_opt_out" boolean not null default true;
alter table "public"."news_tags" add column "can_opt_out" boolean not null default true;
