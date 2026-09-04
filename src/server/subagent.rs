use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

use crate::bridge::openai_compatible::{ChatCompletionRequest, Message as WireMessage};
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

        // A model must carry its wire with it. Both arms used to take
        // `default_provider()`, so an override was a name change and not a
        // route change: `kimi-k3` was sent to Anthropic, which answered
        // `404 model: kimi-k3` (2026-08-14). That failure was loud only by
        // luck — a name the default provider happens to recognise would have
        // run the wrong model in silence. The inherit arm was wrong the same
        // way, and quietly: a fork of an agent whose provider is not the
        // default went to the wrong wire with a model that provider does not
        // serve.
        //
        // An unroutable override is refused rather than guessed at, which is
        // `for_model`'s own instruction — "no entry" and "wrong entry" both
        // mean don't guess a wire for this model.
        let (model, llm) = match params.model {
            Some(m) => {
                let provider = self.server.providers.for_model(&m).ok_or_else(|| {
                    crate::core::tools::defs::ToolError::invalid_input(&format!(
                        "No provider is configured for model `{m}`. Add a \
                         [models.\"{m}\"] entry naming its provider, or omit \
                         `model` to inherit the parent's."
                    ))
                })?;
                (m, provider)
            }
            None => (
                agent.llm_config.model.clone(),
                self.server.providers.for_agent(&agent),
            ),
        };
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
             Your final message will be returned to the caller.",
            params.parent_agent_id
        );

        // Build tool definitions
        let core_tools = crate::core::tools::tool_definitions().await;
        let wire_tools: Vec<crate::bridge::openai_compatible::ToolDefinition> = core_tools
            .iter()
            .map(|t| crate::bridge::openai_compatible::ToolDefinition {
                tool_type: "function".to_string(),
                function: crate::bridge::openai_compatible::ToolFunction {
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
            Some(
                Arc::new(ServerSubagentRunner::new(self.server.clone())) as Arc<dyn SubagentRunner>
            ),
        );

        // Initial messages: system prompt + user prompt.
        // The task must be a *user* message: providers that hoist system
        // messages into a separate field are left with an empty messages
        // array otherwise, and reject the request.
        let mut messages = vec![
            WireMessage::text("system", system_prompt),
            WireMessage::text("user", params.prompt.clone()),
        ];

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
                messages.push(WireMessage::text(
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
                messages.push(WireMessage::text(
                    "system",
                    format!(
                        "[subagent awareness] I'm at {} of {} tool rounds. \
                         This is my last chance to produce a final answer \
                         before my fork returns what I have.",
                        tool_round, max_tool_rounds
                    ),
                ));
            }

            // A fork inherits the parent's logical principal intent, but its
            // live uid is still observed afresh. The block is request-local
            // and therefore cannot be replayed after the fork returns.
            let principal = crate::core::principal::observe(&agent, "subagent");
            let mut request_messages = messages.clone();
            let system_prefix = request_messages
                .iter()
                .take_while(|message| message.role == "system")
                .count();
            request_messages.insert(
                system_prefix,
                WireMessage::text("system", principal.model_system_block()),
            );

            let req = ChatCompletionRequest {
                model: model.clone(),
                messages: request_messages,
                stream: Some(false),
                max_tokens: None,
                temperature,
                tools: Some(wire_tools.clone()),
            };

            let response = llm.chat_completion(req).await.map_err(|e| {
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
            let calls: Vec<crate::bridge::openai_compatible::MessageToolCall> = response
                .tool_calls
                .iter()
                .map(|tc| {
                    crate::bridge::openai_compatible::MessageToolCall::function(
                        tc.id.clone(),
                        tc.name.clone(),
                        tc.arguments.to_string(),
                    )
                })
                .collect();
            messages.push(
                WireMessage::assistant_tool_calls(response.content.clone(), calls)
                    .with_thinking(
                        response.reasoning.clone(),
                        response.reasoning_signature.clone(),
                    ),
            );

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

                messages.push(WireMessage::tool_result(&tc.id, &tc.name, output));
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
