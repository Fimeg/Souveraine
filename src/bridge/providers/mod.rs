//! Concrete [`LlmProvider`](crate::bridge::provider::LlmProvider) implementations.
//!
//! `OpenAiCompatibleClient` (the OpenAI-compatible gateway client) implements the trait
//! in `bridge::openai_compatible`; the OAuth-riding ChatGPT provider lives here.

pub mod openai_oauth;
pub mod responses;
