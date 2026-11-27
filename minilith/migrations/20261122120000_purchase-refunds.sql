-- Refunds used to invalidate a purchase by moving its owner to the sentinel `refunded:`, which is
-- why `owner_id` is not a FK to users. Refunds now only set `refunded_id`, but the FK stays
-- dropped so rows refunded before that change remain valid.
alter table purchases drop constraint purchases_owner_id_fkey;
