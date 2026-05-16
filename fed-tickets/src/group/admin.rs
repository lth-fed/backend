use poem::{Error, error::InternalServerError, http::StatusCode};
use poem_openapi::Object;
use sqlx::PgExecutor;

use crate::group::path::Path;

#[derive(Debug, Object)]
pub struct Adminship {
    pub group_path: Path,
    pub user_id: String,
}

/// This is basically identical to [`super::member::closest_user_membership`].
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn closest_user_adminship(
    db: impl PgExecutor<'_>,
    user_id: &str,
    group_path: &Path,
) -> sqlx::Result<Option<Path>> {
    sqlx::query_scalar!(
        r#"select group_path
        from group_adminships
        where user_id = $1 and group_path @> $2
        order by nlevel(group_path) desc
        limit 1"#,
        user_id,
        group_path.0
    )
    .fetch_optional(db)
    .await
    .map(|opt| opt.map(Path))
}

/// Checks that the user has administrative rights on the given group.
///
/// # Errors
///
/// Returns an error if the database query fails or the user is not an admin.
pub async fn check_adminship(
    db: impl PgExecutor<'_>,
    user_id: &str,
    group: &Path,
) -> poem::Result<()> {
    // If closest_user_adminship returns Some(_), we're good.
    closest_user_adminship(db, user_id, group)
        .await
        .map_err(InternalServerError)?
        .ok_or_else(|| {
            // closest_user_adminship returned None, so the user is not an admin.
            Error::from_string(
                format!("must be an admin of {group}"),
                StatusCode::UNAUTHORIZED,
            )
        })?;

    Ok(())
}

/// Checks that the user has administrative rights on the parent group of the
/// given group.
///
/// # Errors
///
/// Returns an error if the database query fails, if the user is not an admin
/// of the parent group, or if the provided group has no parent group (i.e.,
/// it is the root group).
pub async fn check_parent_adminship(
    db: impl PgExecutor<'_>,
    user_id: &str,
    group: &Path,
) -> poem::Result<()> {
    let parent_group = group.parent().ok_or_else(|| {
        Error::from_string(
            "nobody may become admin of the root group",
            StatusCode::BAD_REQUEST,
        )
    })?;

    check_adminship(db, user_id, &parent_group).await
}

/// Creates an adminship for the user on the given group.
///
/// If the user is not already a member of the group, a membership is created
/// first.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn create_adminship(
    db: impl PgExecutor<'_>,
    user_id: &str,
    group_path: &Path,
) -> sqlx::Result<Adminship> {
    sqlx::query_as!(
        Adminship,
        r#"with ensure_membership as (
            insert into group_memberships (user_id, group_path)
            values ($1, $2)
            on conflict do nothing
        )
        insert into group_adminships (user_id, group_path)
        values ($1, $2)
        -- Noop because if we had `on conflict do nothing`, nothing would be
        -- returned on conflict.
        on conflict(user_id, group_path) do update set user_id = $1, group_path = $2
        returning user_id, group_path"#,
        user_id,
        group_path.0
    )
    .fetch_one(db)
    .await
}

/// Removes the adminship for the given user in the given group.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn remove_adminship(
    db: impl PgExecutor<'_>,
    user_id: &str,
    group_path: &Path,
) -> sqlx::Result<()> {
    sqlx::query!(
        "delete from group_adminships where user_id = $1 and group_path = $2",
        user_id,
        group_path.0
    )
    .execute(db)
    .await
    .map(|_| ())
}

/// Removes all adminships, direct as well as transitive, for the given user in
/// the given group.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn remove_all_adminships(
    db: impl PgExecutor<'_>,
    user_id: &str,
    group_path: &Path,
) -> sqlx::Result<()> {
    sqlx::query!(
        "delete from group_adminships where user_id = $1 and group_path <@ $2",
        user_id,
        group_path.0
    )
    .execute(db)
    .await
    .map(|_| ())
}

/// Returns the list of admins for the given group.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn group_admins(db: impl PgExecutor<'_>, group_path: &Path) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar!(
        "select distinct user_id from group_adminships where group_path @> $1",
        group_path.0
    )
    .fetch_all(db)
    .await
}

/// Returns the list of groups the user is an admin of.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn user_admin_groups(db: impl PgExecutor<'_>, user_id: &str) -> sqlx::Result<Vec<Path>> {
    sqlx::query_scalar!(
        "select group_path from group_adminships where user_id = $1",
        user_id
    )
    .fetch_all(db)
    .await
    .map(|paths| paths.into_iter().map(Path).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    #[sqlx::test(fixtures("adminship"))]
    async fn create_adminship_for_non_member(db: PgPool) {
        let group: Path = "tlth.e".parse().unwrap();

        let adminship = create_adminship(&db, "user_a", &group).await.unwrap();
        assert_eq!(adminship.user_id, "user_a");
        assert_eq!(adminship.group_path.to_string(), "tlth.e");

        assert_eq!(
            group_admins(&db, &group).await.unwrap(),
            vec!["user_a"],
            "the new adminship should be visible via group_admins",
        );
        assert_eq!(
            closest_user_adminship(&db, "user_a", &group)
                .await
                .unwrap()
                .map(|p| p.to_string()),
            Some("tlth.e".to_string()),
        );
    }

    #[sqlx::test(fixtures("adminship"))]
    async fn create_adminship_idempotent(db: PgPool) {
        let group: Path = "tlth.e".parse().unwrap();
        create_adminship(&db, "user_a", &group).await.unwrap();
        create_adminship(&db, "user_a", &group).await.unwrap();
    }

    #[sqlx::test(fixtures("adminship"))]
    async fn test_remove_adminship(db: PgPool) {
        let nolla: Path = "tlth.e.nolla".parse().unwrap();
        let e = nolla.parent().unwrap();

        create_adminship(&db, "user_a", &nolla).await.unwrap();

        assert_eq!(group_admins(&db, &nolla).await.unwrap(), vec!["user_a"]);
        assert!(group_admins(&db, &e).await.unwrap().is_empty());

        create_adminship(&db, "user_a", &e).await.unwrap();
        assert_eq!(group_admins(&db, &e).await.unwrap(), vec!["user_a"]);

        remove_adminship(&db, "user_a", &e).await.unwrap();
        assert_eq!(group_admins(&db, &nolla).await.unwrap(), vec!["user_a"]);
        assert!(group_admins(&db, &e).await.unwrap().is_empty());

        remove_all_adminships(&db, "user_a", &e).await.unwrap();
        assert!(group_admins(&db, &nolla).await.unwrap().is_empty());
    }
}
