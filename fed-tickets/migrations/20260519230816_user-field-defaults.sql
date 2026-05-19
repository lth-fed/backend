alter table "public"."users" alter column "creation" set default now();

alter table "public"."users" alter column "latest_refresh" set default now();


