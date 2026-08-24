The Swish values in `./test-data/auth.sql` are taken from:
<https://developer.swish.nu/documentation/environments#certificates>

Use the following to generate the required private key: `openssl genpkey -algorithm ed25519 -outform der | base64`.

## Recover deleted ticket transactions

Deploy the fixed transaction timeout cleanup, or stop the transactions service, before recovering.
From `backend/`, set `FED_DATABASE_URL` and `TRANSACTIONS_DATABASE_URL`, then preview the recovery:

```sh
DATABASE_URL="$TRANSACTIONS_DATABASE_URL" cargo run -p transactions \
  --bin recover_transactions -- --client-id esek
```

The command is a dry run unless `--apply` is passed. The default `all` provider mode writes nothing
if any nonzero payment is ambiguous. Zero-total transactions are recovered as `free`, which is safe
because Minilith always forces zero-total ticket purchases to the free provider. The recovered
payment reference is `free` and the fee is zero.

Free payments have no provider-side copy from which to recover the original ware language or time.
If translated ware names differ, pass `--language sv` or `--language en`. Their `created` and
`timeout` values use the recovery time and `paid_at` remains null, matching the normal free-payment
shape as closely as the remaining data permits.

`--provider both` checks only Swish and Stripe. `--provider free`, `--provider swish`, or
`--provider stripe` recovers only that selection and skips the rest. Use `--transaction-id <uuid>`
to limit the scope. Stripe Checkout Sessions whose charge has any refunded amount are excluded from
matching, so a refunded duplicate does not make the remaining paid session ambiguous.
