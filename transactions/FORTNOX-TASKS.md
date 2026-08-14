You are to implement a light fortnox integration. The goal is to have a voucher per bank transaction to make everything accounted for. This only happens with swish. Swish gives the money directly. Stripe each monday. With swish the voucher would be the receipt. It'd be nice with a stripe voucher too, that'd need to be uploaded each monday, with the accumulated transactions. If the stripe payout and amount from us don't line up, alert level 2 (see minilith-errors/lib.rs).

A fortnox example in IS is in `minilith/src/example-fortnox.js`

- [x] investigate if this is viable (see `FORTNOX.md`)
- [x] is there a better way for the bookkeeping? (see `FORTNOX.md`)
- [x] interface with the fortnox api, use client_ids, upload receipt for each swish transaction
- [x] make a plan for how to do it with stripe (see `FORTNOX.md`)
