//! 問題收集（Intake）：外部問題進獨立佇列，循序派給終端機裡的 AI Agent。
//! 設計：docs/superpowers/specs/2026-11-05-intake-queue-design.md
pub mod filter;
pub mod rules;
pub mod store;
