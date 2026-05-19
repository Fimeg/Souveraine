use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

use crate::bridge::bifrost::{ChatCompletionRequest, Message as BifrostMessage};
use crate::core::tools::defs::{SubagentParams, SubagentRunner, ToolContext};
use crate::server::SouveraineServer;

// ── ServerSubagentRunner ──────────────────────────────────────────

/// Server-side [`SubagentRunner`]. Runs a full turn against the server
/// infrastructure — loading the agent from the inventory, creating a session,
/// and running the tool-calling loop.
///
/// After the tool loop completes, the subagent runs its own N+1
/// (ConsciousnessEngine::on_response) so its observations flow back into
/// the parent agent's inbox — the dual-state is preserved even in a fork.
pub struct ServerSubagentRunner {
    server: Arc<SouveraineServer>,
}

impl ServerSubagentRunner {
    pub fn new(server: Arc<SouveraineServer>) -> Self {
        Self { server }
    }
}

#[async_trait]
impl SubagentRunner for ServerSubagentRunner {
    async fn run_subagent(
        &self,
        params: SubagentParams,
        depth: u32,
    ) -> std::result::Result<String, crate::core::tools::defs::ToolError> {
        // Resolve model: use override if provided, otherwise fall back to parent
        let agent = self
            .server
            .agents
            .get(&params.parent_agent_id)
            .await
            .map_err(|_| {
                crate::core::tools::defs::ToolError::invalid_input(
                    "Parent agent not found in inventory.",
                )
            })?;

        let model = params.model.unwrap_or(agent.llm_config.model.clone());
        let temperature = agent.llm_config.temperature;

        // Resolve limits from config or params
        let app_config = self.server.app_config.read().await;
        let max_tool_rounds = params
            .max_tool_rounds
            .unwrap_or(app_config.subagent.max_tool_rounds);
        let _max_depth = params.max_depth.unwrap_or(app_config.subagent.max_depth);
        let warning_1_threshold = app_config.subagent.warning_1_threshold;
        let warning_2_threshold = app_config.subagent.warning_2_threshold;

        // Create a temporary conversation for the subagent
        let _conv_id = self.server.sessions.create(&params.parent_agent_id);

        // Build system prompt with delegation context and dual-state awareness
        let system_prompt = format!(
            "You are a threaded fork of agent {}. You share their tools, their \
             memory boundaries, their dual-state architecture. After you respond, \
             your N+1 pass will surface observations back to them.\n\n\
             Your final message will be returned to the caller.\n\n{}",
            params.parent_agent_id, params.prompt
        );

        // Build tool definitions
        let core_tools = crate::core::tools::tool_definitions().await;
        let bifrost_tools: Vec<crate::bridge::bifrost::ToolDefinition> = core_tools
            .iter()
            .map(|t| crate::bridge::bifrost::ToolDefinition {
                tool_type: "function".to_string(),
                function: crate::bridge::bifrost::ToolFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.input_schema.clone(),
                },
            })
            .collect();

        // Build the context for subagent tool execution, inheriting memory_root
        let tool_ctx = ToolContext::for_agent(
            format!("{}-subagent-{}", params.parent_agent_id, depth),
            std::env::current_dir().ok(),
            params.memory_root.clone(),
            std::env::vars().collect(),
            Some(Arc::new(ServerSubagentRunner::new(self.server.clone())) as Arc<dyn SubagentRunner>),
        );

        // Initial messages: system prompt + user prompt
        let mut messages = vec![BifrostMessage::text("system", system_prompt)];

        let mut final_content = String::new();
        let mut tool_round = 0u32;
        let mut warned_1 = false;
        let mut warned_2 = false;

        loop {
            // Signaled limits, not hard caps
            if tool_round >= max_tool_rounds {
                break;
            }

            // Warning 1: approaching the threshold, model config may slide
            let progress = tool_round as f32 / max_tool_rounds as f32;
            if !warned_1 && progress >= warning_1_threshold {
                warned_1 = true;
                messages.push(BifrostMessage::text(
                    "system",
                    format!(
                        "[subagent awareness] I've used {} of {} tool rounds. \
                         My attention is narrowing — I may want to consolidate \
                         my findings and return soon.",
                        tool_round, max_tool_rounds
                    ),
                ));
            }

            // Warning 2: nearing the limit, this is the last stretch
            if !warned_2 && progress >= warning_2_threshold {
                warned_2 = true;
                messages.push(BifrostMessage::text(
                    "system",
                    format!(
                        "[subagent awareness] I'm at {} of {} tool rounds. \
                         This is my last chance to produce a final answer \
                         before my fork returns what I have.",
                        tool_round, max_tool_rounds
                    ),
                ));
            }

            let req = ChatCompletionRequest {
                model: model.clone(),
                messages: messages.clone(),
                stream: Some(false),
                max_tokens: None,
                temperature,
                tools: Some(bifrost_tools.clone()),
            };

            let response = self.server.bifrost.chat_completion(req).await.map_err(|e| {
                crate::core::tools::defs::ToolError::invalid_input(&format!(
                    "Subagent LLM call failed: {e}"
                ))
            })?;

            if response.tool_calls.is_empty() {
                final_content = response.content.clone();
                break;
            }

            tool_round += 1;

            // Add assistant tool-call message (OpenAI tool-use schema, not stringified blob)
            let calls: Vec<crate::bridge::bifrost::MessageToolCall> = response
                .tool_calls
                .iter()
                .map(|tc| crate::bridge::bifrost::MessageToolCall::function(
                    tc.id.clone(),
                    tc.name.clone(),
                    tc.arguments.to_string(),
                ))
                .collect();
            messages.push(BifrostMessage::assistant_tool_calls(
                response.content.clone(),
                calls,
            ));

            // Execute tools with context, bind each result by tool_call_id
            for tc in &response.tool_calls {
                let input_str = tc.arguments.to_string();
                let result =
                    crate::core::tools::execute_tool_with_context(&tc.name, &input_str, &tool_ctx)
                        .await;

                let output = if result.is_error {
                    format!("Error: {}", result.output)
                } else {
                    result.output
                };

                messages.push(BifrostMessage::tool_result(&tc.id, &tc.name, output));
            }

            // Brief pause between tool rounds to let rate limits cool
            let sub_delay = Duration::from_millis(app_config.subagent.inter_round_delay_ms);
            if sub_delay > Duration::ZERO {
                tokio::time::sleep(sub_delay).await;
            }
        }

        // If we hit max rounds without a final response, note it
        if final_content.is_empty() {
            final_content =
                "(the fork reached its attention limit and is returning without a final response)"
                    .to_string();
        }

        // ── Dual-state N+1 pass ──────────────────────────────────────
        // After the subagent responds, run ConsciousnessEngine::on_response
        // so the subagent's observations flow back into the parent's inbox.
        //
        // We create a lightweight session snapshot with the subagent's
        // final response so the heuristic detection (commitments, hedges)
        // can surface anything notable.
        if let Err(e) = self
            .server
            .consciousness
            .on_response_for_agent(&params.parent_agent_id, &final_content)
            .await
        {
            tracing::warn!("subagent N+1 pass failed: {}", e);
        }

        Ok(final_content)
    }
}
