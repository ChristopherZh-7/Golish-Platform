use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::Organization;

/// List all organizations for a project (flat, sorted by parent_id NULLs first
/// then sort_order). Callers typically rebuild the tree client-side.
pub async fn list(pool: &PgPool, project_path: &str) -> Result<Vec<Organization>> {
    let rows = sqlx::query_as::<_, Organization>(
        r#"SELECT * FROM organizations
           WHERE project_path = $1
           ORDER BY parent_id NULLS FIRST, sort_order, name"#,
    )
    .bind(project_path)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn create(
    pool: &PgPool,
    project_path: &str,
    name: &str,
    parent_id: Option<Uuid>,
    description: &str,
    owner: &str,
) -> Result<Organization> {
    let row = sqlx::query_as::<_, Organization>(
        r#"INSERT INTO organizations (project_path, name, parent_id, description, owner)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING *"#,
    )
    .bind(project_path)
    .bind(name)
    .bind(parent_id)
    .bind(description)
    .bind(owner)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn update(
    pool: &PgPool,
    id: Uuid,
    name: Option<&str>,
    description: Option<&str>,
    owner: Option<&str>,
    sort_order: Option<i32>,
) -> Result<Organization> {
    if let Some(n) = name {
        sqlx::query("UPDATE organizations SET name=$1, updated_at=NOW() WHERE id=$2")
            .bind(n)
            .bind(id)
            .execute(pool)
            .await?;
    }
    if let Some(d) = description {
        sqlx::query("UPDATE organizations SET description=$1, updated_at=NOW() WHERE id=$2")
            .bind(d)
            .bind(id)
            .execute(pool)
            .await?;
    }
    if let Some(o) = owner {
        sqlx::query("UPDATE organizations SET owner=$1, updated_at=NOW() WHERE id=$2")
            .bind(o)
            .bind(id)
            .execute(pool)
            .await?;
    }
    if let Some(s) = sort_order {
        sqlx::query("UPDATE organizations SET sort_order=$1, updated_at=NOW() WHERE id=$2")
            .bind(s)
            .bind(id)
            .execute(pool)
            .await?;
    }
    let row = sqlx::query_as::<_, Organization>("SELECT * FROM organizations WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await?;
    Ok(row)
}

/// Move an organization under a new parent (or to root with `new_parent=None`).
/// Caller is responsible for preventing cycles; we add a guard here.
pub async fn move_to(pool: &PgPool, id: Uuid, new_parent: Option<Uuid>) -> Result<()> {
    if let Some(target) = new_parent {
        if target == id {
            anyhow::bail!("cannot move organization under itself");
        }
        // Walk ancestor chain to ensure `target` is not a descendant of `id`.
        let mut cursor = Some(target);
        while let Some(cur) = cursor {
            if cur == id {
                anyhow::bail!("cannot move organization under its own descendant");
            }
            cursor = sqlx::query_scalar::<_, Option<Uuid>>(
                "SELECT parent_id FROM organizations WHERE id = $1",
            )
            .bind(cur)
            .fetch_optional(pool)
            .await?
            .flatten();
        }
    }
    sqlx::query("UPDATE organizations SET parent_id=$1, updated_at=NOW() WHERE id=$2")
        .bind(new_parent)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Cascade-deletes via ON DELETE CASCADE (subtree drops too).
pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM organizations WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
