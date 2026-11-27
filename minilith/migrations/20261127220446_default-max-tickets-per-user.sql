-- New activities allow one ticket per person. Activities created before the limit was enforced
-- were written with the old "unlimited" default (i32::MAX), which was never meant as a choice.
alter table "public"."activities" alter column "max_tickets_per_user" set default 1;

update "public"."activities" set max_tickets_per_user = 1 where max_tickets_per_user = 2147483647;
