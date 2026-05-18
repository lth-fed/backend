use poem::{Error, error::InternalServerError, http::StatusCode};
use poem_openapi::Object;
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::group::path::Path;

#[derive(Debug, Object)]
pub struct Adminship {
    pub group_path: Path,
    pub user_id: String,
}

fn group_not_found(id: Uuid) -> Error {
    Error::from_string(format!("group `{id}` not found"), StatusCode::NOT_FOUND)
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
/// Returns 404 if the group does not exist, 401 if the user is not an admin,
/// or an internal error if the database query fails.
pub async fn check_adminship(
    db: impl PgExecutor<'_>,
    user_id: &str,
    group_id: Uuid,
) -> poem::Result<()> {
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
    .await
    .map_err(InternalServerError)?;

    match row {
        None => Err(group_not_found(group_id)),
        Some(row) if !row.is_admin => Err(Error::from_string(
            format!("must be an admin of group `{group_id}`"),
            StatusCode::UNAUTHORIZED,
        )),
        Some(_) => Ok(()),
    }
}

/// Checks that the user has administrative rights on the parent group of the
/// given group.
///
/// # Errors
///
/// Returns 404 if the group does not exist, 400 if the group is a root group
/// (no parent), 401 if the user is not an admin of the parent, or an internal
/// error if the database query fails.
pub async fn check_parent_adminship(
    db: impl PgExecutor<'_>,
    user_id: &str,
    group_id: Uuid,
) -> poem::Result<()> {
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
    .await
    .map_err(InternalServerError)?;

    match row {
        None => Err(group_not_found(group_id)),
        Some(row) if row.parent_path.is_none() => Err(Error::from_string(
            "nobody may become admin of the root group",
            StatusCode::BAD_REQUEST,
        )),
        Some(row) if !row.is_admin => Err(Error::from_string(
            format!("must be an admin of the parent of group `{group_id}`"),
            StatusCode::UNAUTHORIZED,
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
/// Returns an error if the database query fails.
pub async fn create_adminship(
    db: impl PgExecutor<'_>,
    user_id: &str,
    group_id: Uuid,
) -> sqlx::Result<Adminship> {
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
}

/// Removes the adminship for the given user in the given group.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn remove_adminship(
    db: impl PgExecutor<'_>,
    user_id: &str,
    group_id: Uuid,
) -> sqlx::Result<()> {
    sqlx::query!(
        "delete from group_adminships where user_id = $1 and group_id = $2",
        user_id,
        group_id
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
    group_id: Uuid,
) -> sqlx::Result<()> {
    sqlx::query!(
        r#"delete from group_adminships ga
        using groups g, groups target
        where g.id = ga.group_id
          and target.id = $2
          and ga.user_id = $1
          and g.path <@ target.path"#,
        user_id,
        group_id
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
pub async fn group_admins(db: impl PgExecutor<'_>, group_id: Uuid) -> sqlx::Result<Vec<String>> {
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
}

/// Returns the list of groups the user is an admin of.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn user_admin_groups(db: impl PgExecutor<'_>, user_id: &str) -> sqlx::Result<Vec<Path>> {
    sqlx::query_scalar!(
        r#"select g.path as "path!: Path"
        from group_adminships ga
        join groups g on g.id = ga.group_id
        where ga.user_id = $1"#,
        user_id
    )
    .fetch_all(db)
    .await
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
                .map(|p| p.to_string()),
            Some("tlth.e".to_string()),
        );
    }

    #[sqlx::test(fixtures("adminship"))]
    async fn create_adminship_idempotent(db: PgPool) {
        let e_id = id_of(&db, "tlth.e").await;
        create_adminship(&db, "user_a", e_id).await.unwrap();
        create_adminship(&db, "user_a", e_id).await.unwrap();
    }

    #[sqlx::test(fixtures("adminship"))]
    async fn test_remove_adminship(db: PgPool) {
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

        remove_all_adminships(&db, "user_a", e_id).await.unwrap();
        assert!(group_admins(&db, nolla_id).await.unwrap().is_empty());
    }
}
