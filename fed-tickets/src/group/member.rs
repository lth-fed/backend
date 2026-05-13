use sqlx::PgExecutor;

use crate::group::{Group, path::Path};

/// Returns the closest matching (longest prefix) group path for the given user
/// and group path, if such a membership exists.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn closest_user_membership(
    db: impl PgExecutor<'_>,
    user_id: &str,
    group_path: &Path,
) -> sqlx::Result<Option<Path>> {
    sqlx::query_scalar!(
        r#"select group_path
        from group_memberships
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

/// Returns all the groups that the user is a member of, including nested
/// groups.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn user_groups(db: impl PgExecutor<'_>, user_id: &str) -> sqlx::Result<Vec<Group>> {
    // TODO: Tree structure?
    sqlx::query_as!(
        Group,
        r#"
            select distinct g.path, g.limit_membership_visibility, g.name, g.description, g.deleted
            from groups g
            join group_memberships gm on gm.user_id = $1
            where
                (g.limit_membership_visibility = false and g.path <@ gm.group_path)
                or
                (g.limit_membership_visibility = true and g.path = gm.group_path)
        "#,
        user_id
    )
    .fetch_all(db)
    .await
}

/// Returns the direct and transitive members of a group.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn group_members(
    db: impl PgExecutor<'_>,
    group_path: &Path,
) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar!(
        "select distinct user_id from group_memberships where group_path <@ $1",
        group_path.0
    )
    .fetch_all(db)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    fn sorted_paths(groups: Vec<Group>) -> Vec<String> {
        let mut paths: Vec<String> = groups.into_iter().map(|g| g.path.to_string()).collect();
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
            group_members(&db, &"tlth".parse().unwrap()).await.unwrap(),
            vec!["user_a", "user_b", "user_c"]
        );

        assert_eq!(
            group_members(&db, &"tlth.d.nolla".parse().unwrap())
                .await
                .unwrap(),
            vec!["user_b"]
        );
    }
}
