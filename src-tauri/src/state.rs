use archive_ai_core::brand_detector::BrandStore;
use archive_ai_core::execution_engine::ExecutionEngine;
use archive_ai_core::logger::Logger;
use archive_ai_core::template_engine::TemplateStore;

/// Shared app state, managed by Tauri as `Arc<AppState>` so it can be cloned
/// cheaply into `spawn_blocking` tasks (scanning/analysis/file moves are all
/// blocking filesystem work).
pub struct AppState {
    pub logger: Logger,
    pub brands: BrandStore,
    pub templates: TemplateStore,
    pub execution: ExecutionEngine,
}

pub type Shared = std::sync::Arc<AppState>;
