pub mod commands;
pub mod domain;
pub mod indexer;
pub mod knowledge;
pub mod project_identity;
pub mod providers;
pub mod runtime;
pub mod scanner;
pub mod search;
pub mod storage;

use commands::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::new().expect("open local index"))
        .invoke_handler(tauri::generate_handler![
            commands::scan,
            commands::list_projects,
            commands::list_provider_descriptors,
            commands::set_project_alias,
            commands::get_session,
            commands::get_session_branch,
            commands::global_search,
            commands::get_knowledge_card,
            commands::update_knowledge_card,
            commands::related_sessions,
            commands::semantic_search,
            commands::list_hidden_sessions,
            commands::set_session_hidden,
            commands::rename_session,
            commands::set_session_pinned,
            commands::touch_session,
            commands::status,
            commands::get_index_diagnostics,
            commands::get_scan_settings,
            commands::update_scan_settings,
            commands::activate_claude_source_root,
            commands::get_claude_settings,
            commands::update_claude_settings,
            commands::resume_preflight,
            commands::start_continuation,
            commands::start_fork_continuation,
            commands::poll_continuation,
            commands::write_continuation,
            commands::resize_continuation,
            commands::close_continuation
        ])
        .run(tauri::generate_context!())
        .expect("error while running Session Deck");
}
