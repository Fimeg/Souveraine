//! Concrete [`LlmProvider`](crate::bridge::provider::LlmProvider) implementations.
//!
//! `BifrostClient` (the OpenAI-compatible gateway client) implements the trait
//! in `bridge::bifrost`; the OAuth-riding ChatGPT provider lives here.

pub mod openai_oauth;
pub mod responses;
