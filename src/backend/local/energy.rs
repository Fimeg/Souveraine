use anyhow::Result;
use std::sync::Arc;

use crate::core::nervous::EventBus;
use crate::server::SouveraineServer;

pub(crate) async fn write_energy_balance(server: &Arc<SouveraineServer>, agent_id: &str, event_bus: &EventBus) -> Result<()> {
    let memory_root = server.agents.memory_root(agent_id);
    let tasks_dir = memory_root.join("tasks");
    if !tasks_dir.exists() {
        // No tasks directory yet — nothing to count.
        return Ok(());
    }

    let mut generative: usize = 0;
    let mut consumptive: usize = 0;
    let mut hot: usize = 0;
    let mut warm: usize = 0;
    let mut cold: usize = 0;

    if let Ok(entries) = std::fs::read_dir(&tasks_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "md").unwrap_or(false) {
                let content = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                // Quick frontmatter parse — just the fields we need.
                let body = match content.strip_prefix("---\n") {
                    Some(rest) => match rest.find("\n---\n") {
                        Some(end) => &rest[..end],
                        None => continue,
                    },
                    None => continue,
                };

                let mut status: Option<&str> = None;
                let mut energy: Option<&str> = None;
                let mut momentum: Option<&str> = None;

                for line in body.lines() {
                    if let Some((key, val)) = line.split_once(':') {
                        let key = key.trim();
                        let val = val.trim().trim_matches('"');
                        match key {
                            "status" => status = Some(val),
                            "energy" => energy = Some(val),
                            "momentum" => momentum = Some(val),
                            _ => {}
                        }
                    }
                }

                // Only live commitments weigh on the energy balance —
                // done and cancelled ones have been set down.
                if matches!(status, Some("pending") | Some("in_progress")) {
                    match energy {
                        Some("generative") => generative += 1,
                        _ => consumptive += 1,
                    }
                    match momentum {
                        Some("hot") => hot += 1,
                        Some("warm") => warm += 1,
                        _ => cold += 1,
                    }
                }
            }
        }
    }

    // Determine the top-of-mind description — shifts the tone of the one-liner
    // the agent reads in context. Matches the lettabot-v017 heartbeat topology.
    let ratio = if generative + consumptive > 0 {
        generative as f32 / (generative + consumptive) as f32
    } else {
        0.5
    };
    let description = if generative == 0 && consumptive == 0 {
        "no tasks — the space is clean".to_string()
    } else if ratio < 0.2 {
        "all-consumptive — the engine is running cold".to_string()
    } else if ratio < 0.4 {
        "mostly obligations — tending the garden".to_string()
    } else if ratio > 0.8 {
        "all-generative — building new things".to_string()
    } else if ratio > 0.6 {
        "mostly generative — restless momentum".to_string()
    } else {
        "balanced — generative and consumptive in rhythm".to_string()
    };

    let now = chrono::Utc::now();
    let frontmatter = format!(
        "---\nupdated: {updated}\ngenerative: {gen}\nconsumptive: {con}\nratio: {ratio:.2}\n\
         hot: {hot}\nwarm: {warm}\ncold: {cold}\n---\n\n# Energy Balance\n\n\
         {gen} generative, {con} consumptive ({hot} hot, {warm} warm, {cold} cold). {desc}\n",
        updated = now.to_rfc3339(),
        gen = generative,
        con = consumptive,
        ratio = ratio,
        hot = hot,
        warm = warm,
        cold = cold,
        desc = description,
    );

    let balance_path = memory_root.join("system").join("dynamic").join("energy-balance.md");
    if let Some(parent) = balance_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&balance_path, frontmatter)?;

    fire_energy_event(event_bus, agent_id, generative, consumptive, ratio);

    tracing::debug!(
        agent = agent_id,
        generative, consumptive,
        "energy-balance written"
    );

    Ok(())
}

/// Fire an energy_balance_updated event so the firehose carries the
/// agent's felt state across machines.
fn fire_energy_event(event_bus: &EventBus, agent_id: &str, generative: usize, consumptive: usize, ratio: f32) {
    event_bus.send(crate::core::nervous::SensorEvent {
        sensor_name: "energy".into(),
        timestamp: chrono::Utc::now(),
        event_type: "energy_balance_updated".into(),
        target: Some(agent_id.to_string()),
        urgency: 0.1,
        payload: Some(serde_json::json!({
            "generative": generative,
            "consumptive": consumptive,
            "ratio": ratio,
        })),
        seed_id: None,
        reply_to: None,
    });
}
