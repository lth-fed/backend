alter table "public"."session_validated_users" drop constraint "session_validated_users_session_id_fkey";

create table "public"."admin_tokens" (
    "key" uuid not null,
    "client_id" text not null,
    "name" text not null
);


create table "public"."password_associations" (
    "user_id" text not null,
    "client_id" text not null,
    "password_salt" bytea not null,
    "password_hash" bytea not null
);


alter table "public"."api_keys" add column "email" text;

update "public"."api_keys" set "email" = 'macapar@esek.se' where "email" is null;

alter table "public"."api_keys" alter column "email" set not null;

alter table "public"."session_validated_users" drop column "lth_guild";

CREATE UNIQUE INDEX admin_tokens_pkey ON public.admin_tokens USING btree (key);

CREATE UNIQUE INDEX password_associations_pkey ON public.password_associations USING btree (user_id, client_id);

alter table "public"."admin_tokens" add constraint "admin_tokens_pkey" PRIMARY KEY using index "admin_tokens_pkey";

alter table "public"."password_associations" add constraint "password_associations_pkey" PRIMARY KEY using index "password_associations_pkey";

alter table "public"."session_validated_users" add constraint "session_validated_users_session_id_fkey" FOREIGN KEY ("session_id") REFERENCES "public"."sessions"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."session_validated_users" validate constraint "session_validated_users_session_id_fkey";


