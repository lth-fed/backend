alter table "public"."fortnox_tax_accounts" drop constraint "fortnox_tax_accounts_pkey";

alter table "public"."fortnox_voucher_jobs" drop constraint "fortnox_voucher_jobs_pkey";

drop index if exists "public"."fortnox_tax_accounts_pkey";

drop index if exists "public"."fortnox_voucher_jobs_pending";

drop index if exists "public"."fortnox_voucher_jobs_pkey";

alter table "public"."client_ids" drop constraint "client_ids_fortnox_bank_account_check";

alter table "public"."client_ids" drop constraint "fortnox_configuration_complete";

alter table "public"."fortnox_tax_accounts" drop constraint "fortnox_tax_accounts_client_id_fkey";

alter table "public"."fortnox_tax_accounts" drop constraint "fortnox_tax_accounts_revenue_account_check";

alter table "public"."fortnox_tax_accounts" drop constraint "fortnox_tax_accounts_vat_account_check";

alter table "public"."fortnox_tax_accounts" drop constraint "fortnox_tax_accounts_vat_basis_points_check";

alter table "public"."fortnox_tax_accounts" drop constraint "fortnox_vat_account_required";

alter table "public"."fortnox_voucher_jobs" drop constraint "fortnox_voucher_identity_complete";

alter table "public"."fortnox_voucher_jobs" drop constraint "fortnox_voucher_jobs_attempts_check";

alter table "public"."fortnox_voucher_jobs" drop constraint "fortnox_voucher_jobs_state_check";

alter table "public"."fortnox_voucher_jobs" drop constraint "fortnox_voucher_jobs_transaction_id_fkey";

drop table "public"."fortnox_tax_accounts";

drop table "public"."fortnox_voucher_jobs";

alter table "public"."client_ids" drop column "fortnox_bank_account";

alter table "public"."client_ids" drop column "fortnox_client_id";

alter table "public"."client_ids" drop column "fortnox_client_secret";

alter table "public"."client_ids" drop column "fortnox_tenant_id";

alter table "public"."client_ids" drop column "fortnox_voucher_series";


