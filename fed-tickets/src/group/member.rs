use sqlx::PgExecutor;
use uuid::Uuid;

use crate::group::{Group, path::Path};

/// Returns the closest matching (longest prefix) group path for the given user
/// and group id, if such a membership exists. Returns `None` if the user has
/// no membership covering the group, or if the group does not exist.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn closest_user_membership(
    db: impl PgExecutor<'_>,
    user_id: &str,
    group_id: Uuid,
) -> sqlx::Result<Option<Path>> {
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
            select id, path, membership_inherits_upward, name, description, deleted
            from effective_group_memberships eg
            join groups g on g.id = eg.group_id
            where eg.user_id = $1
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
pub async fn group_members(db: impl PgExecutor<'_>, group_id: Uuid) -> sqlx::Result<Vec<String>> {
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
        let mut paths: Vec<String> = groups.into_iter().map(|g| g.path.to_string()).collect();
        paths.sort();
        paths
    }

    // check out fixtures/user_groups.sql
    #[sqlx::test(fixtures("user_groups"))]
    async fn group_membership(db: PgPool) {
        assert_eq!(
            sorted_paths(user_groups(&db, "user_a").await.unwrap()),
            vec!["tlth", "tlth.e"],
        );

        // Direct membership is the only way into a limit_mv=true group.
        assert_eq!(
            sorted_paths(user_groups(&db, "user_b").await.unwrap()),
            vec!["tlth.d.nolla"],
        );

        // Overlapping memberships dedupe; limit_mv=true `*.nolla` groups stay hidden.
        assert_eq!(
            sorted_paths(user_groups(&db, "user_c").await.unwrap()),
            vec!["tlth", "tlth.f", "tlth.f.styrelsen"],
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
