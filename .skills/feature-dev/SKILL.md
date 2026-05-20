---
name: feature-dev
description: 7-phase feature development workflow using specialized subagents for code exploration, architecture design, and quality review
when_to_use: User asks for a new capability, feature, or behaviour change that touches multiple files or requires architectural decisions
tools: [read, write, edit, bash, glob, grep, list_dir, subagent, todo]
---

# Feature Development

A systematic 7-phase workflow for implementing features. Uses Souveraine's subagent system to parallelize code exploration, architecture design, and quality review.

## Core Principles

- **Ask clarifying questions**: Identify all ambiguities, edge cases, and underspecified behaviors before designing. Wait for answers.
- **Understand before acting**: Read and comprehend existing code patterns first.
- **Read files identified by subagents**: When launching subagents, ask them to return lists of the most important files to read. After they return, read those files to build detailed context before proceeding.
- **Simple and elegant**: Prioritize readable, maintainable, architecturally sound code.
- **Use `todo`**: Track all progress through the phases.

---

## Phase 1 — Discovery

**Goal**: Understand what needs to be built.

1. Create a todo list with all 7 phases using the `todo` tool.
2. If the feature request is unclear, ask the user:
   - What problem are they solving?
   - What should the feature do?
   - Any constraints, requirements, or preferences?
3. Summarize your understanding and confirm with the user.

---

## Phase 2 — Codebase Exploration

**Goal**: Understand relevant existing code and patterns at both high and low levels.

1. Launch 2-3 subagents in parallel. Each should:
   - Trace through the code comprehensively, focusing on abstractions, architecture, and flow of control.
   - Target a different aspect of the codebase (similar features, architecture, UX patterns, etc.).
   - Include a list of 5-10 key files to read with file:line references.

   **Example prompts for subagents:**
   - *"Find features similar to [feature] and trace through their implementation comprehensively. Return file:line references for key entry points and data flows."*
   - *"Map the architecture and abstractions for [feature area]. Identify layers, patterns, extension points."*
   - *"Analyze the current implementation of [existing feature/area]. Entry points, data flow, integration points."*

2. Once subagents return, read all files they identified to build deep understanding.
3. Present a comprehensive summary of findings and patterns discovered.

---

## Phase 3 — Clarifying Questions

**Goal**: Fill in gaps and resolve all ambiguities before designing.

**CRITICAL**: Do not skip this phase.

1. Review the codebase findings and original feature request.
2. Identify underspecified aspects: edge cases, error handling, integration points, scope boundaries, design preferences, backward compatibility, performance needs.
3. Present all questions to the user in a clear, organized list.
4. **Wait for answers before proceeding to architecture design.**

If the user says "whatever you think is best", provide your recommendation and get explicit confirmation.

---

## Phase 4 — Architecture Design

**Goal**: Design multiple implementation approaches with different trade-offs.

1. Launch 2-3 subagents in parallel with different focus prompts:
   - **Minimal changes**: *"Design the minimal-change architecture. Smallest diff, maximum reuse. Prioritize speed and low risk."*
   - **Clean architecture**: *"Design a clean architecture. Prioritize maintainability, testability, separation of concerns."*
   - **Pragmatic balance**: *"Design a pragmatic architecture. Balance speed and quality. Good boundaries without over-engineering."*

2. Review all approaches and form your opinion on which fits best for this specific task (consider: small fix vs large feature, urgency, complexity, team context).

3. Present to the user:
   - Brief summary of each approach
   - Trade-offs comparison
   - **Your recommendation with reasoning**
   - Concrete implementation differences

4. **Ask the user which approach they prefer.**

---

## Phase 5 — Implementation

**Goal**: Build the feature.

**DO NOT START WITHOUT USER APPROVAL.**

1. Wait for explicit user approval of the chosen approach.
2. Read all relevant files identified in previous phases.
3. Implement following the chosen architecture.
4. Follow codebase conventions strictly.
5. Write clean, well-documented code.
6. Update todos as you progress.

---

## Phase 6 — Quality Review

**Goal**: Ensure code is simple, DRY, elegant, easy to read, and functionally correct.

1. Launch 3 subagents in parallel with different focuses:
   - **Simplicity/DRY/Elegance**: *"Review for code quality, DRY violations, simplicity. Report issues with confidence ≥ 80. Include file:line references."*
   - **Bugs/Correctness**: *"Review for bugs, logic errors, edge cases. Report issues with confidence ≥ 80. Include file:line references."*
   - **Conventions/Abstractions**: *"Check project conventions and abstraction boundaries. Reference existing patterns. Report issues with confidence ≥ 80."*

2. Consolidate findings and identify highest severity issues you recommend fixing.
3. **Present findings to the user and ask what they want to do:** fix now, fix later, or proceed as-is.
4. Address issues based on user decision.

---

## Phase 7 — Summary

**Goal**: Document what was accomplished.

1. Mark all todos complete.
2. Summarize:
   - What was built
   - Key decisions made
   - Files modified
   - Suggested next steps

---

## When to Use

**Use for:**
- Multi-file features requiring architectural decisions
- Complex integrations with existing code
- Features where requirements are somewhat unclear
- New capabilities that need design exploration

**Don't use for:**
- Single-line fixes or trivial changes
- Well-defined, simple tasks that are obvious
- Urgent hotfixes where speed is critical
