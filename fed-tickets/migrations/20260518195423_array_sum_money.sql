set check_function_bodies = off;

CREATE OR REPLACE FUNCTION public.array_sum_money(money[])
 RETURNS money
 LANGUAGE sql
 IMMUTABLE STRICT
AS $function$SELECT sum(e) FROM unnest($1) AS a(e)$function$
;
