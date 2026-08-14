# Fortnox bookkeeping integration

## Decision

Creating one Fortnox voucher for each paid Swish transaction is viable. The generated PDF contains
the transaction ID, payment reference, date, merchant identity, wares, VAT, and totals, and is
attached to the voucher. BFN says Swish sales are booked like other sales and require equivalent
supporting documentation. BFN also permits a joint voucher backed by a sales summary, so one voucher
per payment is a traceability choice rather than a legal requirement.

For higher Swish volume, a daily aggregate voucher plus the Swish bank specification would create
fewer Fortnox records and be easier to reconcile. The current per-transaction approach matches the
requested bank-transaction model and keeps failures isolated. An accountant should approve the
voucher series and account mappings before enabling it.

The integration uses Fortnox's current service-account OAuth client-credentials flow. A Fortnox
Accounting licence and consent for the `bookkeeping` and `connectfile` scopes are required. Fortnox
supports the implemented sequence: create voucher, upload a PDF to `Inbox_v`, then create a voucher
file connection.

Sources:

- [BFN: Swishbetalningar](https://www.bfn.se/fragor-och-svar/bokforing/swishbetalningar/)
- [Fortnox: vouchers and file attachments](https://www.fortnox.se/developer/guides-and-good-to-know/best-practices/vouchers)
- [Fortnox: service-account client credentials](https://www.fortnox.se/developer/authorization/get-access-token-using-client-credentials)
- [Fortnox: API scopes](https://www.fortnox.se/developer/guides-and-good-to-know/scopes)

## Enabling a client

Fortnox is opt-in through `client_ids`. Complete all five settings in one update; partial
configuration is rejected by the database.

```sql
update client_ids
set fortnox_client_id = '<integration client id>',
    fortnox_client_secret = '<integration client secret>',
    fortnox_tenant_id = '<Fortnox tenant id>',
    fortnox_voucher_series = '<accountant-approved series>',
    fortnox_bank_account = <accountant-approved bank account>
where client_id = '<transactions client id>';
```

Add every VAT rate that the client can send. Rates are basis points: `2500` is 25%, `1200` is 12%,
`600` is 6%, and `0` is VAT exempt. Account numbers below are intentionally placeholders because
they depend on the organisation's chart of accounts.

```sql
insert into fortnox_tax_accounts
    (client_id, vat_basis_points, revenue_account, vat_account)
values
    ('<transactions client id>', 2500, <revenue account>, <output VAT account>),
    ('<transactions client id>', 0, <VAT-exempt revenue account>, null);
```

Only Swish payments confirmed after Fortnox is enabled are queued. Historical payments are not
backfilled automatically because they may already be booked.

## Failure and duplicate handling

The payment update and Fortnox job are committed atomically. Duplicate Swish callbacks reuse the
same job. Workers claim jobs with `skip locked`, so multiple service instances can run concurrently.
Transient failures before a voucher is created retry with backoff.

Fortnox does not document an idempotency key for voucher creation. If a request might have reached
Fortnox but its response was lost, the job moves to `manual_review` and sends a level-2 alert instead
of risking a duplicate. Search Fortnox voucher comments for the transaction UUID before changing the
job. Once the actual voucher/file identity has been recorded, setting the job back to `pending` safely
resumes the remaining attachment steps.

Swish transaction fees are not deducted in these vouchers: the bank receipt is the gross sale. Book
provider fees separately from the bank or provider fee statement.

## Stripe payout plan

The Stripe integration should be payout-driven, not a blind Monday sum. Stripe explicitly recommends
its payout reconciliation report for automatic payout schedules and a clearing-account model. The
itemised report identifies the balance transactions included in each payout and includes gross, fee,
and net values.

Recommended implementation:

1. Add Stripe payout webhook events and persist each payout ID, currency, amount, arrival date, and
   processing state. Add the Teknologappen transaction UUID as Stripe metadata and persist each
   charge's balance-transaction ID so report rows can be matched exactly.
2. For each paid automatic payout, request Stripe's payout reconciliation summary and itemised report.
   Store the original report file as the immutable supporting document.
3. Reconcile exact integer minor units: all matched gross sales, refunds, disputes, adjustments, and
   fees must produce Stripe's payout net amount. Once bank imports are available, also match the bank
   credit by payout reference and amount.
4. On any missing/duplicate transaction, currency mismatch, or non-zero amount difference, stop the
   voucher and emit `alert(AlertLevel::L2, ...)`. Retry briefly for delayed Stripe data before alerting.
5. Prefer a Stripe clearing account: book sales to the clearing account on their transaction date,
   then let the payout voucher debit the bank account, debit fees, and credit the clearing account.
   Attach the itemised payout report to that voucher. A single weekly voucher containing sales, VAT,
   fees, and bank settlement is simpler but should only be used if the accountant confirms that the
   organisation's bookkeeping method permits recognising the accumulated sales on payout day.
6. Reuse the Fortnox job stages and conservative ambiguous-failure handling. Key the job by Stripe
   payout ID so Monday scheduling and webhook retries cannot create duplicate vouchers.

Stripe sources:

- [Choosing a Stripe reconciliation report](https://docs.stripe.com/reports/select-a-report)
- [Payout reconciliation](https://docs.stripe.com/reports/payout-reconciliation)
- [Stripe Reports API](https://docs.stripe.com/reports/api)
