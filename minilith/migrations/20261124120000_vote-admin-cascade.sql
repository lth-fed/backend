alter table "public"."vote_meeting_admins" drop constraint "vote_meeting_admins_user_id_fkey";

alter table "public"."vote_alternatives" drop constraint "vote_alternatives_round_id_fkey";

alter table "public"."vote_cast_votes" drop constraint "vote_cast_votes_client_id_fkey";

alter table "public"."vote_cast_votes" drop constraint "vote_cast_votes_round_id_fkey";

alter table "public"."vote_meeting_admins" drop constraint "vote_meeting_admins_meeting_id_fkey";

alter table "public"."vote_registered_clients" drop constraint "vote_registered_clients_meeting_id_fkey";

alter table "public"."vote_registered_users" drop constraint "vote_registered_users_meeting_id_fkey";

alter table "public"."vote_rounds" drop constraint "vote_rounds_meeting_id_fkey";

alter table "public"."vote_alternatives" add constraint "vote_alternatives_round_id_fkey" FOREIGN KEY ("round_id") REFERENCES "public"."vote_rounds"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."vote_alternatives" validate constraint "vote_alternatives_round_id_fkey";

alter table "public"."vote_cast_votes" add constraint "vote_cast_votes_client_id_fkey" FOREIGN KEY ("client_id") REFERENCES "public"."vote_registered_clients"("client_id") ON DELETE CASCADE NOT VALID;

alter table "public"."vote_cast_votes" validate constraint "vote_cast_votes_client_id_fkey";

alter table "public"."vote_cast_votes" add constraint "vote_cast_votes_round_id_fkey" FOREIGN KEY ("round_id") REFERENCES "public"."vote_rounds"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."vote_cast_votes" validate constraint "vote_cast_votes_round_id_fkey";

alter table "public"."vote_meeting_admins" add constraint "vote_meeting_admins_meeting_id_fkey" FOREIGN KEY ("meeting_id") REFERENCES "public"."vote_meetings"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."vote_meeting_admins" validate constraint "vote_meeting_admins_meeting_id_fkey";

alter table "public"."vote_registered_clients" add constraint "vote_registered_clients_meeting_id_fkey" FOREIGN KEY ("meeting_id") REFERENCES "public"."vote_meetings"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."vote_registered_clients" validate constraint "vote_registered_clients_meeting_id_fkey";

alter table "public"."vote_registered_users" add constraint "vote_registered_users_meeting_id_fkey" FOREIGN KEY ("meeting_id") REFERENCES "public"."vote_meetings"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."vote_registered_users" validate constraint "vote_registered_users_meeting_id_fkey";

alter table "public"."vote_rounds" add constraint "vote_rounds_meeting_id_fkey" FOREIGN KEY ("meeting_id") REFERENCES "public"."vote_meetings"("id") ON DELETE CASCADE NOT VALID;

alter table "public"."vote_rounds" validate constraint "vote_rounds_meeting_id_fkey";


