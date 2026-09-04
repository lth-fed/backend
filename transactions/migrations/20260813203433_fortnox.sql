create table "public"."fortnox_tax_accounts" (
    "client_id" text not null,
    "vat_basis_points" integer not null,
    "revenue_account" integer not null,
    "vat_account" integer
);


create table "public"."fortnox_voucher_jobs" (
    "transaction_id" uuid not null,
    "state" text not null default 'pending'::text,
    "attempts" integer not null default 0,
    "next_attempt_at" timestamp with time zone not null default now(),
    "started_at" timestamp with time zone,
    "last_error" text,
    "voucher_series" text,
    "voucher_number" integer,
    "voucher_year" integer,
    "file_id" text,
    "completed_at" timestamp with time zone
);


alter table "public"."client_ids" add column "fortnox_bank_account" integer;

alter table "public"."client_ids" add column "fortnox_client_id" text;

alter table "public"."client_ids" add column "fortnox_client_secret" text;

alter table "public"."client_ids" add column "fortnox_tenant_id" text;

alter table "public"."client_ids" add column "fortnox_voucher_series" text;

alter table "public"."transactions" add column "paid_at" timestamp with time zone;

CREATE UNIQUE INDEX fortnox_tax_accounts_pkey ON public.fortnox_tax_accounts USING btree (client_id, vat_basis_points);

CREATE INDEX fortnox_voucher_jobs_pending ON public.fortnox_voucher_jobs USING btree (next_attempt_at) WHERE (state = 'pending'::text);

CREATE UNIQUE INDEX fortnox_voucher_jobs_pkey ON public.fortnox_voucher_jobs USING btree (transaction_id);

alter table "public"."fortnox_tax_accounts" add constraint "fortnox_tax_accounts_pkey" PRIMARY KEY using index "fortnox_tax_accounts_pkey";

alter table "public"."fortnox_voucher_jobs" add constraint "fortnox_voucher_jobs_pkey" PRIMARY KEY using index "fortnox_voucher_jobs_pkey";

alter table "public"."client_ids" add constraint "client_ids_fortnox_bank_account_check" CHECK ((fortnox_bank_account > 0)) not valid;

alter table "public"."client_ids" validate constraint "client_ids_fortnox_bank_account_check";

alter table "public"."client_ids" add constraint "fortnox_configuration_complete" CHECK ((num_nonnulls(fortnox_client_id, fortnox_client_secret, fortnox_tenant_id, fortnox_voucher_series, fortnox_bank_account) = ANY (ARRAY[0, 5]))) not valid;

alter table "public"."client_ids" validate constraint "fortnox_configuration_complete";

alter table "public"."fortnox_tax_accounts" add constraint "fortnox_tax_accounts_client_id_fkey" FOREIGN KEY ("client_id") REFERENCES "public"."client_ids"("client_id") ON DELETE CASCADE NOT VALID;

alter table "public"."fortnox_tax_accounts" validate constraint "fortnox_tax_accounts_client_id_fkey";

alter table "public"."fortnox_tax_accounts" add constraint "fortnox_tax_accounts_revenue_account_check" CHECK ((revenue_account > 0)) not valid;

alter table "public"."fortnox_tax_accounts" validate constraint "fortnox_tax_accounts_revenue_account_check";

alter table "public"."fortnox_tax_accounts" add constraint "fortnox_tax_accounts_vat_account_check" CHECK ((vat_account > 0)) not valid;

alter table "public"."fortnox_tax_accounts" validate constraint "fortnox_tax_accounts_vat_account_check";

alter table "public"."fortnox_tax_accounts" add constraint "fortnox_tax_accounts_vat_basis_points_check" CHECK ((vat_basis_points >= 0)) not valid;

alter table "public"."fortnox_tax_accounts" validate constraint "fortnox_tax_accounts_vat_basis_points_check";

alter table "public"."fortnox_tax_accounts" add constraint "fortnox_vat_account_required" CHECK ((((vat_basis_points = 0) AND (vat_account IS NULL)) OR ((vat_basis_points > 0) AND (vat_account IS NOT NULL)))) not valid;

alter table "public"."fortnox_tax_accounts" validate constraint "fortnox_vat_account_required";

alter table "public"."fortnox_voucher_jobs" add constraint "fortnox_voucher_identity_complete" CHECK ((num_nonnulls(voucher_series, voucher_number, voucher_year) = ANY (ARRAY[0, 3]))) not valid;

alter table "public"."fortnox_voucher_jobs" validate constraint "fortnox_voucher_identity_complete";

alter table "public"."fortnox_voucher_jobs" add constraint "fortnox_voucher_jobs_attempts_check" CHECK ((attempts >= 0)) not valid;

alter table "public"."fortnox_voucher_jobs" validate constraint "fortnox_voucher_jobs_attempts_check";

alter table "public"."fortnox_voucher_jobs" add constraint "fortnox_voucher_jobs_state_check" CHECK ((state = ANY (ARRAY['pending'::text, 'processing'::text, 'manual_review'::text, 'completed'::text]))) not valid;

alter table "public"."fortnox_voucher_jobs" validate constraint "fortnox_voucher_jobs_state_check";

alter table "public"."fortnox_voucher_jobs" add constraint "fortnox_voucher_jobs_transaction_id_fkey" FOREIGN KEY ("transaction_id") REFERENCES "public"."transactions"("id") NOT VALID;

alter table "public"."fortnox_voucher_jobs" validate constraint "fortnox_voucher_jobs_transaction_id_fkey";


