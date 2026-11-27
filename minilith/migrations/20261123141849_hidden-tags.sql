alter table "public"."activity_tags" add column "hidden" boolean not null default false;

alter table "public"."news_tags" add column "hidden" boolean not null default false;
