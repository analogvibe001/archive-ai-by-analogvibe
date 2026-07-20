//! archive-ai-core — pure Rust port of Archive AI's business logic
//! (originally src/core/*.js). No GUI/Tauri dependency: this crate can be
//! built and unit-tested completely standalone (`cargo check` / `cargo test`
//! from this directory), independent of whether the Tauri desktop shell
//! itself builds on the current machine.

pub mod brand_detector;
pub mod date_extractor;
pub mod execution_engine;
pub mod logger;
pub mod project_analyzer;
pub mod scanner;
pub mod smb_resolver;
pub mod template_engine;
