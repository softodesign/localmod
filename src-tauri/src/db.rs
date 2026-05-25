use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OpenFlags};
use serde_json;
use std::path::Path;

pub fn open(path: &Path) -> Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_FULL_MUTEX;
    let conn = Connection::open_with_flags(path, flags)
        .with_context(|| format!("open sqlite at {}", path.display()))?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    Ok(conn)
}

pub fn migrate(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS settings (
          key TEXT PRIMARY KEY NOT NULL,
          value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS models (
          id TEXT PRIMARY KEY NOT NULL,
          name TEXT NOT NULL,
          path TEXT NOT NULL UNIQUE,
          quant TEXT,
          size_bytes INTEGER,
          created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS chats (
          id TEXT PRIMARY KEY NOT NULL,
          title TEXT NOT NULL,
          model_id TEXT,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          FOREIGN KEY (model_id) REFERENCES models(id) ON DELETE SET NULL
        );

        CREATE TABLE IF NOT EXISTS messages (
          id TEXT PRIMARY KEY NOT NULL,
          chat_id TEXT NOT NULL,
          role TEXT NOT NULL,
          content TEXT NOT NULL,
          created_at TEXT NOT NULL,
          FOREIGN KEY (chat_id) REFERENCES chats(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS context_documents (
          id TEXT PRIMARY KEY NOT NULL,
          name TEXT NOT NULL,
          source TEXT NOT NULL,
          kind TEXT NOT NULL,
          stored_path TEXT NOT NULL,
          size_bytes INTEGER,
          chunks INTEGER NOT NULL DEFAULT 0,
          status TEXT NOT NULL DEFAULT 'ready',
          created_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_messages_chat ON messages(chat_id, created_at);
        CREATE INDEX IF NOT EXISTS idx_chats_updated ON chats(updated_at DESC);
        "#,
    )?;
    tx.commit()?;
    add_models_weight_columns(conn)?;
    add_chats_system_prompt_column(conn)?;
    add_models_cloud_columns(conn)?;
    add_projects_table(conn)?;
    add_chats_project_id_column(conn)?;
    add_models_kind_column(conn)?;
    add_models_image_gen_columns(conn)?;
    demote_local_image_gen_models(conn)?;
    Ok(())
}

fn demote_local_image_gen_models(conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE models SET model_kind = 'chat' WHERE model_kind = 'image_gen'",
        [],
    )
    .ok();
    Ok(())
}

fn add_models_image_gen_columns(conn: &Connection) -> Result<()> {
    for sql in [
        "ALTER TABLE models ADD COLUMN image_gen_vae_path TEXT",
        "ALTER TABLE models ADD COLUMN image_gen_llm_path TEXT",
    ] {
        if let Err(e) = conn.execute(sql, []) {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                return Err(e).with_context(|| format!("migration: {sql}"));
            }
        }
    }
    Ok(())
}

fn add_models_kind_column(conn: &Connection) -> Result<()> {
    if let Err(e) = conn.execute(
        "ALTER TABLE models ADD COLUMN model_kind TEXT NOT NULL DEFAULT 'chat'",
        [],
    ) {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            return Err(e).with_context(|| "migration: models.model_kind");
        }
    }
    Ok(())
}

fn add_models_weight_columns(conn: &Connection) -> Result<()> {
    for sql in [
        "ALTER TABLE models ADD COLUMN weights_format TEXT NOT NULL DEFAULT 'gguf'",
        "ALTER TABLE models ADD COLUMN shard_index INTEGER",
        "ALTER TABLE models ADD COLUMN shard_total INTEGER",
    ] {
        if let Err(e) = conn.execute(sql, []) {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                return Err(e).with_context(|| format!("migration: {sql}"));
            }
        }
    }
    Ok(())
}

fn add_chats_system_prompt_column(conn: &Connection) -> Result<()> {
    if let Err(e) =
        conn.execute("ALTER TABLE chats ADD COLUMN system_prompt TEXT NOT NULL DEFAULT ''", [])
    {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            return Err(e).with_context(|| "migration: chats.system_prompt");
        }
    }
    Ok(())
}

fn add_models_cloud_columns(conn: &Connection) -> Result<()> {
    for sql in [
        "ALTER TABLE models ADD COLUMN cloud_provider TEXT",
        "ALTER TABLE models ADD COLUMN cloud_api_model TEXT",
    ] {
        if let Err(e) = conn.execute(sql, []) {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                return Err(e).with_context(|| format!("migration: {sql}"));
            }
        }
    }
    Ok(())
}

fn add_projects_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS projects (
          id TEXT PRIMARY KEY NOT NULL,
          name TEXT NOT NULL,
          description TEXT NOT NULL DEFAULT '',
          tags TEXT NOT NULL DEFAULT '[]',
          context TEXT NOT NULL DEFAULT '',
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_projects_updated ON projects(updated_at DESC);
        "#,
    )
    .context("migration: projects table")?;
    Ok(())
}

fn add_chats_project_id_column(conn: &Connection) -> Result<()> {
    if let Err(e) = conn.execute(
        "ALTER TABLE chats ADD COLUMN project_id TEXT REFERENCES projects(id) ON DELETE SET NULL",
        [],
    ) {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            return Err(e).with_context(|| "migration: chats.project_id");
        }
    }
    if let Err(e) = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_chats_project ON chats(project_id, updated_at DESC)",
        [],
    ) {
        let msg = e.to_string();
        if !msg.contains("already exists") {
            return Err(e).with_context(|| "migration: idx_chats_project");
        }
    }
    Ok(())
}

/// Sync one `models` row per configured cloud provider from `settings` (OpenAI, Anthropic, OpenRouter).
pub fn sync_cloud_models_from_settings(conn: &Connection) -> Result<()> {
    const SPECS: &[(&str, &str, &str, &str)] = &[
        ("cloud_openai", "lm-cloud-openai", "openai", "OpenAI"),
        ("cloud_anthropic", "lm-cloud-anthropic", "anthropic", "Anthropic"),
        ("cloud_openrouter", "lm-cloud-openrouter", "openrouter", "OpenRouter"),
        ("cloud_custom", "lm-cloud-custom", "custom", "Custom"),
    ];

    for &(setting_key, row_id, prov_slug, label) in SPECS {
        conn.execute("DELETE FROM models WHERE id = ?1", params![row_id])?;
        let raw = get_setting(conn, setting_key)?.unwrap_or_default();
        let cfg: Option<crate::cloud_infer::CloudProviderStored> = if raw.trim().is_empty() {
            None
        } else {
            serde_json::from_str(&raw).ok()
        };
        if let Some(c) = cfg {
            let key = c.api_key.trim();
            let model = c.model.trim();
            if model.is_empty() {
                continue;
            }
            if prov_slug != "custom" && key.is_empty() {
                continue;
            }
            if prov_slug == "custom" {
                let base = c.base_url.as_deref().unwrap_or("").trim();
                if base.is_empty() {
                    continue;
                }
            }
            let path = format!("cloud://{prov_slug}/{model}");
            let name = format!("{label} · {model}");
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO models (id, name, path, quant, size_bytes, created_at, weights_format, shard_index, shard_total, cloud_provider, cloud_api_model) VALUES (?1, ?2, ?3, NULL, NULL, ?4, 'cloud', NULL, NULL, ?5, ?6)",
                params![row_id, name, path, now, prov_slug, model],
            )?;
        }
    }
    Ok(())
}

/// When removing a cloud library row, clear stored API JSON so the modal and DB stay consistent.
pub fn clear_cloud_setting_for_model_id(conn: &Connection, id: &str) -> Result<()> {
    let key = match id {
        "lm-cloud-openai" => Some("cloud_openai"),
        "lm-cloud-anthropic" => Some("cloud_anthropic"),
        "lm-cloud-openrouter" => Some("cloud_openrouter"),
        "lm-cloud-custom" => Some("cloud_custom"),
        _ => None,
    };
    if let Some(k) = key {
        set_setting(conn, k, "")?;
    }
    Ok(())
}

pub fn ensure_defaults(conn: &Connection, models_dir: &Path) -> Result<()> {
    let pairs: [(&str, String); 14] = [
        (
            "models_dir",
            models_dir
                .to_str()
                .unwrap_or("models")
                .to_string(),
        ),
        ("context_dir", String::new()),
        ("data_dir", String::new()),
        ("n_ctx", "4096".into()),
        ("n_threads", "0".into()),
        ("n_gpu_layers", "0".into()),
        ("temperature", "0.7".into()),
        ("top_p", "0.9".into()),
        ("max_tokens", "768".into()),
        ("seed", "1234".into()),
        ("loaded_model_id", "".into()),
        ("theme", "dark".into()),
        ("language", "en".into()),
        ("startup", "restore".into()),
    ];
    for (k, v) in pairs {
        conn.execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2)",
            params![k, v],
        )?;
    }
    Ok(())
}

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
    let mut rows = stmt.query(params![key])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row.get(0)?))
    } else {
        Ok(None)
    }
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}
