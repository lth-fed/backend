alter table "public"."group_adminships" drop constraint "group_adminships_user_id_fkey";

alter table "public"."group_memberships" drop constraint "group_memberships_user_id_fkey";

create table "public"."image_uploads" (
    "key" text not null
);


CREATE UNIQUE INDEX image_uploads_pkey ON public.image_uploads USING btree (key);

alter table "public"."image_uploads" add constraint "image_uploads_pkey" PRIMARY KEY using index "image_uploads_pkey";


