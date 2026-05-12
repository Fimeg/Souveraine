use sqlx::{migrate::MigrateDatabase, sqlite::SqlitePoolOptions, Sqlite, SqlitePool};
use std::path::Path;

pub async fn init_database(db_path: &Path) -> anyhow::Result<SqlitePool> {
    let db_url = format!("sqlite:{}", db_path.display());

    if !db_path.exists() {
        Sqlite::create_database(&db_url).await?;
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS agents (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            llm_model TEXT,
            context_window INTEGER DEFAULT 128000,
            tags TEXT,
            is_active BOOLEAN DEFAULT 1,
            config_json TEXT
        );
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS conversations (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            message_count INTEGER DEFAULT 0,
            is_active BOOLEAN DEFAULT 1,
            metadata_json TEXT,
            FOREIGN KEY (agent_id) REFERENCES agents(id)
        );
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS sessions (
            conversation_id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            started_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            last_activity TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            turn_count INTEGER DEFAULT 0,
            context_pressure REAL DEFAULT 0.0,
            FOREIGN KEY (conversation_id) REFERENCES conversations(id),
            FOREIGN KEY (agent_id) REFERENCES agents(id)
        );
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_agents_updated ON agents(updated_at DESC)")
        .execute(&pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_conversations_agent ON conversations(agent_id, updated_at DESC)")
        .execute(&pool)
        .await?;

    // Instance registry — one row per running souveraine process per agent.
    // last_seen_at is heartbeated; rows stale > 5 min are pruned on startup.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS agent_instances (
            agent_id     TEXT NOT NULL,
            instance_id  TEXT NOT NULL,
            pid          INTEGER NOT NULL,
            hostname     TEXT NOT NULL,
            started_at   TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            last_seen_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            PRIMARY KEY (agent_id, instance_id)
        );
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_instances_agent ON agent_instances(agent_id, last_seen_at DESC)")
        .execute(&pool)
        .await?;

    // lifetime_active_seconds: cumulative time this agent has had at least
    // one running instance. Incremented by the heartbeat tick. Drives the
    // uptime % on the manager card (capped at 99 in the UI).
    add_column_if_missing(&pool, "agents", "lifetime_active_seconds", "INTEGER NOT NULL DEFAULT 0").await?;

    Ok(pool)
}

/// SQLite-friendly idempotent column add. PRAGMA table_info is checked first
/// because SQLite has no `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`.
async fn add_column_if_missing(
    pool: &SqlitePool,
    table: &str,
    column: &str,
    column_decl: &str,
) -> anyhow::Result<()> {
    let rows: Vec<(i64, String)> = sqlx::query_as(
        &format!("SELECT cid, name FROM pragma_table_info('{table}')"),
    )
    .fetch_all(pool)
    .await?;
    if rows.iter().any(|(_, name)| name == column) {
        return Ok(());
    }
    sqlx::query(&format!("ALTER TABLE {table} ADD COLUMN {column} {column_decl}"))
        .execute(pool)
        .await?;
    Ok(())
}
