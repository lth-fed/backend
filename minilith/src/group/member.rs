use sqlx::PgExecutor;
use uuid::Uuid;

use crate::group::{FatGroup, path::Path};
use crate::{DbInternationalizedString as DIS, MinilithResult};

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
    .map_err(Into::into)
}

/// Returns the groups in the user's accessible tree.
///
/// # Errors
///
/// DB.
pub async fn user_groups_tree(
    db: impl PgExecutor<'_>,
    user_id: &str,
) -> MinilithResult<Vec<FatGroup>> {
    sqlx::query_as!(
        FatGroup,
        r#"
            select distinct
                g.id, g.path, g.limit_membership_visibility,
                g.name as "name!: DIS",
                g.description as "description!: DIS",
                g.deleted,
                logo.id as logo_id,
                logo.url as logo_url,
                (select array(select user_id from group_adminships where group_id = g.id)) as admin_ids
            from groups g
            join group_memberships gm on gm.user_id = $1
            join groups mg on mg.id = gm.group_id
            join images logo on logo.id = g.logo_id
            where subpath(g.path, 0, 1) = subpath(mg.path, 0, 1)
            and g.deleted = false
            order by g.path
        "#,
        user_id
    )
    .fetch_all(db)
    .await
    .map_err(Into::into)
}

/// Returns the direct and transitive members of a group.
///
/// # Errors
///
/// DB.
pub async fn group_members(db: impl PgExecutor<'_>, group_id: Uuid) -> MinilithResult<Vec<String>> {
    sqlx::query_scalar!(
        r#"select gm.user_id
        from group_memberships gm
        left outer join group_adminships ga
            on ga.group_id = gm.group_id and ga.user_id = gm.user_id
        where gm.group_id = $1
        -- if there's a group adminship, we don't list it
        and ga.user_id is null
        order by gm.user_id"#,
        group_id
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

    fn sorted_paths(groups: Vec<FatGroup>) -> Vec<String> {
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
        let complete_tlth_tree = vec![
            "tlth",
            "tlth.d",
            "tlth.d.nolla",
            "tlth.d.styrelsen",
            "tlth.e",
            "tlth.e.nolla",
            "tlth.e.styrelsen",
            "tlth.f",
            "tlth.f.nolla",
            "tlth.f.styrelsen",
        ];

        // Any direct membership reveals the complete root tree for filters.
        assert_eq!(
            sorted_paths(user_groups_tree(&db, "user_a").await.unwrap()),
            complete_tlth_tree.clone(),
        );

        assert_eq!(
            sorted_paths(user_groups_tree(&db, "user_b").await.unwrap()),
            complete_tlth_tree.clone(),
        );

        assert_eq!(
            sorted_paths(user_groups_tree(&db, "user_c").await.unwrap()),
            complete_tlth_tree,
        );

        assert!(user_groups_tree(&db, "nobody").await.unwrap().is_empty());

        assert_eq!(
            group_members(&db, id_of(&db, "tlth").await).await.unwrap(),
            vec!["user_c"]
        );

        assert_eq!(
            group_members(&db, id_of(&db, "tlth.d.nolla").await)
                .await
                .unwrap(),
            vec!["user_b"]
        );
    }
}
