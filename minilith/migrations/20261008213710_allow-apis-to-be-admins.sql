alter table "public"."group_adminships" drop constraint "group_adminships_email_account";

alter table "public"."group_adminships" add constraint "group_adminships_email_account" CHECK (((user_id ~~ 'email:%'::text) OR (user_id ~~ 'api:%'::text))) not valid;

alter table "public"."group_adminships" validate constraint "group_adminships_email_account";


