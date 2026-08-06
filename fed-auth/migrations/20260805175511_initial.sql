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


create table "public"."email_token_holding" (
    "id" uuid not null,
    "email" text not null,
    "code" text not null,
    "created" timestamp with time zone not null default now()
);


create table "public"."saml2_request_id_cache" (
    "id" text not null,
    "created" timestamp with time zone not null default now()
);


create table "public"."session_validated_users" (
    "session_id" text not null,
    "sub" text not null,
    "email" text,
    "full_name" text,
    "lth_guild" text
);


create table "public"."sessions" (
    "id" text not null,
    "redirect_uri" text not null,
    "client_id" text not null,
    "state" text,
    "nonce" text,
    "callback_url_v1" text,
    "code_challenge" text not null,
    "datasharing_confirmed" boolean not null default false,
    "redirect_requires_datasharing" boolean not null,
    "created" timestamp with time zone not null default now()
);


CREATE UNIQUE INDEX api_keys_pkey ON public.api_keys USING btree (key);

CREATE UNIQUE INDEX auth_refresh_tokens_pkey ON public.auth_refresh_tokens USING btree (refresh_token, client_id);

CREATE UNIQUE INDEX email_token_holding_pkey ON public.email_token_holding USING btree (id);

CREATE UNIQUE INDEX saml2_request_id_cache_pkey ON public.saml2_request_id_cache USING btree (id);

CREATE UNIQUE INDEX session_validated_users_pkey ON public.session_validated_users USING btree (session_id);

CREATE UNIQUE INDEX sessions_pkey ON public.sessions USING btree (id);

alter table "public"."api_keys" add constraint "api_keys_pkey" PRIMARY KEY using index "api_keys_pkey";

alter table "public"."auth_refresh_tokens" add constraint "auth_refresh_tokens_pkey" PRIMARY KEY using index "auth_refresh_tokens_pkey";

alter table "public"."email_token_holding" add constraint "email_token_holding_pkey" PRIMARY KEY using index "email_token_holding_pkey";

alter table "public"."saml2_request_id_cache" add constraint "saml2_request_id_cache_pkey" PRIMARY KEY using index "saml2_request_id_cache_pkey";

alter table "public"."session_validated_users" add constraint "session_validated_users_pkey" PRIMARY KEY using index "session_validated_users_pkey";

alter table "public"."sessions" add constraint "sessions_pkey" PRIMARY KEY using index "sessions_pkey";

alter table "public"."session_validated_users" add constraint "session_validated_users_session_id_fkey" FOREIGN KEY ("session_id") REFERENCES "public"."sessions"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."session_validated_users" validate constraint "session_validated_users_session_id_fkey";


