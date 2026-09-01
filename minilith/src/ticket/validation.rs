#[derive(Object)]
struct ValidateActivity {
    id: Uuid,
    title: IS,
    description: IS,
    time_start: OffsetDateTime,
    time_end: OffsetDateTime,
    image_url: String,
}

/// The frontend has to encode / decode the QR with both these datapoints, maybe through
/// `<id>.<time>` or JSON.
#[derive(Object)]
struct ValidateRequest {
    purchased_ticket_id: Uuid,
    created_at: OffsetDateTime,
}
#[derive(Object)]
struct Validation {
    at: OffsetDateTime,
}
#[derive(Object)]
struct ValidateResponse {
    verified: bool,
    ticket_kind_name: IS,
    owner_id: Option<String>,
    owner_name: Option<String>,
    has_been_transfered: bool,
    purchaser_name: Option<String>,
    previous_verifications: Vec<Validation>,
    purchased_addons: Vec<PurchasedAddon>,
}
impl ValidateResponse {
    pub fn not_valid() -> Self {
        Self {
            verified: false,
            ticket_kind_name: IS::empty(),
            owner_id: None,
            owner_name: None,
            has_been_transfered: false,
            purchaser_name: None,
            previous_verifications: vec![],
            purchased_addons: vec![],
        }
    }
}

async fn validatable_activities(&self, auth: User) -> MinilithResult<Vec<ValidateActivity>> {
    sqlx::query!(
        "select title as \"title!: DIS\", description as \"description!: DIS\",
            a.id, url, time_start, time_end
            from activity_verifiers
            inner join activities a on a.id = activity_verifiers.activity_id
            inner join images on images.id = a.image_id
            where user_id = $1
            and a.time_end > now() - '24 hours'::interval",
        auth.get_id()
    )
    .map(|row| ValidateActivity {
        id: row.id,
        title: row.title.0,
        description: row.description.0,
        time_start: row.time_start,
        time_end: row.time_end,
        image_url: row.url,
    })
    .fetch_all(&self.db)
    .await
    .map_err(Into::into)
    .map(Json)
}
async fn validate(&self, auth: User, body: ValidateRequest) -> MinilithResult<ValidateResponse> {
    let now = OffsetDateTime::now_utc();
    // TODO(frontend-hack: 25/08/2026): some people are insane and don't have accurate time on their phone, so we have to
    // increase leeway before we implement server side time adjustment
    let leeway = 5 * time::Duration::MINUTE;
    if body.created_at < now.saturating_sub(leeway) || body.created_at > now.saturating_add(leeway)
    {
        return Ok(Json(ValidateResponse::not_valid()));
    }
    let Some(row) = sqlx::query!(
        "select owner_id, purchaser_id, owner.name as oname, purchaser.name as pname,
            ticket_kind_id, kind.name as \"ticket_kind_name: DIS\"
            from purchased_tickets 
            inner join ticket_kinds kind on kind.id = purchased_tickets.ticket_kind_id
            inner join activity_verifiers on activity_verifiers.activity_id = kind.activity_id
            inner join users owner on owner.id = owner_id
            inner join users purchaser on purchaser.id = purchaser_id
            where purchased_tickets.id = $1 
                and activity_verifiers.user_id = $2",
        body.purchased_ticket_id,
        auth.get_id()
    )
    .fetch_optional(&self.db)
    .await?
    else {
        return Ok(Json(ValidateResponse::not_valid()));
    };
    let previous_verifications = sqlx::query!(
        "select timestamp from purchased_ticket_validations where purchased_ticket_id = $1",
        body.purchased_ticket_id
    )
    .map(|row| Validation { at: row.timestamp })
    .fetch_all(&self.db)
    .await?;
    let mut available_options: HashMap<Uuid, Vec<AddonOption>> = sqlx::query!(
        "select opt.id, opt.idx, opt.name as \"name!: DIS\", opt.price,
            bookkeeping_prices as \"bp!: Vec<i64>\", bookkeeping_price_categories,
            add.id as add_id
            from ticket_kinds kind
            inner join ticket_addons add on add.ticket_kind_id = kind.id 
            inner join ticket_addon_options opt on opt.ticket_addon_id = add.id
            where kind.id = $1",
        row.ticket_kind_id
    )
    .map(|row| {
        (
            row.add_id,
            AddonOption {
                id: row.id,
                idx: row.idx,
                name: row.name.0,
                price: row.price.0,
                bookkeeping_prices: row.bp,
                bookkeeping_price_categories: row.bookkeeping_price_categories,
            },
        )
    })
    .fetch_all(&self.db)
    .await?
    .into_iter()
    .fold(HashMap::new(), |mut map, (addon_id, option)| {
        map.entry(addon_id).or_default().push(option);
        map
    });
    let purchased_addons: Vec<PurchasedAddon> = sqlx::query!(
        r#"select
                purchased_ticket_addons.ticket_id as "ticket_id",
                ticket_addons.id as "addon_id",
                ticket_addons.name as "addon_name: DIS",
                ticket_addons.multiple_alternatives as "multiple_alternatives",
                ticket_addons.has_text_field as "has_text_field",
                ticket_addons.required as "required",
                purchased_ticket_addons.selected_options as "selected_options",
                purchased_ticket_addons.selected_text as "selected_text"
            from purchased_ticket_addons
            inner join ticket_addons on
                ticket_addons.id = purchased_ticket_addons.addon_id
            where purchased_ticket_addons.ticket_id = $1
            order by ticket_addons.idx
            "#,
        body.purchased_ticket_id
    )
    .map(|row| PurchasedAddon {
        inner: Addon {
            id: row.addon_id,
            name: row.addon_name.0,
            multiple_alternatives: row.multiple_alternatives,
            has_text_field: row.has_text_field,
            required: row.required,
        },
        selected_options: row.selected_options,
        selected_text: row.selected_text,
        options: available_options.remove(&row.addon_id).unwrap_or_default(),
    })
    .fetch_all(&self.context.db)
    .await?;
    sqlx::query!(
        "insert into purchased_ticket_validations (id, purchased_ticket_id)
            values ($1, $2)",
        Uuid::new_v4(),
        body.purchased_ticket_id
    )
    .execute(&self.db)
    .await?;
    Ok(Json(ValidateResponse {
        verified: true,
        ticket_kind_name: row.ticket_kind_name.0,
        has_been_transfered: row.owner_id != row.purchaser_id,
        owner_id: Some(row.owner_id),
        owner_name: Some(
            self.decrypt_string(row.oname)
                .wrap_err_encryption("validate name")?,
        ),
        purchaser_name: Some(
            self.decrypt_string(row.pname)
                .wrap_err_encryption("validate purchaser name")?,
        ),
        previous_verifications,
        purchased_addons,
    }))
}
