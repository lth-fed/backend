//! This contains all functionality specific to admins.
//!
//! Viewing data which normal users also view is handled by their respective functions instead.

use std::collections::{BTreeMap, HashMap};
use std::ops::Deref;
use std::sync::Arc;

use fed_auth_verifier::User;
use minilith_errors::{MinilithEndpointError, MinilithErrorResultExt as _, escape_email_html};
use poem_openapi::payload::{Binary, Json, Response};
use poem_openapi::{Object, OpenApi, param::Path};
use s3::post_policy::PostPolicyExpiration;
use sqlx::PgExecutor;
use sqlx::postgres::types::PgMoney;
use sqlx::types::Uuid;
use sqlx::types::time::OffsetDateTime;
use tracing::error;

use crate::activities::PoemLocation;
use crate::activities::Router as ActivityRouter;
use crate::context::ContextWrapper;
use crate::group::admin::{
    Adminship, check_activity_adminship, check_direct_adminship, check_direct_or_parent_adminship,
    check_ticket_kind_adminship, create_adminship_change, group_admins,
};
use crate::group::member::group_members;
use crate::group::{self, Group};
use crate::ticket::{AvailableAddon, Router as TicketRouter};
use crate::{
    DbInternationalizedString as DIS, InternationalizedString, MinilithErrorOptionExt as _,
    MinilithResult, report, transactions,
};

#[derive(Clone, Debug)]
pub struct Router {
    pub context: ContextWrapper,
}
impl Deref for Router {
    type Target = ContextWrapper;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

#[derive(Object, Debug)]
pub struct ExternalSaleCategory {
    /// Use `"null"` for if it's not alcohol.
    ///
    /// Prefill dropdown with categories from the ticket kinds from this activity.
    /// User should be able to create new categories too.
    pub alcohol_category: String,
    /// In ören.
    pub total: i64,
}
#[derive(Object, Debug)]
pub struct ReportRequest {
    external_sales: Vec<ExternalSaleCategory>,
    external_sale_fees: i64,
}

#[derive(Object)]
struct ObjectUploadAllowanceRequest {
    /// Must not contain the `.`. I.e. `jpg`, `JPEG`, `png` is ok.
    extension: String,
}
/// See `Post File using FormData in Node.js` at
/// <https://www.npmjs.com/package/@aws-sdk/s3-presigned-post>.
#[derive(Object)]
struct ObjectUploadAllowanceResponse {
    url: String,
    fields: HashMap<String, String>,
    dynamic_fields: HashMap<String, String>,

    /// The key you must upload to.
    key: String,
    /// max size in bytes the object can be.
    max_size_bytes: u32,
}

#[derive(Debug, Object)]
struct PutActivity {
    responsible_name: String,
    /// Must use a `mailto:` or `tel:` URI.
    responsible_contact: String,
    creator_id: Uuid,
    title: InternationalizedString,
    description: InternationalizedString,
    location: PoemLocation,
    time_start: OffsetDateTime,
    time_end: OffsetDateTime,
    image_id: Uuid,
    is_hidden: bool,
    is_hidden_for_other_admins: bool,
    max_tickets: i32,
    /// Should not include `creator_id`.
    host_ids: Vec<Uuid>,
}

#[derive(Object)]
struct PutTicketKind {
    activity_id: Uuid,
    name: InternationalizedString,
    price: i64,
    purchasing_available_start: OffsetDateTime,
    purchasing_available_stop: OffsetDateTime,
    max_tickets: i32,
    min_tickets: i32,
    allow_transfer_ticket_start: OffsetDateTime,
    allow_transfer_ticket_stop: OffsetDateTime,
    allow_transfer_ticket_bypass_allowed_groups: bool,
    allowed_group_ids: Vec<Uuid>,
    addons: Vec<AvailableAddon>,
}

#[derive(Debug, Object)]
struct PutGroup {
    path: group::Path,
    name: InternationalizedString,
    description: InternationalizedString,
    limit_membership_visibility: bool,
    logo_id: Uuid,
}

#[derive(Debug, Object)]
struct PutTicketNotification {
    title: InternationalizedString,
    content: InternationalizedString,
    send_at: OffsetDateTime,
}

#[derive(Debug, Object)]
struct TicketNotification {
    kind: String,
    #[oai(flatten)]
    notification: PutTicketNotification,
}

#[derive(Debug, Clone, Object)]
struct AdminPurchasedAddon {
    addon_id: Uuid,
    selected_options: Vec<i32>,
    selected_text: String,
}

#[derive(Debug, Object)]
struct AdminPurchasedTicket {
    id: Uuid,
    ticket_kind_id: Uuid,
    purchaser_id: String,
    owner_id: String,
    transaction_id: Uuid,
    addons: Vec<AdminPurchasedAddon>,
}

#[derive(Debug, Object)]
struct GroupIdRequest {
    group_id: Uuid,
}

#[derive(Debug, Object)]
pub struct CreateAdminship {
    /// The ID of the user to make an admin.
    pub user_id: String,
}

#[derive(Debug)]
pub struct EmailRecipient {
    pub user_id: String,
    pub language: Vec<u8>,
    pub nonce: Vec<u8>,
    pub group_name: DIS,
}

#[derive(Clone, Copy, Debug)]
enum AdminshipEmailChange {
    Created,
    Removed,
}

pub(crate) async fn change_admin_email_recipients(
    db: impl PgExecutor<'_>,
    group_id: Uuid,
    actor_id: &str,
    affected_user_id: &str,
) -> MinilithResult<Vec<EmailRecipient>> {
    sqlx::query_as!(
        EmailRecipient,
        r#"select distinct
            group_adminships.user_id,
            users.language,
            users.nonce,
            target.name as "group_name!: DIS"
        from groups target
        inner join groups admin_group
            on admin_group.id = target.id
            or admin_group.path = target.parent_path
        inner join group_adminships
            on group_adminships.group_id = admin_group.id
        inner join users on users.id = group_adminships.user_id
        where target.id = $1
        and (
            group_adminships.user_id <> $2
            or group_adminships.user_id = $3
        )
        order by group_adminships.user_id"#,
        group_id,
        actor_id,
        affected_user_id,
    )
    .fetch_all(db)
    .await
    .map_err(Into::into)
}

async fn lock_group_for_adminship_change(
    db: impl PgExecutor<'_>,
    group_id: Uuid,
) -> MinilithResult<()> {
    sqlx::query_scalar!("select id from groups where id = $1 for update", group_id)
        .fetch_one(db)
        .await?;
    Ok(())
}

fn email_from_admin_id(user_id: &str) -> &str {
    user_id.strip_prefix("email:").unwrap_or(user_id)
}

async fn send_adminship_emails(
    context: &crate::Context,
    recipients: Vec<EmailRecipient>,
    actor_id: &str,
    affected_user_id: &str,
    change: AdminshipEmailChange,
) -> MinilithResult<()> {
    let Some(email_client) = context.email_client() else {
        return Ok(());
    };

    let actor = email_from_admin_id(actor_id);
    let affected_user = email_from_admin_id(affected_user_id);
    let mut messages: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for recipient in recipients {
        let language = context
            .decrypt_string(recipient.language, &recipient.nonce)
            .wrap_err_encryption("admin email recipient language")?;
        let group_name = recipient.group_name.resolve_intl(&language, "<group>");
        #[allow(clippy::single_match_else, reason = "futureproofing")]
        let (subject, html) = match language.split('-').next() {
            Some("sv") => {
                let action = match change {
                    AdminshipEmailChange::Created => "lagt till",
                    AdminshipEmailChange::Removed => "tagit bort",
                };
                (
                    format!("Administratörerna för {group_name} har uppdaterats"),
                    format!(
                        "<p><strong>{}</strong> har {action} <strong>{}</strong> som administratör för \
                        <strong>{}</strong>.</p>",
                        escape_email_html(actor),
                        escape_email_html(affected_user),
                        escape_email_html(group_name),
                    ),
                )
            }
            _ => {
                let action = match change {
                    AdminshipEmailChange::Created => "added",
                    AdminshipEmailChange::Removed => "removed",
                };
                (
                    format!("Administrators of {group_name} were updated"),
                    format!(
                        "<p><strong>{}</strong> {action} <strong>{}</strong> as an administrator of \
                        <strong>{}</strong>.</p>",
                        escape_email_html(actor),
                        escape_email_html(affected_user),
                        escape_email_html(group_name),
                    ),
                )
            }
        };
        messages
            .entry((subject, html))
            .or_default()
            .push(email_from_admin_id(&recipient.user_id).to_owned());
    }

    for ((subject, html), recipients) in messages {
        email_client
            .send_html(
                "Teknologappen",
                recipients.iter().map(String::as_str),
                &subject,
                html,
            )
            .await
            .wrap_err_internal("failed to send adminship update email")?;
    }
    Ok(())
}

impl Router {
    async fn check_any_direct_adminship(
        &self,
        user_id: &str,
        group_ids: &[Uuid],
    ) -> MinilithResult<()> {
        let allowed = sqlx::query_scalar!(
            r#"select exists (
                select 1 from group_adminships
                where user_id = $1 and group_id = any($2)
            ) as "exists!""#,
            user_id,
            group_ids,
        )
        .fetch_one(&self.db)
        .await?;
        if allowed {
            Ok(())
        } else {
            Err(MinilithEndpointError::bad_frontend_code(
                "must directly administer at least one activity host",
                "",
            ))
        }
    }

    async fn ensure_image_registered(
        &self,
        txn: &mut sqlx_tracing::Transaction<'_, sqlx::Postgres>,
        image_id: Uuid,
    ) -> MinilithResult<()> {
        let registered = sqlx::query_scalar!(
            r#"select exists (select 1 from images where id = $1) as "exists!""#,
            image_id,
        )
        .fetch_one(&mut txn.executor())
        .await?;
        if registered {
            return Ok(());
        }

        let prefix = format!("{image_id}.");
        let pages = self
            .image_bucket()
            .list(prefix.clone(), None)
            .await
            .wrap_err_internal("s3: failed to find uploaded image")?;
        let mut objects = pages
            .into_iter()
            .flat_map(|page| page.contents)
            .filter(|object| {
                object
                    .key
                    .strip_prefix(&prefix)
                    .is_some_and(is_allowed_image_extension)
            });
        let object = objects.next().ok_or_else(|| {
            MinilithEndpointError::bad_frontend_code(
                "IMG_NOT_UPLOADED",
                "image_id does not refer to an uploaded image",
            )
        })?;
        if objects.next().is_some() {
            return Err(MinilithEndpointError::bad_frontend_code(
                "IMG_AMBIGUOUS",
                "multiple uploaded images have the same image_id",
            ));
        }

        let size = i64::try_from(object.size)
            .wrap_err_internal("uploaded image size does not fit in the database")?;
        let url = format!("{}/{}", self.image_public_url(), object.key);
        sqlx::query!(
            r#"insert into images (id, size, url) values ($1, $2, $3)
            on conflict (id) do nothing"#,
            image_id,
            size,
            url,
        )
        .execute(&mut txn.executor())
        .await?;
        Ok(())
    }
}

fn is_allowed_image_extension(extension: &str) -> bool {
    matches!(extension, "jpg" | "jpeg" | "webp" | "png" | "avif")
}

#[OpenApi(prefix_path = "/admin")]
impl Router {
    /// Creates or fully replaces an activity. Existing activities require a
    /// direct adminship in any current host; new activities require one in any
    /// submitted host. The responsible name must not be blank, the contact must
    /// be a `mailto:` or `tel:` URI, the end must follow the start, and the
    /// overall ticket cap cannot be below existing reservations or purchases.
    /// `image_id` must refer to an image uploaded through `/admin/upload-image`.
    #[oai(path = "/activities/:id", method = "put")]
    #[allow(
        clippy::too_many_lines,
        reason = "there's just a lot of shit to do and it's very isolated"
    )]
    async fn put_activity(
        &self,
        user: User,
        Path(id): Path<Uuid>,
        Json(body): Json<PutActivity>,
    ) -> MinilithResult<()> {
        if body.responsible_name.trim().is_empty() {
            return Err(MinilithEndpointError::bad_frontend_code(
                "responsible_name must not be empty",
                "",
            ));
        }
        if !(body.responsible_contact.starts_with("mailto:")
            || body.responsible_contact.starts_with("tel:"))
        {
            return Err(MinilithEndpointError::bad_frontend_code(
                "responsible_contact must start with `mailto:` or `tel:`",
                "",
            ));
        }
        if body.time_end <= body.time_start {
            return Err(MinilithEndpointError::bad_frontend_code(
                "time_end must be after time_start",
                "",
            ));
        }

        let mut host_ids = body.host_ids;
        host_ids.push(body.creator_id);
        host_ids.sort_unstable();
        host_ids.dedup();

        let exists = sqlx::query_scalar!(
            r#"select exists (select 1 from activities where id = $1) as "exists!""#,
            id
        )
        .fetch_one(&self.db)
        .await?;
        if exists {
            check_activity_adminship(&self.db, user.get_id(), id).await?;
        } else {
            self.check_any_direct_adminship(user.get_id(), &host_ids)
                .await?;
        }
        let mut txn = self.db.begin().await?;
        if exists {
            sqlx::query_scalar!("select id from activities where id = $1 for update", id,)
                .fetch_one(&mut txn.executor())
                .await?;
        }
        let currently_reserved = sqlx::query_scalar!(
            r#"select coalesce(sum(reserved_or_purchased_tickets), 0)::int
            from ticket_kinds where activity_id = $1"#,
            id,
        )
        .fetch_one(&mut txn.executor())
        .await?
        .unwrap_or(0);
        if body.max_tickets < currently_reserved {
            return Err(MinilithEndpointError::bad_frontend_code(
                "activity max_tickets is below its reserved or purchased ticket count",
                "",
            ));
        }
        self.ensure_image_registered(&mut txn, body.image_id)
            .await?;

        let name = body
            .location
            .name
            .as_ref()
            .map(InternationalizedString::to_json_value);
        let directions = body
            .location
            .directions
            .as_ref()
            .map(InternationalizedString::to_json_value);
        let (north, east) = body
            .location
            .coordinate_wgs84
            .map_or((None, None), |point| (Some(point.north), Some(point.east)));

        sqlx::query!(
            r#"insert into activities (
                id, responsible_name, responsible_contact, creator_id,
                title, description, location, time_start, time_end, image_id,
                is_hidden, is_hidden_for_other_admins, max_tickets
            )
            values (
                $1, $2, $3, $4, $5, $6,
                row(
                    $7::jsonb,
                    $8::jsonb,
                    case when $9::float8 is null or $10::float8 is null
                        then null else point($9, $10) end,
                    $11
                )::location,
                $12, $13, $14, $15, $16, $17
            )
            on conflict (id) do update set
                responsible_name = excluded.responsible_name,
                responsible_contact = excluded.responsible_contact,
                creator_id = excluded.creator_id,
                title = excluded.title,
                description = excluded.description,
                location = excluded.location,
                time_start = excluded.time_start,
                time_end = excluded.time_end,
                image_id = excluded.image_id,
                is_hidden = excluded.is_hidden,
                is_hidden_for_other_admins = excluded.is_hidden_for_other_admins,
                max_tickets = excluded.max_tickets"#,
            id,
            body.responsible_name,
            body.responsible_contact,
            body.creator_id,
            body.title.to_json_value(),
            body.description.to_json_value(),
            name,
            directions,
            north,
            east,
            body.location.url,
            body.time_start,
            body.time_end,
            body.image_id,
            body.is_hidden,
            body.is_hidden_for_other_admins,
            body.max_tickets,
        )
        .execute(&mut txn.executor())
        .await?;
        sqlx::query!("delete from activity_hosts where activity_id = $1", id)
            .execute(&mut txn.executor())
            .await?;
        sqlx::query!(
            r#"insert into activity_hosts (activity_id, group_id)
            select $1, group_id from unnest($2::uuid[]) as host(group_id)"#,
            id,
            &host_ids,
        )
        .execute(&mut txn.executor())
        .await?;
        txn.commit().await?;

        Ok(())
    }

    /// Builds the activity's bookkeeping report as a PDF. Monetary request
    /// values and all report calculations use integer öre.
    #[oai(path = "/activities/:id/report", method = "post")]
    #[allow(
        clippy::too_many_lines,
        reason = "there's just a lot of shit to do and it's very isolated"
    )]
    async fn report(
        &self,
        user: User,
        Path(id): Path<Uuid>,
        Json(body): Json<ReportRequest>,
    ) -> MinilithResult<Response<Binary<Vec<u8>>>> {
        check_activity_adminship(&self.db, user.get_id(), id).await?;
        let ticket_kinds =
            sqlx::query_scalar!("select id from ticket_kinds where activity_id = $1", id)
                .fetch_all(&self.db)
                .await?;
        let mut tickets = Vec::new();
        let mut kinds = HashMap::with_capacity(ticket_kinds.len());
        let router = TicketRouter {
            context: Arc::clone(&self.context),
        };
        for kind in &ticket_kinds {
            tickets.extend(self.purchased_tickets(user.clone(), Path(*kind)).await?.0);
            kinds.insert(*kind, router.load_ticket_kind_unchecked(*kind).await?);
        }
        // todo: get ticket kinds for prices and such
        let router = ActivityRouter {
            context: Arc::clone(&self.context),
        };
        let activity = router.details(user.clone(), Path(id)).await?.0;

        let user_row = sqlx::query!("select * from users where id = $1", user.get_id())
            .fetch_one(&self.db)
            .await?;

        let lang = self
            .decrypt_string(user_row.language, &user_row.nonce)
            .wrap_err_encryption("admin language")?;

        let language = match lang.split('-').next().unwrap_or(&lang) {
            "sv" => report::Language::Swedish,
            _ => report::Language::English,
        };

        let creator_logo_url = sqlx::query_scalar!(
            "select url from groups
            inner join images on images.id = groups.logo_id
            where groups.id = $1",
            activity.creator_id
        )
        .fetch_one(&self.db)
        .await?;
        let extension = creator_logo_url
            .rsplit('.')
            .next()
            .map(str::to_ascii_lowercase);
        let mut format = match extension.as_deref() {
            Some("jpg" | "jpeg") => Some(report::ImageKind::Jpg),
            Some("png") => Some(report::ImageKind::Png),
            Some("svg") => Some(report::ImageKind::Svg),
            Some("webp") => Some(report::ImageKind::Webp),
            _ => None,
        };
        let image_data = if format.is_some() {
            let path = creator_logo_url
                .rsplit('/')
                .next()
                .unwrap_or(&creator_logo_url);
            match self.image_bucket().get_object(path).await {
                Err(err) => {
                    error!(error = ?err, "s3: Failed to get creator icon");
                    format = None;
                    None
                }
                Ok(resp) => Some(resp.into_bytes()),
            }
        } else {
            None
        };

        let transaction_ids = tickets
            .iter()
            .map(|ticket| ticket.transaction_id)
            .collect::<Vec<_>>();

        let fees = self
            .transactions_post("/v0/info")
            .json(&transactions::InfoRequest { transaction_ids })
            .send()
            .await
            .wrap_err_internal("report: failed to get transaction info")?
            .error_for_status()
            .wrap_err_internal("report: transaction non 2xx")?
            .json::<Vec<transactions::SingleInfoResponse>>()
            .await
            .wrap_err_internal("report: failed to get/parse JSON body")?
            .into_iter()
            .map(|info| info.total_fees)
            .sum();

        let mut per_object = BTreeMap::new();
        let mut per_alcohol_category = BTreeMap::new();
        for ticket in &tickets {
            let kind = kinds.get(&ticket.ticket_kind_id).wrap_err_internal(
                "report: no ticket kind in this activity for the purchased tickets!!",
            )?;
            let kind_name = kind.inner.ticket_kind_name.resolve_intl(&lang, "");

            per_object
                .entry((report::Kind::Ticket, kind_name.to_owned()))
                .or_insert((kind.price, 0))
                .1 += 1;
            *per_alcohol_category.entry("null".to_owned()).or_insert(0) += kind.price;

            for addon in &ticket.addons {
                let addon_data = kind
                    .available_addons
                    .iter()
                    .find(|a_a| a_a.inner.id == addon.addon_id)
                    .wrap_err_internal("report: no addon for purchased ticket in kind")?;
                for option in &addon.selected_options {
                    let option_data = addon_data
                        .options
                        .iter()
                        .find(|opt| opt.idx == *option)
                        .wrap_err_internal("report: no option for purchased ticket in kind")?;
                    let name = format!(
                        "{} - {}",
                        addon_data.inner.name.resolve_intl(&lang, ""),
                        option_data.name.resolve_intl(&lang, "")
                    );

                    per_object
                        .entry((report::Kind::Option, name))
                        .or_insert((option_data.price, 0))
                        .1 += 1;

                    for (category, price) in option_data
                        .bookkeeping_price_categories
                        .iter()
                        .zip(option_data.bookkeeping_prices.iter())
                    {
                        *per_alcohol_category.entry(category.clone()).or_insert(0) += *price;
                    }
                }
            }
        }
        for sale in body.external_sales {
            per_object
                .entry((report::Kind::External, String::new()))
                .or_insert((0, 1))
                .0 += sale.total;
            *per_alcohol_category
                .entry(sale.alcohol_category)
                .or_insert(0) += sale.total;
        }

        let data = report::Data {
            language,
            activity_name: activity.title.resolve_intl(&lang, "<title>").to_owned(),
            creator_name: activity
                .hosts
                .first()
                .map_or("", |host| host.name.resolve_intl(&lang, ""))
                .to_owned(),
            creator_logo_format: format,
            creator_logo_data: image_data,
            // we also have to request the transaction api to get fees
            fees,
            fees_external: body.external_sale_fees,
            per_object: per_object
                .into_iter()
                .map(|((kind, name), (price, number))| report::Object {
                    name,
                    kind,
                    price,
                    number,
                })
                .collect(),
            per_alcohol_category: per_alcohol_category
                .into_iter()
                .map(|(category, amount)| report::AlcoholCategory {
                    name: category,
                    amount,
                })
                .collect(),
        };

        let pdf = report::compile(self.report_typst(), &data)?;

        Ok(Response::new(Binary(pdf)).header(
            "content-disposition",
            format!("attachment; filename=\"activity-report-{id}.pdf\""),
        ))
    }

    /// Lists purchased tickets and addon selections for a ticket kind.
    #[oai(path = "/ticket-kinds/:id/purchased-tickets", method = "get")]
    async fn purchased_tickets(
        &self,
        user: User,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Json<Vec<AdminPurchasedTicket>>> {
        check_ticket_kind_adminship(&self.db, user.get_id(), id).await?;
        let addons = sqlx::query!(
            r#"select
                purchased_ticket_addons.ticket_id,
                purchased_ticket_addons.addon_id,
                purchased_ticket_addons.selected_options,
                purchased_ticket_addons.selected_text
            from purchased_tickets
            inner join purchased_ticket_addons
                on purchased_ticket_addons.ticket_id = purchased_tickets.id
            where purchased_tickets.ticket_kind_id = $1"#,
            id,
        )
        .map(|row| {
            (
                row.ticket_id,
                AdminPurchasedAddon {
                    addon_id: row.addon_id,
                    selected_options: row.selected_options,
                    selected_text: row.selected_text,
                },
            )
        })
        .fetch_all(&self.db)
        .await?
        .into_iter()
        .fold(
            HashMap::<Uuid, Vec<AdminPurchasedAddon>>::new(),
            |mut map, item| {
                map.entry(item.0).or_default().push(item.1);
                map
            },
        );
        let tickets = sqlx::query!(
            r#"select
                purchased_tickets.id,
                purchased_tickets.ticket_kind_id,
                purchased_tickets.purchaser_id,
                purchased_tickets.owner_id,
                purchased_tickets.transaction_id
            from purchased_tickets
            where purchased_tickets.ticket_kind_id = $1
            order by purchased_tickets.id"#,
            id,
        )
        .map(|row| AdminPurchasedTicket {
            id: row.id,
            ticket_kind_id: row.ticket_kind_id,
            purchaser_id: row.purchaser_id,
            owner_id: row.owner_id,
            transaction_id: row.transaction_id,
            addons: addons.get(&row.id).cloned().unwrap_or_default(),
        })
        .fetch_all(&self.db)
        .await?;
        Ok(Json(tickets))
    }

    /// Creates or fully replaces an unpurchased ticket kind, including its
    /// allowlist, addons, and options. After the first purchase, only the
    /// purchasing window and option bookkeeping may change.
    #[oai(path = "/ticket-kinds/:id", method = "put")]
    #[allow(
        clippy::too_many_lines,
        reason = "there's just a lot of shit to do and it's very isolated"
    )]
    async fn put_ticket_kind(
        &self,
        user: User,
        Path(id): Path<Uuid>,
        Json(mut body): Json<PutTicketKind>,
    ) -> MinilithResult<()> {
        // ========
        // Initial checks
        // ========
        let existing_id = sqlx::query_scalar!("select id from ticket_kinds where id = $1", id,)
            .fetch_optional(&self.db)
            .await?;
        if existing_id.is_some() {
            // if this belongs to another activity by coincidence
            check_ticket_kind_adminship(&self.db, user.get_id(), id).await?;
        }
        check_activity_adminship(&self.db, user.get_id(), body.activity_id).await?;

        body.allowed_group_ids.sort_unstable();
        body.allowed_group_ids.dedup();
        let existing = if existing_id.is_some() {
            Some(
                TicketRouter {
                    context: Arc::clone(&self.context),
                }
                .load_ticket_kind_unchecked(id)
                .await?,
            )
        } else {
            None
        };

        let already_reserved = existing
            .as_ref()
            .map_or(0, crate::ticket::Kind::reserved_or_purchased_tickets);
        if body.max_tickets < already_reserved {
            return Err(MinilithEndpointError::bad_frontend_code(
                "ticket kind max_tickets is below its reservation count",
                "",
            ));
        }
        if existing
            .as_ref()
            .is_some_and(|existing| existing.activity_id() != body.activity_id)
        {
            return Err(MinilithEndpointError::bad_frontend_code(
                "cannot change activity_id",
                "",
            ));
        }

        // ========
        // Limited edit if has_been_purchased
        // ========
        if let Some(existing) = existing
            .as_ref()
            .filter(|ticket| ticket.has_been_purchased())
        {
            if !existing.immutable_fields_match(
                body.activity_id,
                body.price,
                &body.allowed_group_ids,
                &body.addons,
            ) {
                return Err(MinilithEndpointError::bad_frontend_code(
                    "a purchased ticket kind's structure and pricing are immutable",
                    "only purchasing availability and option bookkeeping may change",
                ));
            }

            let mut txn = self.db.begin().await?;
            sqlx::query!(
                r#"update ticket_kinds set
                    purchasing_available_start = $2,
                    purchasing_available_stop = $3,
                    name = $4,
                    max_tickets = $5,
                    min_tickets = $6,
                    allow_transfer_ticket_start = $7,
                    allow_transfer_ticket_stop = $8,
                    allow_transfer_ticket_bypass_allowed_groups = $9
                where id = $1"#,
                id,
                body.purchasing_available_start,
                body.purchasing_available_stop,
                body.name.to_json_value(),
                body.max_tickets,
                body.min_tickets,
                body.allow_transfer_ticket_start,
                body.allow_transfer_ticket_stop,
                body.allow_transfer_ticket_bypass_allowed_groups,
            )
            .execute(&mut txn.executor())
            .await?;
            for addon in &body.addons {
                for option in &addon.options {
                    let prices: Vec<PgMoney> = option
                        .bookkeeping_prices
                        .iter()
                        .copied()
                        .map(PgMoney)
                        .collect();
                    sqlx::query!(
                        r#"update ticket_addon_options set
                            bookkeeping_prices = $2,
                            bookkeeping_price_categories = $3
                        where id = $1 and ticket_addon_id = $4"#,
                        option.id,
                        &prices,
                        &option.bookkeeping_price_categories,
                        addon.inner.id,
                    )
                    .execute(&mut txn.executor())
                    .await?;
                }
            }
            txn.commit().await?;
            return Ok(());
        }
        // ========
        // Lock for extensive edit
        // ========
        let mut txn = self.db.begin().await?;
        if existing_id.is_some() {
            let purchased_now = sqlx::query_scalar!(
                r#"select has_been_purchased
                from ticket_kinds
                where id = $1
                for update"#,
                id,
            )
            .fetch_one(&mut txn.executor())
            .await?;
            if purchased_now {
                return Err(MinilithEndpointError::bad_frontend_code(
                    "the ticket kind was purchased while it was being edited",
                    "retry to update only purchasing availability and bookkeeping",
                ));
            }
        }

        // ========
        // Edit ticket kind
        // ========
        sqlx::query!(
            r#"insert into ticket_kinds (
                id, activity_id, name, price,
                purchasing_available_start, purchasing_available_stop,
                max_tickets, min_tickets, reserved_or_purchased_tickets,
                allow_transfer_ticket_start, allow_transfer_ticket_stop,
                allow_transfer_ticket_bypass_allowed_groups,
                has_been_purchased, has_been_released
            ) values (
                $1, $2, $3, $4, $5, $6, $7, $8, 0, $9, $10, $11, false, false
            )
            on conflict (id) do update set
                name = excluded.name,
                price = excluded.price,
                purchasing_available_start = excluded.purchasing_available_start,
                purchasing_available_stop = excluded.purchasing_available_stop,
                max_tickets = excluded.max_tickets,
                min_tickets = excluded.min_tickets,
                allow_transfer_ticket_start = excluded.allow_transfer_ticket_start,
                allow_transfer_ticket_stop = excluded.allow_transfer_ticket_stop,
                allow_transfer_ticket_bypass_allowed_groups =
                    excluded.allow_transfer_ticket_bypass_allowed_groups"#,
            id,
            body.activity_id,
            body.name.to_json_value(),
            PgMoney(body.price),
            body.purchasing_available_start,
            body.purchasing_available_stop,
            body.max_tickets,
            body.min_tickets,
            body.allow_transfer_ticket_start,
            body.allow_transfer_ticket_stop,
            body.allow_transfer_ticket_bypass_allowed_groups,
        )
        .execute(&mut txn.executor())
        .await?;

        // ========
        // Allowed groups
        // ========
        sqlx::query!(
            "delete from ticket_kind_allowed_groups where ticket_kind_id = $1",
            id,
        )
        .execute(&mut txn.executor())
        .await?;
        sqlx::query!(
            r#"insert into ticket_kind_allowed_groups (ticket_kind_id, group_id)
            select $1, group_id from unnest($2::uuid[]) allowed(group_id)"#,
            id,
            &body.allowed_group_ids,
        )
        .execute(&mut txn.executor())
        .await?;

        // ========
        // Options for addons
        // ========
        sqlx::query!(
            r#"delete from ticket_addon_options
            where ticket_addon_id in (
                select id from ticket_addons where ticket_kind_id = $1
            )"#,
            id,
        )
        // ========
        // Addons
        // ========
        .execute(&mut txn.executor())
        .await?;
        sqlx::query!("delete from ticket_addons where ticket_kind_id = $1", id)
            .execute(&mut txn.executor())
            .await?;

        for (addon_idx, addon) in body.addons.iter().enumerate() {
            #[allow(
                clippy::cast_possible_wrap,
                clippy::cast_possible_truncation,
                reason = "we won't have more than i32::MAX addons!"
            )]
            let addon_idx = addon_idx as i32;
            sqlx::query!(
                r#"insert into ticket_addons (
                    id, ticket_kind_id, idx, name,
                    multiple_alternatives, has_text_field, required
                ) values ($1, $2, $3, $4, $5, $6, $7)"#,
                addon.inner.id,
                id,
                addon_idx,
                addon.inner.name.to_json_value(),
                addon.inner.multiple_alternatives,
                addon.inner.has_text_field,
                addon.inner.required,
            )
            .execute(&mut txn.executor())
            .await?;
            for (option_idx, option) in addon.options.iter().enumerate() {
                #[allow(
                    clippy::cast_possible_wrap,
                    clippy::cast_possible_truncation,
                    reason = "we won't have more than i32::MAX options for an addon, i really hope"
                )]
                let option_idx = option_idx as i32;
                let prices: Vec<PgMoney> = option
                    .bookkeeping_prices
                    .iter()
                    .copied()
                    .map(PgMoney)
                    .collect();
                sqlx::query!(
                    r#"insert into ticket_addon_options (
                        id, ticket_addon_id, idx, name, price,
                        bookkeeping_prices, bookkeeping_price_categories
                    ) values ($1, $2, $3, $4, $5, $6, $7)"#,
                    option.id,
                    addon.inner.id,
                    option_idx,
                    option.name.to_json_value(),
                    PgMoney(option.price),
                    &prices,
                    &option.bookkeeping_price_categories,
                )
                .execute(&mut txn.executor())
                .await?;
            }
        }
        txn.commit().await?;
        Ok(())
    }

    /// Creates or replaces a named notification for a ticket kind.
    #[oai(
        path = "/ticket-kinds/:ticket_kind_id/notifications/:kind",
        method = "put"
    )]
    async fn put_ticket_notification(
        &self,
        user: User,
        Path(ticket_kind_id): Path<Uuid>,
        Path(kind): Path<String>,
        Json(body): Json<PutTicketNotification>,
    ) -> MinilithResult<Json<TicketNotification>> {
        check_ticket_kind_adminship(&self.db, user.get_id(), ticket_kind_id).await?;
        let mut txn = self.db.begin().await?;
        sqlx::query_scalar!(
            "select id from ticket_kinds where id = $1 for update",
            ticket_kind_id,
        )
        .fetch_one(&mut txn.executor())
        .await?;
        let notification_id = sqlx::query_scalar!(
            r#"select notification_id from ticket_kind_notifications
            where ticket_kind_id = $1 and id = $2"#,
            ticket_kind_id,
            kind,
        )
        .fetch_optional(&mut txn.executor())
        .await?
        .unwrap_or_else(Uuid::new_v4);
        sqlx::query!(
            r#"insert into notifications (id, title, content, send_at)
            values ($1, $2, $3, $4)
            on conflict (id) do update set
                title = excluded.title,
                content = excluded.content,
                send_at = excluded.send_at"#,
            notification_id,
            body.title.to_json_value(),
            body.content.to_json_value(),
            body.send_at,
        )
        .execute(&mut txn.executor())
        .await?;
        sqlx::query!(
            r#"insert into ticket_kind_notifications
                (id, ticket_kind_id, notification_id)
            values ($1, $2, $3)
            on conflict (id, ticket_kind_id) do update set
                notification_id = excluded.notification_id"#,
            kind,
            ticket_kind_id,
            notification_id,
        )
        .execute(&mut txn.executor())
        .await?;
        txn.commit().await?;
        Ok(Json(TicketNotification {
            kind,
            notification: body,
        }))
    }

    /// Gets a named ticket-kind notification.
    #[oai(
        path = "/ticket-kinds/:ticket_kind_id/notifications/:kind",
        method = "get"
    )]
    async fn get_ticket_notification(
        &self,
        user: User,
        Path(ticket_kind_id): Path<Uuid>,
        Path(kind): Path<String>,
    ) -> MinilithResult<Json<TicketNotification>> {
        check_ticket_kind_adminship(&self.db, user.get_id(), ticket_kind_id).await?;
        let row = sqlx::query!(
            r#"select
                notifications.title as "title!: DIS",
                notifications.content as "content!: DIS",
                notifications.send_at
            from ticket_kind_notifications
            inner join notifications
                on notifications.id = ticket_kind_notifications.notification_id
            where ticket_kind_notifications.ticket_kind_id = $1
            and ticket_kind_notifications.id = $2"#,
            ticket_kind_id,
            kind,
        )
        .fetch_optional(&self.db)
        .await?
        .wrap_err_not_found()?;
        Ok(Json(TicketNotification {
            kind,
            notification: PutTicketNotification {
                title: row.title.0,
                content: row.content.0,
                send_at: row.send_at,
            },
        }))
    }

    /// Lists notifications that are still scheduled for a ticket kind, ordered
    /// by delivery time. Successfully processed notifications are not retained.
    #[oai(path = "/ticket-kinds/:ticket_kind_id/notifications", method = "get")]
    async fn list_ticket_notifications(
        &self,
        user: User,
        Path(ticket_kind_id): Path<Uuid>,
    ) -> MinilithResult<Json<Vec<TicketNotification>>> {
        check_ticket_kind_adminship(&self.db, user.get_id(), ticket_kind_id).await?;
        let notifications = sqlx::query!(
            r#"select
                ticket_kind_notifications.id as kind,
                notifications.title as "title!: DIS",
                notifications.content as "content!: DIS",
                notifications.send_at
            from ticket_kind_notifications
            inner join notifications
                on notifications.id = ticket_kind_notifications.notification_id
            where ticket_kind_notifications.ticket_kind_id = $1
            order by notifications.send_at, ticket_kind_notifications.id"#,
            ticket_kind_id,
        )
        .map(|row| TicketNotification {
            kind: row.kind,
            notification: PutTicketNotification {
                title: row.title.0,
                content: row.content.0,
                send_at: row.send_at,
            },
        })
        .fetch_all(&self.db)
        .await?;
        Ok(Json(notifications))
    }

    /// # Extension
    ///
    /// Only the following extensions are allowed (case doesn't matter):
    ///
    /// - jpg
    /// - jpeg
    /// - webp
    /// - png
    /// - avif
    ///
    /// Notably, no GIF support.
    ///
    /// # Errors
    ///
    /// - You must be admin for some group.
    /// - Extension has to be valid
    /// - internal errors
    #[oai(path = "/upload-image", method = "post")]
    async fn upload_image(
        &self,
        user: User,
        Json(mut body): Json<ObjectUploadAllowanceRequest>,
    ) -> MinilithResult<Json<ObjectUploadAllowanceResponse>> {
        // only admins can upload
        group::admin::check_has_any_adminship(&self.db, user.get_id()).await?;

        body.extension.make_ascii_lowercase();
        if !is_allowed_image_extension(&body.extension) {
            return Err(MinilithEndpointError::bad_frontend_code(
                "invalid extension",
                "",
            ));
        }

        let uuid = Uuid::new_v4();
        let max_size_bytes = 1024u32 * 1024 * 4;
        let key = format!("{uuid}.{}", body.extension);
        let policy = s3::post_policy::PostPolicy::new(PostPolicyExpiration::ExpiresIn(60 * 5))
            .condition(
                s3::PostPolicyField::Key,
                s3::PostPolicyValue::Exact(key.as_str().into()),
            )
            .wrap_err_internal("s3: bad key condition")?
            .condition(
                s3::PostPolicyField::ContentLengthRange,
                s3::PostPolicyValue::Range(0, max_size_bytes),
            )
            .wrap_err_internal("s3: bad content length condition")?
            .condition(
                s3::PostPolicyField::ContentType,
                s3::PostPolicyValue::StartsWith("image/".into()),
            )
            .wrap_err_internal("s3: bad content type condition")?
            .sign(self.image_bucket().clone().into())
            .await
            .wrap_err_internal("s3: failed to sign")?;

        let mut dynamic_fields = policy.dynamic_fields;
        // `content-length-range` is a POST-policy condition, not a form field:
        // https://docs.aws.amazon.com/AmazonS3/latest/developerguide/sigv4-HTTPPOSTConstructPolicy.html
        // rust-s3 0.37.2 nevertheless includes Range conditions in `dynamic_fields`:
        // https://github.com/durch/rust-s3/blob/v0.37.2/src/post_policy.rs#L135-L148
        dynamic_fields.remove("content-length-range");

        Ok(Json(ObjectUploadAllowanceResponse {
            // The signed policy covers the form fields. The browser sends them
            // through the public proxy rather than the internal S3 hostname.
            url: self.image_public_url().to_owned(),
            fields: policy.fields,
            dynamic_fields,
            key,
            max_size_bytes,
        }))
    }

    // ==========
    // GROUPS
    // ==========

    /// Creates or fully replaces a group. Existing groups require a direct
    /// adminship; new groups require a direct adminship in their parent and
    /// grant the creator a direct adminship. Moving an existing group also
    /// requires a direct adminship in the new parent.
    /// `logo_id` must refer to an image uploaded through `/admin/upload-image`.
    #[oai(path = "/groups/:id", method = "put")]
    #[allow(
        clippy::too_many_lines,
        reason = "there's just a lot of shit to do and it's very isolated"
    )]
    async fn put_group(
        &self,
        user: User,
        Path(id): Path<Uuid>,
        Json(body): Json<PutGroup>,
    ) -> MinilithResult<()> {
        let mut txn = self.db.begin().await?;
        let old_path = sqlx::query_scalar!(
            r#"select path as "path!: group::Path" from groups where id = $1 for update"#,
            id,
        )
        .fetch_optional(&mut txn.executor())
        .await?;
        let is_new = old_path.is_none();
        let new_parent = body
            .path
            .parent()
            .wrap_err_bad_frontend("a group must have a parent")?;

        if let Some(old_path) = &old_path {
            check_direct_adminship(&mut txn.executor(), user.get_id(), id).await?;
            let old_parent = old_path
                .parent()
                .wrap_err_bad_frontend("the root group cannot be edited")?;
            if old_parent.to_string() != new_parent.to_string() {
                let new_parent_id = group::id_by_path(&mut txn.executor(), &new_parent)
                    .await?
                    .wrap_err_bad_frontend("new parent group does not exist")?;
                check_direct_adminship(&mut txn.executor(), user.get_id(), new_parent_id).await?;
            }
        } else {
            let parent_id = group::id_by_path(&mut txn.executor(), &new_parent)
                .await?
                .wrap_err_bad_frontend("parent group does not exist")?;
            check_direct_adminship(&mut txn.executor(), user.get_id(), parent_id).await?;
        }
        self.ensure_image_registered(&mut txn, body.logo_id).await?;

        let result = if let Some(old_path) = old_path {
            if old_path == body.path {
                sqlx::query!(
                    r#"update groups set
                        name = $2,
                        description = $3,
                        limit_membership_visibility = $4,
                        logo_id = $5
                    where id = $1"#,
                    id,
                    body.name.to_json_value(),
                    body.description.to_json_value(),
                    body.limit_membership_visibility,
                    body.logo_id,
                )
                .execute(&mut txn.executor())
                .await
            } else {
                sqlx::query!(
                    r#"
                update groups
                set path = $2::ltree || subpath(path, nlevel($3::ltree)),
                    name = case when id = $1 then $4 else name end,
                    description = case when id = $1 then $5 else description end,
                    limit_membership_visibility = case
                        when id = $1 then $6
                        else limit_membership_visibility
                    end,
                    logo_id = case when id = $1 then $7 else logo_id end
                where path <@ $3::ltree"#,
                    id,
                    body.path.0,
                    old_path.0,
                    body.name.to_json_value(),
                    body.description.to_json_value(),
                    body.limit_membership_visibility,
                    body.logo_id,
                )
                .execute(&mut txn.executor())
                .await
            }
        } else {
            sqlx::query!(
                r#"insert into groups (
                    id, path, name, description,
                    limit_membership_visibility, logo_id
                ) values ($1, $2, $3, $4, $5, $6)"#,
                id,
                body.path.0,
                body.name.to_json_value(),
                body.description.to_json_value(),
                body.limit_membership_visibility,
                body.logo_id,
            )
            .execute(&mut txn.executor())
            .await
        };
        result.map_err(|error| match error {
            sqlx::Error::Database(ref db_error) => match db_error.constraint() {
                Some("groups_path_key" | "groups_pkey") => {
                    MinilithEndpointError::bad_frontend_code(
                        "GRP_EXISTS",
                        "a group with the same path or ID already exists",
                    )
                }
                Some("groups_parent_path_fkey") => MinilithEndpointError::bad_frontend_code(
                    "GRP_NULL_PARENT",
                    format!("no parent with path `{new_parent}` exists"),
                ),
                _ => MinilithEndpointError::db(error),
            },
            other_error => MinilithEndpointError::db(other_error),
        })?;

        if is_new {
            create_adminship_change(&mut txn.executor(), user.get_id(), id).await?;
        }

        txn.commit().await?;
        Ok(())
    }

    /// Hides a directly administered group.
    #[oai(path = "/groups/:group_id", method = "delete")]
    async fn hide_group(&self, user: User, Path(group_id): Path<Uuid>) -> MinilithResult<()> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        sqlx::query!("update groups set deleted = true where id = $1", group_id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    /// Lists pending requests for a directly administered group.
    #[oai(path = "/groups/:group_id/member-requests", method = "get")]
    async fn membership_requests(
        &self,
        user: User,
        Path(group_id): Path<Uuid>,
    ) -> MinilithResult<Json<Vec<String>>> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        let users = sqlx::query_scalar!(
            "select member_id from group_member_requests where group_id = $1 order by member_id",
            group_id,
        )
        .fetch_all(&self.db)
        .await?;
        Ok(Json(users))
    }

    /// Accepts a pending membership request.
    #[oai(path = "/groups/:group_id/member-requests/:member_id", method = "put")]
    async fn accept_membership_request(
        &self,
        user: User,
        Path(group_id): Path<Uuid>,
        Path(member_id): Path<String>,
    ) -> MinilithResult<()> {
        let mut txn = self.db.begin().await?;
        check_direct_adminship(&mut txn.executor(), user.get_id(), group_id).await?;
        let accepted = sqlx::query_scalar!(
            r#"with removed as (
                delete from group_member_requests
                where group_id = $1 and member_id = $2
                returning member_id, group_id
            ), inserted as (
                insert into group_memberships (user_id, group_id)
                select member_id, group_id from removed
                on conflict do nothing
            )
            select member_id from removed"#,
            group_id,
            member_id,
        )
        .fetch_optional(&mut txn.executor())
        .await?;
        if accepted.is_none() {
            return Err(MinilithEndpointError::not_found());
        }
        txn.commit().await?;
        Ok(())
    }

    /// List all members of a group. To do it, you need to be an admin of the
    /// group.
    ///
    /// # Errors
    ///
    /// - none
    #[oai(path = "/groups/:group_id/members", method = "get")]
    async fn list_members(
        &self,
        user: User,
        Path(group_id): Path<Uuid>,
    ) -> MinilithResult<Json<Vec<String>>> {
        let mut txn = self.db.begin().await?;
        check_direct_adminship(&mut txn.executor(), user.get_id(), group_id).await?;
        let members = group_members(&mut txn.executor(), group_id).await?;

        Ok(Json(members))
    }

    /// Adds a direct member to a directly administered group.
    #[oai(path = "/groups/:group_id/members/:member_id", method = "put")]
    async fn add_member(
        &self,
        user: User,
        Path(group_id): Path<Uuid>,
        Path(member_id): Path<String>,
    ) -> MinilithResult<()> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        sqlx::query!(
            r#"insert into group_memberships (user_id, group_id)
            values ($1, $2) on conflict do nothing"#,
            member_id,
            group_id,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Removes a direct member. Any adminship in the same group is independent
    /// and remains in place.
    #[oai(path = "/groups/:group_id/members/:member_id", method = "delete")]
    async fn remove_member(
        &self,
        user: User,
        Path(group_id): Path<Uuid>,
        Path(member_id): Path<String>,
    ) -> MinilithResult<()> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        if check_direct_adminship(&self.db, &member_id, group_id)
            .await
            .is_ok()
        {
            return Err(MinilithEndpointError::bad_frontend_code(
                "tried to remove the membership of an admin",
                "",
            ));
        }
        sqlx::query!(
            "delete from group_memberships where user_id = $1 and group_id = $2",
            member_id,
            group_id,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// List all admins of a group. To do it, you need to be an admin of the
    /// group.
    ///
    /// # Errors
    ///
    /// - none
    #[oai(path = "/groups/:group_id/admins", method = "get")]
    async fn list_admins(
        &self,
        user: User,
        Path(group_id): Path<Uuid>,
    ) -> MinilithResult<Json<Vec<String>>> {
        let mut txn = self.db.begin().await?;
        check_direct_or_parent_adminship(&mut txn.executor(), user.get_id(), group_id).await?;
        let admins = group_admins(&mut txn.executor(), group_id).await?;

        Ok(Json(admins))
    }

    /// Create an adminship for a user in a group.
    ///
    /// The user performing this action must be a literal super-admin, meaning
    /// they must at least be an administrator of the parent group.
    ///
    /// # Errors
    ///
    /// - the user must be admin of the parent of this group
    #[oai(path = "/groups/:group_id/admins", method = "post")]
    async fn create_adminship(
        &self,
        user: User,
        Path(group_id): Path<Uuid>,
        Json(create_adminship): Json<CreateAdminship>,
    ) -> MinilithResult<Json<Adminship>> {
        let CreateAdminship { user_id } = create_adminship;

        let mut txn = self.db.begin().await?;

        check_direct_or_parent_adminship(&mut txn.executor(), user.get_id(), group_id).await?;
        lock_group_for_adminship_change(&mut txn.executor(), group_id).await?;
        let (adminship, created) =
            create_adminship_change(&mut txn.executor(), &user_id, group_id).await?;
        let recipients = if created && self.email_client().is_some() {
            change_admin_email_recipients(&mut txn.executor(), group_id, user.get_id(), &user_id)
                .await?
        } else {
            Vec::new()
        };
        if created {
            send_adminship_emails(
                &self.context,
                recipients,
                user.get_id(),
                &user_id,
                AdminshipEmailChange::Created,
            )
            .await?;
        }
        txn.commit().await?;

        Ok(Json(adminship))
    }

    /// Removes an adminship & membership for a user in a group.
    ///
    /// The user performing this action must be a literal super-admin, meaning
    /// they must at least be an administrator of the parent group.
    ///
    /// # Errors
    ///
    /// - root must have no admins
    /// - the user must be admin of the parent of this group
    #[oai(path = "/groups/:group_id/admins/:user_id", method = "delete")]
    async fn remove_adminship(
        &self,
        user: User,
        Path(group_id): Path<Uuid>,
        Path(user_id): Path<String>,
    ) -> MinilithResult<()> {
        let mut txn = self.db.begin().await?;

        check_direct_or_parent_adminship(&mut txn.executor(), user.get_id(), group_id).await?;
        lock_group_for_adminship_change(&mut txn.executor(), group_id).await?;
        let recipients = if self.email_client().is_some() {
            change_admin_email_recipients(&mut txn.executor(), group_id, user.get_id(), &user_id)
                .await?
        } else {
            Vec::new()
        };
        let deleted = sqlx::query!(
            "delete from group_adminships where user_id = $1 and group_id = $2",
            user_id,
            group_id
        )
        .execute(&mut txn.executor())
        .await?;
        sqlx::query!(
            "delete from group_memberships where user_id = $1 and group_id = $2",
            &user_id,
            group_id
        )
        .execute(&mut txn.executor())
        .await?;
        if deleted.rows_affected() == 1 {
            send_adminship_emails(
                &self.context,
                recipients,
                user.get_id(),
                &user_id,
                AdminshipEmailChange::Removed,
            )
            .await?;
        }
        txn.commit().await?;

        Ok(())
    }

    /// Lists groups whose direct members may request membership in this group.
    #[oai(path = "/groups/:group_id/joiner-groups", method = "get")]
    async fn list_joiner_groups(
        &self,
        user: User,
        Path(group_id): Path<Uuid>,
    ) -> MinilithResult<Json<Vec<Group>>> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        let groups = sqlx::query_as!(
            Group,
            r#"select
                groups.id, groups.path,
                groups.limit_membership_visibility,
                groups.name as "name!: DIS",
                groups.description as "description!: DIS",
                groups.deleted,
                logo.id as logo_id,
                logo.url as logo_url
            from groups_ask_to_join
            inner join groups on groups.id = groups_ask_to_join.joiner_id
            inner join images logo on logo.id = groups.logo_id
            where groups_ask_to_join.target_id = $1
            order by groups.path"#,
            group_id,
        )
        .fetch_all(&self.db)
        .await?;
        Ok(Json(groups))
    }

    /// Allows direct members of another group to request membership.
    #[oai(path = "/groups/:group_id/joiner-groups", method = "put")]
    async fn add_joiner_group(
        &self,
        user: User,
        Path(group_id): Path<Uuid>,
        Json(body): Json<GroupIdRequest>,
    ) -> MinilithResult<()> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        sqlx::query!(
            r#"insert into groups_ask_to_join (target_id, joiner_id)
            values ($1, $2) on conflict do nothing"#,
            group_id,
            body.group_id,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Removes a group from the join-request allowlist.
    #[oai(path = "/groups/:group_id/joiner-groups/:joiner_id", method = "delete")]
    async fn remove_joiner_group(
        &self,
        user: User,
        Path(group_id): Path<Uuid>,
        Path(joiner_id): Path<Uuid>,
    ) -> MinilithResult<()> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        sqlx::query!(
            "delete from groups_ask_to_join where target_id = $1 and joiner_id = $2",
            group_id,
            joiner_id,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Lists groups whose direct admins may view activities hosted by this
    /// group.
    #[oai(path = "/groups/:group_id/activity-admin-groups", method = "get")]
    async fn list_activity_admin_groups(
        &self,
        user: User,
        Path(group_id): Path<Uuid>,
    ) -> MinilithResult<Json<Vec<Group>>> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        let groups = sqlx::query_as!(
            Group,
            r#"select
                groups.id, groups.path,
                groups.limit_membership_visibility,
                groups.name as "name!: DIS",
                groups.description as "description!: DIS",
                groups.deleted,
                logo.id as logo_id,
                logo.url as logo_url
            from allow_admins_from_group_view_activities allowed
            inner join groups on groups.id = allowed.access_group_id
            inner join images logo on logo.id = groups.logo_id
            where allowed.host_group_id = $1
            order by groups.path"#,
            group_id,
        )
        .fetch_all(&self.db)
        .await?;
        Ok(Json(groups))
    }

    /// Grants another group's direct admins access to this group's activities.
    #[oai(path = "/groups/:group_id/activity-admin-groups", method = "put")]
    async fn add_activity_admin_group(
        &self,
        user: User,
        Path(group_id): Path<Uuid>,
        Json(body): Json<GroupIdRequest>,
    ) -> MinilithResult<()> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        sqlx::query!(
            r#"insert into allow_admins_from_group_view_activities
                (host_group_id, access_group_id)
            values ($1, $2) on conflict do nothing"#,
            group_id,
            body.group_id,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Revokes another group's activity access.
    #[oai(
        path = "/groups/:group_id/activity-admin-groups/:access_group_id",
        method = "delete"
    )]
    async fn remove_activity_admin_group(
        &self,
        user: User,
        Path(group_id): Path<Uuid>,
        Path(access_group_id): Path<Uuid>,
    ) -> MinilithResult<()> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        sqlx::query!(
            r#"delete from allow_admins_from_group_view_activities
            where host_group_id = $1 and access_group_id = $2"#,
            group_id,
            access_group_id,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }
}
