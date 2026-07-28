create table "public"."api_tokens" (
    "token" text not null,
    "client_id" text not null,
    "callback_url_v1" text not null
);


create table "public"."client_ids" (
    "client_id" text not null,
    "swish_cert" text not null,
    "swish_key" text not null,
    "swish_number" text not null,
    "stripe_secret" text,
    "name" text not null,
    "email" text not null,
    "address" text not null,
    "organization_number" text not null,
    "svg_icon" text
);


create table "public"."stripe_checkouts" (
    "transaction_id" uuid not null,
    "stripe_id" text not null
);


create table "public"."stripe_customers" (
    "customer_id" text not null,
    "stripe_id" text not null
);


create table "public"."transaction_wares" (
    "idx" integer not null,
    "transaction_id" uuid not null,
    "name" text not null,
    "amount" money not null,
    "currency" text not null default 'SEK'::text,
    "tax" double precision not null
);


create table "public"."transactions" (
    "id" uuid not null,
    "customer_id" text,
    "client_id" text not null,
    "callback_url_v1" text not null,
    "created" timestamp with time zone not null default now(),
    "payment_reference" text,
    "timeout" timestamp with time zone not null,
    "provider" provider not null,
    "total_transaction_fee" money not null,
    "total_transaction_fee_currency" text not null default 'SEK'::text,
    "callback_identifier" uuid not null,
    "refund_reference" text,
    "refund_id" uuid,
    "refund_callback_identifier" uuid
);


CREATE UNIQUE INDEX api_tokens_pkey ON public.api_tokens USING btree (token);

CREATE UNIQUE INDEX client_ids_pkey ON public.client_ids USING btree (client_id);

CREATE UNIQUE INDEX stripe_checkouts_pkey ON public.stripe_checkouts USING btree (transaction_id);

CREATE INDEX stripe_checkouts_stripe_id ON public.stripe_checkouts USING hash (stripe_id);

CREATE UNIQUE INDEX stripe_customers_pkey ON public.stripe_customers USING btree (customer_id);

CREATE UNIQUE INDEX transaction_wares_pkey ON public.transaction_wares USING btree (idx, transaction_id);

CREATE INDEX transaction_wares_transaction_id ON public.transaction_wares USING hash (transaction_id);

CREATE UNIQUE INDEX transactions_pkey ON public.transactions USING btree (id);

CREATE INDEX transactions_timeout ON public.transactions USING btree (timeout);

alter table "public"."api_tokens" add constraint "api_tokens_pkey" PRIMARY KEY using index "api_tokens_pkey";

alter table "public"."client_ids" add constraint "client_ids_pkey" PRIMARY KEY using index "client_ids_pkey";

alter table "public"."stripe_checkouts" add constraint "stripe_checkouts_pkey" PRIMARY KEY using index "stripe_checkouts_pkey";

alter table "public"."stripe_customers" add constraint "stripe_customers_pkey" PRIMARY KEY using index "stripe_customers_pkey";

alter table "public"."transaction_wares" add constraint "transaction_wares_pkey" PRIMARY KEY using index "transaction_wares_pkey";

alter table "public"."transactions" add constraint "transactions_pkey" PRIMARY KEY using index "transactions_pkey";

alter table "public"."api_tokens" add constraint "api_tokens_client_id_fkey" FOREIGN KEY ("client_id") REFERENCES "public"."client_ids"("client_id") NOT VALID;

alter table "public"."api_tokens" validate constraint "api_tokens_client_id_fkey";

alter table "public"."stripe_checkouts" add constraint "stripe_checkouts_transaction_id_fkey" FOREIGN KEY ("transaction_id") REFERENCES "public"."transactions"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."stripe_checkouts" validate constraint "stripe_checkouts_transaction_id_fkey";

alter table "public"."transaction_wares" add constraint "transaction_wares_currency_check" CHECK ((currency = 'SEK'::text)) not valid;

alter table "public"."transaction_wares" validate constraint "transaction_wares_currency_check";

alter table "public"."transaction_wares" add constraint "transaction_wares_tax_check" CHECK ((tax >= (1.0)::double precision)) not valid;

alter table "public"."transaction_wares" validate constraint "transaction_wares_tax_check";

alter table "public"."transaction_wares" add constraint "transaction_wares_transaction_id_fkey" FOREIGN KEY ("transaction_id") REFERENCES "public"."transactions"("id") NOT VALID;

alter table "public"."transaction_wares" validate constraint "transaction_wares_transaction_id_fkey";

alter table "public"."transactions" add constraint "transactions_client_id_fkey" FOREIGN KEY ("client_id") REFERENCES "public"."client_ids"("client_id") NOT VALID;

alter table "public"."transactions" validate constraint "transactions_client_id_fkey";

alter table "public"."transactions" add constraint "transactions_total_transaction_fee_currency_check" CHECK ((total_transaction_fee_currency = 'SEK'::text)) not valid;

alter table "public"."transactions" validate constraint "transactions_total_transaction_fee_currency_check";


