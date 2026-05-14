//! CLI command implementations

use anyhow::Result;
use serde::Serialize;
use std::path::PathBuf;

/// Report for model list (JSON output)
#[derive(Debug, Serialize)]
pub struct ModelListReport {
    pub selected: String,
    pub bifrost_discovered: Vec<String>,
    pub configured: Vec<String>,
    pub all_models: Vec<ModelReport>,
}

#[derive(Debug, Serialize)]
pub struct ModelReport {
    pub name: String,
    pub provider: String,
    pub context_limit: usize,
    pub output_limit: usize,
    pub from_bifrost: bool,
}

/// List or set models
pub async fn run_model_command(
    model_name: Option<&str>,
    emit_json: bool,
    verbose: bool,
) -> Result<()> {
    use crate::bridge::bifrost::BifrostClient;
    use crate::core::config::ConsciousnessConfig;

    // Load config
    let config_path = std::env::current_dir()
        .map(|d| d.join("souveraine.toml"))
        .unwrap_or_else(|_| PathBuf::from("souveraine.toml"));
    let config = ConsciousnessConfig::load(&config_path)?;

    // Create Bifrost client
    let bifrost = BifrostClient::new(
        &config.bifrost.base_url,
        &config.bifrost.api_key,
        &config.bifrost.virtual_key,
        &config.bifrost.primary_model,
        config.bifrost.timeout_secs,
    );

    // Fetch models from Bifrost
    let bifrost_models = bifrost.list_models().await.unwrap_or_default();

    // Merge with configured models
    let mut all_models = bifrost_models.clone();
    for name in config.models.keys() {
        if !all_models.contains(name) {
            all_models.push(name.clone());
        }
    }

    // Handle model selection
    if let Some(name) = model_name {
        if !all_models.contains(&name.to_string()) {
            anyhow::bail!(
                "Model '{}' not found. Available: {:?}",
                name,
                all_models
            );
        }

        // Update config and save
        let mut new_config = config.clone();
        new_config.bifrost.primary_model = name.to_string();
        new_config.save(&config_path)?;

        println!("Selected model: {}", name);
        return Ok(());
    }

    // List models
    if emit_json {
        let report = ModelListReport {
            selected: config.bifrost.primary_model.clone(),
            bifrost_discovered: bifrost_models.clone(),
            configured: config.models.keys().cloned().collect(),
            all_models: all_models
                .iter()
                .map(|name| {
                    let cfg = config.models.get(name);
                    ModelReport {
                        name: name.clone(),
                        provider: cfg
                            .map(|c| c.provider.clone())
                            .unwrap_or_else(|| "bifrost".to_string()),
                        context_limit: cfg
                            .map(|c| c.context_limit)
                            .unwrap_or(128_000),
                        output_limit: cfg
                            .map(|c| c.output_limit)
                            .unwrap_or(8_192),
                        from_bifrost: bifrost_models.contains(name),
                    }
                })
                .collect(),
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        if verbose {
            println!("Selected: {}", config.bifrost.primary_model);
            println!("Bifrost discovered: {}", bifrost_models.len());
            println!("Configured models: {}", config.models.len());
            println!("Total available: {}", all_models.len());
            println!();
        }
        for model in all_models {
            let marker = if bifrost_models.contains(&model) {
                "⚡"
            } else {
                "⚙️"
            };
            println!("{} {}", marker, model);
        }
    }

    Ok(())
}
