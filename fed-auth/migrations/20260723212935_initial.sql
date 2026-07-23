create table "public"."api_keys" (
    "key" uuid not null,
    "user_id" text not null,
    "client_id" text not null
);


create table "public"."auth_refresh_tokens" (
    "refresh_token" uuid not null,
    "client_id" text not null,
    "user_id" text not null,
    "nonce" text,
    "auth_time" timestamp with time zone not null
);


CREATE UNIQUE INDEX api_keys_pkey ON public.api_keys USING btree (key);

CREATE UNIQUE INDEX auth_refresh_tokens_pkey ON public.auth_refresh_tokens USING btree (refresh_token, client_id);

alter table "public"."api_keys" add constraint "api_keys_pkey" PRIMARY KEY using index "api_keys_pkey";

alter table "public"."auth_refresh_tokens" add constraint "auth_refresh_tokens_pkey" PRIMARY KEY using index "auth_refresh_tokens_pkey";


