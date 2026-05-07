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

    Ok(pool)
}
