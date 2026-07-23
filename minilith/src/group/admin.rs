use poem_openapi::Object;
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::group::path::Path;
use crate::{MinilithEndpointError, MinilithResult};

#[derive(Debug, Object)]
pub struct Adminship {
    pub group_path: Path,
    pub user_id: String,
}

/// Returns the closest matching (longest prefix) admin path for the given user
/// and group id, if such an adminship exists. Returns `None` if the user has
/// no admin path covering the group, or if the group does not exist.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn closest_user_adminship(
    db: impl PgExecutor<'_>,
    user_id: &str,
    group_id: Uuid,
) -> sqlx::Result<Option<Path>> {
    sqlx::query_scalar!(
        // `g.path @> target.path` — admin path is an ancestor of (or equal
        // to) the target, so admin of any ancestor counts.
        r#"select g.path as "path!: Path"
        from group_adminships ga
        join groups g on g.id = ga.group_id
        join groups target on target.id = $2
        where ga.user_id = $1 and g.path @> target.path
        order by nlevel(g.path) desc
        limit 1"#,
        user_id,
        group_id
    )
    .fetch_optional(db)
    .await
}

/// Checks that the user has administrative rights on the given group.
///
/// # Errors
///
/// - the user has to be an admin of this group
pub async fn check_adminship(
    db: impl PgExecutor<'_>,
    user_id: &str,
    group_id: Uuid,
) -> MinilithResult<()> {
    let row = sqlx::query!(
        r#"select exists (
            select 1 from group_adminships ga
            join groups g on g.id = ga.group_id
            where ga.user_id = $1 and g.path @> target.path
        ) as "is_admin!"
        from groups target
        where target.id = $2"#,
        user_id,
        group_id
    )
    .fetch_optional(db)
    .await?;

    match row {
        None => Err(MinilithEndpointError::not_found()),
        Some(row) if !row.is_admin => Err(MinilithEndpointError::bad_frontend_code(
            format!("must be an admin of the group {group_id}"),
            "",
        )),
        Some(_) => Ok(()),
    }
}

/// Checks that the user has administrative rights on the parent group of the
/// given group.
///
/// # Errors
///
/// - root must have no admins
/// - the user must be admin of the parent of this group
pub async fn check_parent_adminship(
    db: impl PgExecutor<'_>,
    user_id: &str,
    group_id: Uuid,
) -> MinilithResult<()> {
    let row = sqlx::query!(
        r#"select
            target.parent_path as "parent_path: Path",
            exists (
                select 1 from group_adminships ga
                join groups g on g.id = ga.group_id
                where ga.user_id = $1 and g.path @> target.parent_path
            ) as "is_admin!"
        from groups target
        where target.id = $2"#,
        user_id,
        group_id
    )
    .fetch_optional(db)
    .await?;

    match row {
        None => Err(MinilithEndpointError::not_found()),
        Some(row) if row.parent_path.is_none() => Err(MinilithEndpointError::bad_frontend_code(
            "nobody may become admin of the root group",
            "",
        )),
        Some(row) if !row.is_admin => Err(MinilithEndpointError::bad_frontend_code(
            format!("you must be an admin of the parent of the group {group_id}"),
            "",
        )),
        Some(_) => Ok(()),
    }
}

/// Creates an adminship for the user on the given group.
///
/// If the user is not already a member of the group, a membership is created
/// first.
///
/// # Errors
///
/// DB.
pub async fn create_adminship(
    db: impl PgExecutor<'_>,
    user_id: &str,
    group_id: Uuid,
) -> MinilithResult<Adminship> {
    sqlx::query_as!(
        Adminship,
        r#"with ensure_membership as (
            insert into group_memberships (user_id, group_id)
            values ($1, $2)
            on conflict do nothing
        ), upsert_adminship as (
            insert into group_adminships (user_id, group_id)
            values ($1, $2)
            -- Noop because if we had `on conflict do nothing`, nothing would be
            -- returned on conflict.
            on conflict(user_id, group_id) do update set user_id = $1, group_id = $2
            returning user_id, group_id
        )
        select ua.user_id, g.path as "group_path!: Path"
        from upsert_adminship ua
        join groups g on g.id = ua.group_id"#,
        user_id,
        group_id
    )
    .fetch_one(db)
    .await
    .map_err(Into::into)
}

/// Removes the adminship for the given user in the given group.
///
/// # Errors
///
/// DB.
pub async fn remove_adminship(
    db: impl PgExecutor<'_>,
    user_id: &str,
    group_id: Uuid,
) -> MinilithResult<()> {
    sqlx::query!(
        "delete from group_adminships where user_id = $1 and group_id = $2",
        user_id,
        group_id
    )
    .execute(db)
    .await
    .map(|_| ())
    .map_err(Into::into)
}

/// Returns the list of admins for the given group.
///
/// # Errors
///
/// DB.
pub async fn group_admins(db: impl PgExecutor<'_>, group_id: Uuid) -> MinilithResult<Vec<String>> {
    sqlx::query_scalar!(
        r#"select distinct ga.user_id
        from group_adminships ga
        join groups g on g.id = ga.group_id
        join groups target on target.id = $1
        where g.path @> target.path"#,
        group_id
    )
    .fetch_all(db)
    .await
    .map_err(Into::into)
}

/// Returns the list of groups the user is an admin of.
///
/// # Errors
///
/// DB.
pub async fn user_admin_groups(
    db: impl PgExecutor<'_>,
    user_id: &str,
) -> MinilithResult<Vec<Path>> {
    sqlx::query_scalar!(
        r#"select g.path as "path!: Path"
        from group_adminships ga
        join groups g on g.id = ga.group_id
        where ga.user_id = $1"#,
        user_id
    )
    .fetch_all(db)
    .await
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    use crate::group::id_by_path;

    async fn id_of(db: &PgPool, path: &str) -> Uuid {
        let path: Path = path.parse().unwrap();
        id_by_path(db, &path).await.unwrap().unwrap()
    }

    #[sqlx::test(fixtures("adminship"))]
    async fn create_adminship_for_non_member(db: PgPool) {
        let e_id = id_of(&db, "tlth.e").await;

        let adminship = create_adminship(&db, "user_a", e_id).await.unwrap();
        assert_eq!(adminship.user_id, "user_a");
        assert_eq!(adminship.group_path.to_string(), "tlth.e");

        assert_eq!(
            group_admins(&db, e_id).await.unwrap(),
            vec!["user_a"],
            "the new adminship should be visible via group_admins",
        );
        assert_eq!(
            closest_user_adminship(&db, "user_a", e_id)
                .await
                .unwrap()
                .map(|path| path.to_string()),
            Some("tlth.e".to_owned()),
        );
    }

    #[sqlx::test(fixtures("adminship"))]
    async fn create_adminship_idempotent(db: PgPool) {
        let e_id = id_of(&db, "tlth.e").await;
        create_adminship(&db, "user_a", e_id).await.unwrap();
        create_adminship(&db, "user_a", e_id).await.unwrap();
    }

    #[sqlx::test(fixtures("adminship"))]
    async fn remove_adminships(db: PgPool) {
        let nolla_id = id_of(&db, "tlth.e.nolla").await;
        let e_id = id_of(&db, "tlth.e").await;

        create_adminship(&db, "user_a", nolla_id).await.unwrap();

        assert_eq!(group_admins(&db, nolla_id).await.unwrap(), vec!["user_a"]);
        assert!(group_admins(&db, e_id).await.unwrap().is_empty());

        create_adminship(&db, "user_a", e_id).await.unwrap();
        assert_eq!(group_admins(&db, e_id).await.unwrap(), vec!["user_a"]);

        remove_adminship(&db, "user_a", e_id).await.unwrap();
        assert_eq!(group_admins(&db, nolla_id).await.unwrap(), vec!["user_a"]);
        assert!(group_admins(&db, e_id).await.unwrap().is_empty());
    }
}
