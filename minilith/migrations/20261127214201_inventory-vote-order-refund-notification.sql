-- Three additions: the `inventory` stock record kind, the `refund` notification kind and an
-- explicit display order for vote alternatives. The enums are recreated (instead of appended to)
-- so the value order matches `schema/14-stock.sql` and `schema/20-notifications.sql`.

alter type "public"."notification_kind" rename to "notification_kind__old_version_to_be_dropped";

create type "public"."notification_kind" as enum ('activity', 'news', 'ticket_release', 'transfer', 'reservation', 'refund');

alter table "public"."push_notifications"
    alter column kind type "public"."notification_kind" using kind::text::"public"."notification_kind";

drop type "public"."notification_kind__old_version_to_be_dropped";

alter type "public"."stock_record_kind" rename to "stock_record_kind__old_version_to_be_dropped";

create type "public"."stock_record_kind" as enum ('purchase', 'refund', 'sales', 'sales-refund', 'take-out', 'put-back', 'balance', 'inventory');

alter table "public"."stock_transaction_records"
    alter column kind type "public"."stock_record_kind" using kind::text::"public"."stock_record_kind";

drop type "public"."stock_record_kind__old_version_to_be_dropped";

-- existing rounds had no explicit order; number them deterministically instead of failing on the
-- not-null constraint
alter table "public"."vote_alternatives" add column "idx" integer;

update "public"."vote_alternatives" alternative
set idx = ordered.ord
from (
    select id, row_number() over (partition by round_id order by id) - 1 as ord
    from "public"."vote_alternatives"
) ordered
where ordered.id = alternative.id;

alter table "public"."vote_alternatives" alter column "idx" set not null;
