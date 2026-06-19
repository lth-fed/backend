use sqlx::PgExecutor;
use uuid::Uuid;

use crate::group::{Group, path::Path};
use crate::{DbInternationalizedString as DIS, MinilithErrorResultExt as _, MinilithResult};

/// Returns the closest matching (longest prefix) group path for the given user
/// and group id, if such a membership exists. Returns `None` if the user has
/// no membership covering the group, or if the group does not exist.
///
/// # Errors
///
/// DB.
pub async fn closest_user_membership(
    db: impl PgExecutor<'_>,
    user_id: &str,
    group_id: Uuid,
) -> MinilithResult<Option<Path>> {
    sqlx::query_scalar!(
        r#"select g.path as "path!: Path"
        from group_memberships gm
        join groups g on g.id = gm.group_id
        join groups target on target.id = $2
        where gm.user_id = $1 and g.path @> target.path
        order by nlevel(g.path) desc
        limit 1"#,
        user_id,
        group_id
    )
    .fetch_optional(db)
    .await
    .wrap_err_db()
}

/// Returns all the groups that the user is a member of, including nested
/// groups.
///
/// # Errors
///
/// DB.
pub async fn user_groups(db: impl PgExecutor<'_>, user_id: &str) -> MinilithResult<Vec<Group>> {
    // TODO: Tree structure?
    sqlx::query_as!(
        Group,
        r#"
            select distinct g.id, g.path, g.limit_membership_visibility, g.name as "name!: DIS", g.description as "description!: DIS", g.deleted
            from groups g
            join group_memberships gm on gm.user_id = $1
            join groups mg on mg.id = gm.group_id
            where
                (g.limit_membership_visibility = false and g.path <@ mg.path)
                or
                (g.limit_membership_visibility = true and g.id = gm.group_id)
        "#,
        user_id
    )
    .fetch_all(db)
    .await
    .wrap_err_db()
}

/// Returns the direct and transitive members of a group.
///
/// # Errors
///
/// DB.
pub async fn group_members(db: impl PgExecutor<'_>, group_id: Uuid) -> MinilithResult<Vec<String>> {
    sqlx::query_scalar!(
        r#"select distinct gm.user_id
        from group_memberships gm
        join groups g on g.id = gm.group_id
        join groups target on target.id = $1
        where g.path <@ target.path"#,
        group_id
    )
    .fetch_all(db)
    .await
    .wrap_err_db()
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

    fn sorted_paths(groups: Vec<Group>) -> Vec<String> {
        let mut paths: Vec<String> = groups
            .into_iter()
            .map(|group| group.path.to_string())
            .collect();
        paths.sort();
        paths
    }

    // check out fixtures/user_groups.sql
    #[sqlx::test(fixtures("user_groups"))]
    async fn group_membership(db: PgPool) {
        // Membership in `tlth.e` (limit_mv=false) reveals it and its limit_mv=false
        // descendants, but not the limit_mv=true `tlth.e.nolla`.
        assert_eq!(
            sorted_paths(user_groups(&db, "user_a").await.unwrap()),
            vec!["tlth.e", "tlth.e.styrelsen"],
        );

        // Direct membership is the only way into a limit_mv=true group.
        assert_eq!(
            sorted_paths(user_groups(&db, "user_b").await.unwrap()),
            vec!["tlth.d.nolla"],
        );

        // Overlapping memberships dedupe; limit_mv=true `*.nolla` groups stay hidden.
        assert_eq!(
            sorted_paths(user_groups(&db, "user_c").await.unwrap()),
            vec![
                "tlth",
                "tlth.d",
                "tlth.d.styrelsen",
                "tlth.e",
                "tlth.e.styrelsen",
                "tlth.f",
                "tlth.f.styrelsen",
            ],
        );

        assert!(user_groups(&db, "nobody").await.unwrap().is_empty());

        assert_eq!(
            group_members(&db, id_of(&db, "tlth").await).await.unwrap(),
            vec!["user_a", "user_b", "user_c"]
        );

        assert_eq!(
            group_members(&db, id_of(&db, "tlth.d.nolla").await)
                .await
                .unwrap(),
            vec!["user_b"]
        );
    }
}
