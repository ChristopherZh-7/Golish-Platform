//! Schema migration for the on-disk settings TOML.
//!
//! Each user-facing schema bump (controlled by [`SCHEMA_VERSION`]) gets one
//! migration arm in [`migrate_settings`]; the loader runs the chain in sequence
//! before deserialising the typed [`crate::schema::GolishSettings`].

use anyhow::Result;

use crate::schema::SCHEMA_VERSION;

/// Apply a chain of schema migrations to bring a raw TOML value
/// up to the current [`SCHEMA_VERSION`].
///
/// Each migration function handles exactly one version bump.
/// Add a new arm whenever `SCHEMA_VERSION` is incremented.
pub fn migrate_settings(toml: &mut toml::Value) -> Result<()> {
    let table = toml
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("Settings root is not a TOML table"))?;

    // Detect current version (supports both legacy `version` and new `schema_version`)
    let current: u32 = table
        .get("schema_version")
        .or_else(|| table.get("version"))
        .and_then(|v| v.as_integer())
        .map(|v| v as u32)
        .unwrap_or(1);

    // When adding a new schema version, uncomment and extend this loop:
    //
    //   let mut v = current;
    //   while v < SCHEMA_VERSION {
    //       match v {
    //           1 => migrate_v1_to_v2(table)?,
    //           _ => anyhow::bail!("No migration from v{} to v{}", v, v + 1),
    //       }
    //       v += 1;
    //   }

    if current > SCHEMA_VERSION {
        tracing::warn!(
            "Settings file has schema_version {} (> current {}); loading anyway",
            current,
            SCHEMA_VERSION,
        );
    }

    // Normalise: ensure the canonical key is `schema_version`, drop legacy `version`
    table.insert(
        "schema_version".to_string(),
        toml::Value::Integer(i64::from(SCHEMA_VERSION)),
    );
    table.remove("version");

    Ok(())
}
