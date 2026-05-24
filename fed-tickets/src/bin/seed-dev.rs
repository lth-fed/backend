//! Seed the dev database with the same activities, groups, and ticket
//! kinds the frontend's `lib/api/*.ts` mock data uses, so a freshly
//! migrated DB renders the demo screens against real backend data.
//!
//! Idempotent: every insert is `on conflict do nothing` so re-running
//! is safe. Run with `cargo run --bin seed-dev` (from `fed-tickets/`).
//!
//! Adding new fixtures: extend the data tables at the top of `seed()`
//! and re-run. The activities reference user/group/image rows by id, so
//! keep the constants consistent if you add more.

#![allow(
    clippy::expect_used,
    clippy::min_ident_chars,
    clippy::too_many_lines,
    clippy::doc_markdown,
    reason = "fixture script: panicking on bad static input is fine, the data tables are intentionally large, and short loop bindings (`a`, `k`) read more cleanly than full words in tight scopes."
)]

use std::collections::HashMap;

use color_eyre::eyre::WrapErr as _;
use fed_tickets::Context;
use sqlx::postgres::types::{PgLTree, PgMoney};
use sqlx::types::Json;
use sqlx::types::time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

const LOGO_IMG: Uuid = Uuid::from_u128(0x7c315a13_eff7_4268_89b9_5e072611ea21);

/// Shared image id every seed group + activity points at, to avoid
/// re-uploading the same placeholder per row. Matches the one
/// `populate_tlth` in `user.rs` already inserts.
async fn seed_image(ctx: &Context) -> color_eyre::Result<()> {
    sqlx::query!(
        "insert into images (id, size, url) values ($1, 0, 'https://icelk.dev/logo.png') on conflict do nothing",
        LOGO_IMG
    )
    .execute(&ctx.db)
    .await
    .wrap_err("seed image")?;
    Ok(())
}

/// `tlth` and one node per known guild code. Names are filled with both
/// English and Swedish strings so `pickI18n` on the frontend renders
/// real labels (the empty `'{}'::jsonb` seed the auth flow leaves behind
/// otherwise produces blank rows).
async fn seed_groups(ctx: &Context) -> color_eyre::Result<()> {
    let groups: &[(&str, &str, &str)] = &[
        ("tlth", "TLTH", "Teknologkåren vid Lunds Tekniska Högskola"),
        ("tlth.f", "F-sektionen", "Fysiksektionen"),
        ("tlth.e", "E-sektionen", "E-sektionen vid LTH"),
        ("tlth.m", "M-sektionen", "Maskinteknik"),
        ("tlth.v", "V-sektionen", "Väg- och vattenbyggnad"),
        ("tlth.a", "A-sektionen", "Arkitektur"),
        ("tlth.k", "K-sektionen", "Kemi"),
        ("tlth.d", "D-sektionen", "Datateknik"),
        ("tlth.ing", "Ingenjörssektionen", "Ingenjörshögskolan"),
        ("tlth.w", "W-sektionen", "Ekosystemteknik"),
        ("tlth.i", "I-sektionen", "Industriell ekonomi"),
    ];
    for (path, sv, sv_desc) in groups {
        let name = serde_json::json!({ "en": sv, "sv": sv });
        let description = serde_json::json!({ "en": sv_desc, "sv": sv_desc });
        let path = path.parse::<PgLTree>().wrap_err("parse path")?;
        sqlx::query!(
            "insert into groups (path, limit_membership_visibility, name, description, logo_id, deleted)
             values ($1, false, $2, $3, $4, false)
             on conflict (path) do update set name = excluded.name, description = excluded.description",
            path,
            name,
            description,
            LOGO_IMG,
        )
        .execute(&ctx.db)
        .await
        .wrap_err_with(|| format!("seed group {path}"))?;
    }
    Ok(())
}

/// Look up a group id by path. Groups must already be seeded.
async fn group_id(ctx: &Context, path: &str) -> color_eyre::Result<Uuid> {
    let path = path.parse::<PgLTree>().wrap_err("parse path")?;
    let row = sqlx::query!("select id from groups where path = $1", path)
        .fetch_one(&ctx.db)
        .await
        .wrap_err_with(|| format!("lookup group {path}"))?;
    Ok(row.id)
}

/// Test users + pre-seeded group memberships. Names are encrypted using
/// the same nonce + chacha20 setup the auth callback uses, so re-signing
/// in via the test provider doesn't replace them. Memberships are
/// inserted directly (rather than relying on `demo.csv` + auth callback)
/// so every seeded user can immediately see seeded activities without
/// the demo-csv detour — the activity-list query requires membership in
/// a group that's allowed to purchase one of the activity's kinds.
async fn seed_users(ctx: &Context) -> color_eyre::Result<()> {
    let users: &[(&str, &str, &str)] = &[
        ("test:si1234mc-s", "Simon Mechler", "tlth.d"),
        ("test:ma5657ed-s", "Max Edman", "tlth.e"),
        ("test:er7826an-s", "Erik Andersson", "tlth.e"),
    ];
    for (id, name, guild_path) in users {
        let nonce: [u8; 12] = rand::random();
        let mut name_bytes: Vec<u8> = (*name).as_bytes().to_vec();
        ctx.endecrypt_mut_slice(&mut name_bytes, &nonce);
        sqlx::query!(
            "insert into users (id, name, language, nonce) values ($1, $2, ''::bytea, $3)
             on conflict (id) do nothing",
            id,
            name_bytes,
            &nonce[..]
        )
        .execute(&ctx.db)
        .await
        .wrap_err_with(|| format!("seed user {id}"))?;

        let group_id = group_id(ctx, guild_path).await?;
        sqlx::query!(
            "insert into group_memberships (user_id, group_id) values ($1, $2)
             on conflict do nothing",
            id,
            group_id,
        )
        .execute(&ctx.db)
        .await
        .wrap_err_with(|| format!("seed membership {id} -> {guild_path}"))?;
    }
    Ok(())
}

struct ActivitySeed<'a> {
    id: Uuid,
    creator_path: &'a str,
    host_paths: &'a [&'a str],
    responsible_id: &'a str,
    title_en: &'a str,
    title_sv: &'a str,
    description_en: &'a str,
    description_sv: &'a str,
    location_en: &'a str,
    location_sv: &'a str,
    time_start: OffsetDateTime,
    time_end: OffsetDateTime,
    image_url: &'a str,
    max_tickets: i32,
    ticket_kinds: &'a [TicketKindSeed<'a>],
}

struct TicketKindSeed<'a> {
    id: Uuid,
    name_en: &'a str,
    name_sv: &'a str,
    /// Öre (i64). The schema stores this as `money`; PgMoney serialises
    /// it as an int64 of minor units.
    price_ore: i64,
    purchasing_start: OffsetDateTime,
    purchasing_stop: OffsetDateTime,
    max_tickets: i32,
}

/// Parse an RFC 3339 timestamp string at compile-call-site. Panics if
/// the literal is malformed — fine for fixture data where the strings
/// are static.
fn dt(rfc3339: &str) -> OffsetDateTime {
    OffsetDateTime::parse(rfc3339, &Rfc3339).expect("valid rfc3339 datetime")
}

async fn seed_activities(ctx: &Context) -> color_eyre::Result<()> {
    let activities: &[ActivitySeed] = &[
        ActivitySeed {
            id: Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_000a),
            creator_path: "tlth.a",
            host_paths: &["tlth.a", "tlth.f"],
            responsible_id: "test:si1234mc-s",
            title_en: "Other sitting kinda",
            title_sv: "Annan sittning typ",
            description_en: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.",
            description_sv: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.",
            location_en: "Gasque Hall",
            location_sv: "Gasque-salen",
            time_start: dt("2026-06-15T17:00:00Z"),
            time_end: dt("2026-06-15T23:00:00Z"),
            image_url: "https://picsum.photos/seed/home-a/640/360",
            max_tickets: 200,
            ticket_kinds: &[
                TicketKindSeed {
                    id: Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_a000_0001),
                    name_en: "Standard",
                    name_sv: "Standard",
                    price_ore: 12_000,
                    purchasing_start: dt("2026-05-20T00:00:00Z"),
                    purchasing_stop: dt("2026-06-15T12:00:00Z"),
                    max_tickets: 150,
                },
                TicketKindSeed {
                    id: Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_a000_0002),
                    name_en: "VIP",
                    name_sv: "VIP",
                    price_ore: 22_000,
                    purchasing_start: dt("2026-05-20T00:00:00Z"),
                    purchasing_stop: dt("2026-06-15T12:00:00Z"),
                    max_tickets: 5,
                },
                TicketKindSeed {
                    id: Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_a000_0003),
                    name_en: "Sponsor",
                    name_sv: "Sponsor",
                    price_ore: 0,
                    purchasing_start: dt("2026-05-20T00:00:00Z"),
                    purchasing_stop: dt("2026-06-15T12:00:00Z"),
                    max_tickets: 50,
                },
            ],
        },
        ActivitySeed {
            id: Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_000b),
            creator_path: "tlth.d",
            host_paths: &["tlth.d"],
            responsible_id: "test:si1234mc-s",
            title_en: "Spring fest",
            title_sv: "Vårfest",
            description_en: "Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat.",
            description_sv: "Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat.",
            location_en: "Kårhuset",
            location_sv: "Kårhuset",
            time_start: dt("2026-06-22T21:00:00Z"),
            time_end: dt("2026-06-23T02:00:00Z"),
            image_url: "https://picsum.photos/seed/home-b/640/360",
            max_tickets: 400,
            ticket_kinds: &[
                TicketKindSeed {
                    id: Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_b000_0001),
                    name_en: "Early bird",
                    name_sv: "Early bird",
                    price_ore: 8_000,
                    purchasing_start: dt("2026-05-20T00:00:00Z"),
                    purchasing_stop: dt("2026-06-01T00:00:00Z"),
                    max_tickets: 100,
                },
                TicketKindSeed {
                    id: Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_b000_0002),
                    name_en: "Standard",
                    name_sv: "Standard",
                    price_ore: 11_000,
                    purchasing_start: dt("2026-06-01T00:00:00Z"),
                    purchasing_stop: dt("2026-06-22T12:00:00Z"),
                    max_tickets: 250,
                },
                TicketKindSeed {
                    id: Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_b000_0003),
                    name_en: "After party",
                    name_sv: "After party",
                    price_ore: 5_000,
                    purchasing_start: dt("2026-06-15T00:00:00Z"),
                    purchasing_stop: dt("2026-06-23T01:00:00Z"),
                    max_tickets: 50,
                },
            ],
        },
        ActivitySeed {
            id: Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_000c),
            creator_path: "tlth.i",
            host_paths: &["tlth.i"],
            responsible_id: "test:si1234mc-s",
            title_en: "Tuesday pub",
            title_sv: "Tisdagspub",
            description_en: "Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur.",
            description_sv: "Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur.",
            location_en: "Pub lokal",
            location_sv: "Pub-lokalen",
            time_start: dt("2026-06-29T18:00:00Z"),
            time_end: dt("2026-06-29T23:00:00Z"),
            image_url: "https://picsum.photos/seed/home-c/640/360",
            max_tickets: 100,
            ticket_kinds: &[
                TicketKindSeed {
                    id: Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_c000_0001),
                    name_en: "Entry",
                    name_sv: "Entré",
                    price_ore: 4_000,
                    purchasing_start: dt("2026-06-15T00:00:00Z"),
                    purchasing_stop: dt("2026-06-29T17:00:00Z"),
                    max_tickets: 80,
                },
                TicketKindSeed {
                    id: Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_c000_0002),
                    name_en: "Member",
                    name_sv: "Medlem",
                    price_ore: 0,
                    purchasing_start: dt("2026-06-15T00:00:00Z"),
                    purchasing_stop: dt("2026-06-29T17:00:00Z"),
                    max_tickets: 20,
                },
            ],
        },
    ];

    // Per-activity image rows; the URL differs across the three so we
    // can't reuse a single image_id like the groups do. The image id is
    // derived deterministically from the activity id so re-runs hit the
    // same row (idempotency).
    let mut image_ids: HashMap<&str, Uuid> = HashMap::new();
    for a in activities {
        let id =
            Uuid::from_u128(a.id.as_u128() ^ 0x49_4d_47_5f_53_45_45_44_49_44_5f_5f_5f_5f_5f_5f);
        sqlx::query!(
            "insert into images (id, size, url) values ($1, 0, $2) on conflict do nothing",
            id,
            a.image_url,
        )
        .execute(&ctx.db)
        .await
        .wrap_err("seed activity image")?;
        image_ids.insert(a.image_url, id);
    }

    // Allow every seeded ticket_kind to be purchased by anyone under
    // the `tlth` root. Without rows in `ticket_kind_allowed_groups` the
    // activity-list inner join finds no matches and the homepage stays
    // empty.
    let tlth_id = group_id(ctx, "tlth").await?;

    for a in activities {
        let creator_id = group_id(ctx, a.creator_path).await?;
        let image_id = *image_ids.get(a.image_url).expect("inserted above");
        let title = serde_json::json!({ "en": a.title_en, "sv": a.title_sv });
        let description = serde_json::json!({ "en": a.description_en, "sv": a.description_sv });
        let location_name = serde_json::json!({ "en": a.location_en, "sv": a.location_sv });

        // The `location` composite type doesn't have a clean sqlx
        // mapping for inserts via the macro, so we build it inline with
        // ROW(name, directions, coordinate, url)::location. Upsert
        // semantics so re-runs pick up updated fixture times.
        sqlx::query!(
            "insert into activities
                (id, responsible_id, creator_id, title, description, location,
                 time_start, time_end, image_id, is_hidden, max_tickets)
             values ($1, $2, $3, $4, $5, row($6::jsonb, null, null, null)::location, $7, $8, $9, false, $10)
             on conflict (id) do update set
                title = excluded.title,
                description = excluded.description,
                location = excluded.location,
                time_start = excluded.time_start,
                time_end = excluded.time_end,
                image_id = excluded.image_id,
                is_hidden = excluded.is_hidden,
                max_tickets = excluded.max_tickets",
            a.id,
            a.responsible_id,
            creator_id,
            title,
            description,
            location_name,
            a.time_start,
            a.time_end,
            image_id,
            a.max_tickets,
        )
        .execute(&ctx.db)
        .await
        .wrap_err_with(|| format!("seed activity {}", a.id))?;

        for path in a.host_paths {
            let group_id = group_id(ctx, path).await?;
            sqlx::query!(
                "insert into activity_hosts (activity_id, group_id) values ($1, $2) on conflict do nothing",
                a.id,
                group_id,
            )
            .execute(&ctx.db)
            .await
            .wrap_err("seed activity host")?;
        }

        for k in a.ticket_kinds {
            let name = serde_json::json!({ "en": k.name_en, "sv": k.name_sv });
            // `money` is i64 minor units. PgMoney binds straight to the
            // money column type without any text-format detour.
            let price = PgMoney(k.price_ore);
            sqlx::query!(
                "insert into ticket_kinds
                    (id, activity_id, name, price,
                     purchasing_available_start, purchasing_available_stop,
                     max_tickets, min_tickets, reserved_or_purchased_tickets,
                     allow_transfer_ticket_start, allow_transfer_ticket_stop,
                     allow_transfer_ticket_bypass_allowed_groups, has_been_purchased)
                 values ($1, $2, $3, $4,
                         $5, $6, $7, 0, 0, $5, $6, false, false)
                 on conflict (id) do update set
                    name = excluded.name,
                    price = excluded.price,
                    purchasing_available_start = excluded.purchasing_available_start,
                    purchasing_available_stop = excluded.purchasing_available_stop,
                    max_tickets = excluded.max_tickets,
                    allow_transfer_ticket_start = excluded.allow_transfer_ticket_start,
                    allow_transfer_ticket_stop = excluded.allow_transfer_ticket_stop",
                k.id,
                a.id,
                Json(&name) as _,
                price,
                k.purchasing_start,
                k.purchasing_stop,
                k.max_tickets,
            )
            .execute(&ctx.db)
            .await
            .wrap_err_with(|| format!("seed ticket kind {}", k.id))?;

            sqlx::query!(
                "insert into ticket_kind_allowed_groups (ticket_kind_id, group_id)
                 values ($1, $2) on conflict do nothing",
                k.id,
                tlth_id,
            )
            .execute(&ctx.db)
            .await
            .wrap_err_with(|| format!("seed ticket kind allowed group {}", k.id))?;
        }
    }
    Ok(())
}

async fn seed(ctx: &Context) -> color_eyre::Result<()> {
    seed_image(ctx).await?;
    seed_groups(ctx).await?;
    seed_users(ctx).await?;
    seed_activities(ctx).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let ctx = Context::new(None).await?;
    seed(&ctx).await?;
    tracing::info!("seed-dev: done");
    Ok(())
}
