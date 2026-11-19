// One integration-test binary instead of ~140.
//
// Every integration-test target statically links the whole library; with
// one target per file that was ~140 links per build. Auto-discovery is
// off (autotests = false in Cargo.toml) and this file mounts each test
// file as a module instead. Test files stay where they were: #[path]
// resolves relative to this file's directory, so proptest's
// file!()-derived regression paths keep working unchanged. Shared helpers
// live in the `support` module, mounted once here — a per-file
// #[path = "support/..."] mount would duplicate the helper per includer
// (clippy::duplicate_mod).
//
// Adding a test file means adding its mod line here; the
// suite_completeness test at the bottom pins that this list stays
// complete.
//
// One binary means one process under plain `cargo test`, and this crate has
// process-global state (the active-theme arc-swap in src/config/theme.rs)
// that some tests store to while others render from it. The process's own
// stdin is the second such piece: `process_runner_gives_the_child_a_null_stdin`
// in gcloud_runner_test.rs replaces fd 0 with a pipe for the length of the
// test, because a child that inherits a `/dev/null` stdin and one that is
// given it are indistinguishable. No other test reads fd 0, so
// nothing races with it today — but under bare `cargo test` that is an
// argument rather than the isolation nextest provides. The tiers and CI
// therefore run this suite under nextest, which gives each test its own
// process; a rare theme-flavored flake under bare `cargo test` is that race,
// not the test.
//
// nextest is REQUIRED by both tiers, not preferred. They used to
// fall back to libtest with a printed note, which ran the suite in exactly
// the racing configuration and then closed green — so the isolation above is
// a property of every gating path rather than of whatever the machine had
// installed. Bare `cargo test` remains the one unisolated way to run this
// suite, deliberately and only by hand.

mod support;

#[path = "app_error_kind_test.rs"]
mod app_error_kind_test;
#[path = "architecture_layout_evidence.rs"]
mod architecture_layout_evidence;
#[path = "architecture_layout_test.rs"]
mod architecture_layout_test;
#[path = "argv_option_guard_evidence.rs"]
mod argv_option_guard_evidence;
#[path = "batch_cancel_evidence.rs"]
mod batch_cancel_evidence;
#[path = "batch_state_test.rs"]
mod batch_state_test;
#[path = "batch_supersession_evidence.rs"]
mod batch_supersession_evidence;
#[path = "bigquery_drilldown_test.rs"]
mod bigquery_drilldown_test;
#[path = "bigquery_enrichment_test.rs"]
mod bigquery_enrichment_test;
#[path = "bigquery_rendering_test.rs"]
mod bigquery_rendering_test;
#[path = "bigquery_schema_test.rs"]
mod bigquery_schema_test;
#[path = "bigquery_service_test.rs"]
mod bigquery_service_test;
#[path = "bigquery_state_test.rs"]
mod bigquery_state_test;
#[path = "chrome_line_fitting_property_test.rs"]
mod chrome_line_fitting_property_test;
#[path = "ci_gates_test.rs"]
mod ci_gates_test;
#[path = "ci_reliability_evidence.rs"]
mod ci_reliability_evidence;
#[path = "cli_test.rs"]
mod cli_test;
#[path = "cloud_functions_rendering_test.rs"]
mod cloud_functions_rendering_test;
#[path = "cloud_functions_service_test.rs"]
mod cloud_functions_service_test;
#[path = "cloud_functions_state_test.rs"]
mod cloud_functions_state_test;
#[path = "cloud_run_parse_test.rs"]
mod cloud_run_parse_test;
#[path = "cloud_run_reducer_test.rs"]
mod cloud_run_reducer_test;
#[path = "cloud_run_service_test.rs"]
mod cloud_run_service_test;
#[path = "cloud_run_state_test.rs"]
mod cloud_run_state_test;
#[path = "cloudsql_detail_parse_test.rs"]
mod cloudsql_detail_parse_test;
#[path = "cloudsql_evidence.rs"]
mod cloudsql_evidence;
#[path = "cloudsql_parse_test.rs"]
mod cloudsql_parse_test;
#[path = "cloudsql_service_test.rs"]
mod cloudsql_service_test;
#[path = "column_catalog_test.rs"]
mod column_catalog_test;
#[path = "column_layout_test.rs"]
mod column_layout_test;
#[path = "columns_reload_test.rs"]
mod columns_reload_test;
#[path = "compute_detail_parse_test.rs"]
mod compute_detail_parse_test;
#[path = "compute_parse_test.rs"]
mod compute_parse_test;
#[path = "compute_service_test.rs"]
mod compute_service_test;
#[path = "config_aliases_test.rs"]
mod config_aliases_test;
#[path = "config_columns_test.rs"]
mod config_columns_test;
#[path = "config_evidence.rs"]
mod config_evidence;
#[path = "config_hotkeys_test.rs"]
mod config_hotkeys_test;
#[path = "config_info_test.rs"]
mod config_info_test;
#[path = "config_jumps_test.rs"]
mod config_jumps_test;
#[path = "config_plugins_test.rs"]
mod config_plugins_test;
#[path = "config_read_guard_evidence.rs"]
mod config_read_guard_evidence;
#[path = "config_reload_test.rs"]
mod config_reload_test;
#[path = "config_reload_theme_test.rs"]
mod config_reload_theme_test;
#[path = "config_resolution_test.rs"]
mod config_resolution_test;
#[path = "config_startup_test.rs"]
mod config_startup_test;
#[path = "config_theme_test.rs"]
mod config_theme_test;
#[path = "config_warnings_evidence.rs"]
mod config_warnings_evidence;
#[path = "configuration_form_cursor_test.rs"]
mod configuration_form_cursor_test;
#[path = "configuration_parse_test.rs"]
mod configuration_parse_test;
#[path = "configurations_table_render_test.rs"]
mod configurations_table_render_test;
#[path = "confirmation_wrapping_test.rs"]
mod confirmation_wrapping_test;
#[path = "context_cache_evidence.rs"]
mod context_cache_evidence;
#[path = "context_credentials_test.rs"]
mod context_credentials_test;
#[path = "context_picker_cursor_identity_test.rs"]
mod context_picker_cursor_identity_test;
#[path = "context_picker_favorites_render_test.rs"]
mod context_picker_favorites_render_test;
#[path = "copy_diagnostic_test.rs"]
mod copy_diagnostic_test;
#[path = "credential_naming_evidence.rs"]
mod credential_naming_evidence;
#[path = "cross_jumps_evidence.rs"]
mod cross_jumps_evidence;
#[path = "delta_test.rs"]
mod delta_test;
#[path = "demo_runner_test.rs"]
mod demo_runner_test;
#[path = "demo_sdk_fixture_test.rs"]
mod demo_sdk_fixture_test;
#[path = "demo_showcase_test.rs"]
mod demo_showcase_test;
#[path = "detached_fetch_evidence.rs"]
mod detached_fetch_evidence;
#[path = "detail_field_projection_test.rs"]
mod detail_field_projection_test;
#[path = "detail_field_selection_test.rs"]
mod detail_field_selection_test;
#[path = "detail_request_test.rs"]
mod detail_request_test;
#[path = "diagnostics_classify_test.rs"]
mod diagnostics_classify_test;
#[path = "diagnostics_generation_test.rs"]
mod diagnostics_generation_test;
#[path = "diagnostics_probe_test.rs"]
mod diagnostics_probe_test;
#[path = "discovery_service_test.rs"]
mod discovery_service_test;
#[path = "doc_pinning_evidence.rs"]
mod doc_pinning_evidence;
#[path = "docs_completeness_test.rs"]
mod docs_completeness_test;
#[path = "documentation_test.rs"]
mod documentation_test;
#[path = "effect_dispatch_test.rs"]
mod effect_dispatch_test;
#[path = "effect_tasks_in_flight_test.rs"]
mod effect_tasks_in_flight_test;
#[path = "empty_state_ladder_evidence.rs"]
mod empty_state_ladder_evidence;
#[path = "entrypoint_evidence.rs"]
mod entrypoint_evidence;
#[path = "exit_code_evidence.rs"]
mod exit_code_evidence;
#[path = "filter_cache_evidence.rs"]
mod filter_cache_evidence;
#[path = "filter_searchable_fields_test.rs"]
mod filter_searchable_fields_test;
#[path = "finops_card_band_test.rs"]
mod finops_card_band_test;
#[path = "finops_projection_test.rs"]
mod finops_projection_test;
#[path = "finops_reducer_test.rs"]
mod finops_reducer_test;
#[path = "finops_service_test.rs"]
mod finops_service_test;
#[path = "finops_state_test.rs"]
mod finops_state_test;
#[path = "finops_workspace_test.rs"]
mod finops_workspace_test;
#[path = "firewall_narrowing_test.rs"]
mod firewall_narrowing_test;
#[path = "freshness_evidence.rs"]
mod freshness_evidence;
#[path = "gcloud_call_audit.rs"]
mod gcloud_call_audit;
#[path = "gcloud_runner_test.rs"]
mod gcloud_runner_test;
#[path = "gcs_parse_test.rs"]
mod gcs_parse_test;
#[path = "gcs_service_test.rs"]
mod gcs_service_test;
#[path = "generation_test.rs"]
mod generation_test;
#[path = "gke_parse_test.rs"]
mod gke_parse_test;
#[path = "gke_service_test.rs"]
mod gke_service_test;
#[path = "grammar_reconcile_evidence.rs"]
mod grammar_reconcile_evidence;
#[path = "help_drill_depth_evidence.rs"]
mod help_drill_depth_evidence;
#[path = "help_service_scope_test.rs"]
mod help_service_scope_test;
#[path = "home_overview_projection_test.rs"]
mod home_overview_projection_test;
#[path = "iam_policy_rendering_test.rs"]
mod iam_policy_rendering_test;
#[path = "iam_policy_state_test.rs"]
mod iam_policy_state_test;
#[path = "iam_rendering_test.rs"]
mod iam_rendering_test;
#[path = "iam_service_test.rs"]
mod iam_service_test;
#[path = "iam_state_test.rs"]
mod iam_state_test;
#[path = "interactive_glue_evidence.rs"]
mod interactive_glue_evidence;
#[path = "interactive_process_test.rs"]
mod interactive_process_test;
#[path = "jump_instance_network_fields_test.rs"]
mod jump_instance_network_fields_test;
#[path = "jump_list_target_test.rs"]
mod jump_list_target_test;
#[path = "jump_rules_evidence.rs"]
mod jump_rules_evidence;
#[path = "k9s_launch_test.rs"]
mod k9s_launch_test;
#[path = "keymap_help_parity_test.rs"]
mod keymap_help_parity_test;
#[path = "keymap_mode_implication_property_test.rs"]
mod keymap_mode_implication_property_test;
#[path = "keymap_modifier_evidence.rs"]
mod keymap_modifier_evidence;
#[path = "keymap_test.rs"]
mod keymap_test;
#[path = "lifecycle_parity_evidence.rs"]
mod lifecycle_parity_evidence;
#[path = "list_filter_test.rs"]
mod list_filter_test;
#[path = "log_domain_test.rs"]
mod log_domain_test;
#[path = "log_export_test.rs"]
mod log_export_test;
#[path = "log_query_test.rs"]
mod log_query_test;
#[path = "log_viewer_evidence.rs"]
mod log_viewer_evidence;
#[path = "logging_evidence.rs"]
mod logging_evidence;
#[path = "logging_floor_test.rs"]
mod logging_floor_test;
#[path = "logging_initial_selection_test.rs"]
mod logging_initial_selection_test;
#[path = "logging_projection_test.rs"]
mod logging_projection_test;
#[path = "logging_read_order_test.rs"]
mod logging_read_order_test;
#[path = "overlay_backdrop_test.rs"]
mod overlay_backdrop_test;
#[path = "status_label_rendering_test.rs"]
mod status_label_rendering_test;

#[path = "logging_service_test.rs"]
mod logging_service_test;
#[path = "logging_state_test.rs"]
mod logging_state_test;
#[path = "logviewer_debt_evidence.rs"]
mod logviewer_debt_evidence;
#[path = "logviewer_visual_evidence.rs"]
mod logviewer_visual_evidence;
#[path = "matches_filter_walk_test.rs"]
mod matches_filter_walk_test;
#[path = "native_list_cap_test.rs"]
mod native_list_cap_test;
#[path = "nav_history_evidence.rs"]
mod nav_history_evidence;
#[path = "navigation_state_test.rs"]
mod navigation_state_test;
#[path = "operations_log_test.rs"]
mod operations_log_test;
#[path = "operations_settlement_property_test.rs"]
mod operations_settlement_property_test;
#[path = "overview_diagnostics_state_test.rs"]
mod overview_diagnostics_state_test;
#[path = "overview_system_state_test.rs"]
mod overview_system_state_test;
#[path = "palette_evidence.rs"]
mod palette_evidence;
#[path = "palette_log_entries_evidence.rs"]
mod palette_log_entries_evidence;
#[path = "persistent_disks_rendering_test.rs"]
mod persistent_disks_rendering_test;
#[path = "persistent_disks_service_test.rs"]
mod persistent_disks_service_test;
#[path = "persistent_disks_state_test.rs"]
mod persistent_disks_state_test;
#[path = "picker_render_evidence.rs"]
mod picker_render_evidence;
#[path = "picker_viewport_drift_test.rs"]
mod picker_viewport_drift_test;
#[path = "picker_viewport_evidence.rs"]
mod picker_viewport_evidence;
#[path = "picker_viewport_test.rs"]
mod picker_viewport_test;
#[path = "plugin_bool_prompt_evidence.rs"]
mod plugin_bool_prompt_evidence;
#[path = "plugin_cancellation_evidence.rs"]
mod plugin_cancellation_evidence;
#[path = "plugin_confirmation_identity_evidence.rs"]
mod plugin_confirmation_identity_evidence;
#[path = "plugin_execution_evidence.rs"]
mod plugin_execution_evidence;
#[path = "plugin_input_schema_evidence.rs"]
mod plugin_input_schema_evidence;
#[path = "plugin_numeric_prompt_evidence.rs"]
mod plugin_numeric_prompt_evidence;
#[path = "plugin_numeric_prompt_render_evidence.rs"]
mod plugin_numeric_prompt_render_evidence;
#[path = "plugin_output_evidence.rs"]
mod plugin_output_evidence;
#[path = "plugin_prefix_evidence.rs"]
mod plugin_prefix_evidence;
#[path = "plugin_preview_evidence.rs"]
mod plugin_preview_evidence;
#[path = "plugin_prompt_evidence.rs"]
mod plugin_prompt_evidence;
#[path = "plugin_read_only_evidence.rs"]
mod plugin_read_only_evidence;
#[path = "plugin_refresh_evidence.rs"]
mod plugin_refresh_evidence;
#[path = "project_theme_override_render_evidence.rs"]
mod project_theme_override_render_evidence;
#[path = "project_theme_overrides_test.rs"]
mod project_theme_overrides_test;
#[path = "pubsub_rendering_test.rs"]
mod pubsub_rendering_test;
#[path = "pubsub_service_test.rs"]
mod pubsub_service_test;
#[path = "pubsub_state_test.rs"]
mod pubsub_state_test;
#[path = "read_path_bench.rs"]
mod read_path_bench;
#[path = "reducer_command_test.rs"]
mod reducer_command_test;
#[path = "reducer_configurations_test.rs"]
mod reducer_configurations_test;
#[path = "reducer_logging_test.rs"]
mod reducer_logging_test;
#[path = "reducer_navigation_test.rs"]
mod reducer_navigation_test;
#[path = "reducer_navigator_test.rs"]
mod reducer_navigator_test;
#[path = "reducer_overview_probe_test.rs"]
mod reducer_overview_probe_test;
#[path = "reducer_resources_test.rs"]
mod reducer_resources_test;
#[path = "refresh_refusal_evidence.rs"]
mod refresh_refusal_evidence;
#[path = "reload_wording_evidence.rs"]
mod reload_wording_evidence;
#[path = "remembered_view_evidence.rs"]
mod remembered_view_evidence;

#[path = "render_bench.rs"]
mod render_bench;
#[path = "rendered_column_alignment_test.rs"]
mod rendered_column_alignment_test;
#[path = "reschedule_test.rs"]
mod reschedule_test;
#[path = "resource_effect_test.rs"]
mod resource_effect_test;
#[path = "resource_jump_picker_test.rs"]
mod resource_jump_picker_test;
#[path = "resource_jumps_test.rs"]
mod resource_jumps_test;
#[path = "resource_load_phase_test.rs"]
mod resource_load_phase_test;
#[path = "resource_marks_test.rs"]
mod resource_marks_test;
#[path = "resource_overview_accessors_test.rs"]
mod resource_overview_accessors_test;
#[path = "resource_projection_test.rs"]
mod resource_projection_test;
#[path = "resource_selection_logs_test.rs"]
mod resource_selection_logs_test;
#[path = "resource_service_trait_test.rs"]
mod resource_service_trait_test;
#[path = "retry_gate_test.rs"]
mod retry_gate_test;
#[path = "runtime_batch_effect_test.rs"]
mod runtime_batch_effect_test;
#[path = "runtime_cloudsql_action_effect_test.rs"]
mod runtime_cloudsql_action_effect_test;
#[path = "runtime_compute_action_effect_test.rs"]
mod runtime_compute_action_effect_test;
#[path = "runtime_configuration_effect_test.rs"]
mod runtime_configuration_effect_test;
#[path = "runtime_input_context_test.rs"]
mod runtime_input_context_test;
#[path = "runtime_interactive_test.rs"]
mod runtime_interactive_test;
#[path = "runtime_native_effect_test.rs"]
mod runtime_native_effect_test;

#[path = "runtime_logging_effect_test.rs"]
mod runtime_logging_effect_test;
#[path = "runtime_logging_tail_test.rs"]
mod runtime_logging_tail_test;
#[path = "runtime_tunnel_effect_test.rs"]
mod runtime_tunnel_effect_test;
#[path = "sdk_stub_seam_test.rs"]
mod sdk_stub_seam_test;

#[path = "sdk_bridge_test.rs"]
mod sdk_bridge_test;

#[path = "sdk_client_cache_test.rs"]
mod sdk_client_cache_test;
#[path = "sdk_cloud_run_mapper_test.rs"]
mod sdk_cloud_run_mapper_test;
#[path = "sdk_cloudsql_mapper_test.rs"]
mod sdk_cloudsql_mapper_test;
#[path = "sdk_compute_mapper_test.rs"]
mod sdk_compute_mapper_test;
#[path = "sdk_detail_redaction_test.rs"]
mod sdk_detail_redaction_test;
#[path = "sdk_discovery_mapper_test.rs"]
mod sdk_discovery_mapper_test;
#[path = "sdk_disks_mapper_test.rs"]
mod sdk_disks_mapper_test;
#[path = "sdk_finops_mapper_test.rs"]
mod sdk_finops_mapper_test;
#[path = "sdk_functions_mapper_test.rs"]
mod sdk_functions_mapper_test;
#[path = "sdk_gke_mapper_test.rs"]
mod sdk_gke_mapper_test;
#[path = "sdk_helpers_test.rs"]
mod sdk_helpers_test;
#[path = "sdk_iam_mapper_test.rs"]
mod sdk_iam_mapper_test;
#[path = "sdk_logging_mapper_test.rs"]
mod sdk_logging_mapper_test;
#[path = "sdk_pubsub_mapper_test.rs"]
mod sdk_pubsub_mapper_test;
#[path = "sdk_secrets_mapper_test.rs"]
mod sdk_secrets_mapper_test;
#[path = "sdk_shared_primitives_test.rs"]
mod sdk_shared_primitives_test;
#[path = "sdk_storage_mapper_test.rs"]
mod sdk_storage_mapper_test;
#[path = "sdk_vpc_mapper_test.rs"]
mod sdk_vpc_mapper_test;
#[path = "secrets_detail_state_test.rs"]
mod secrets_detail_state_test;
#[path = "secrets_no_payload_access_test.rs"]
mod secrets_no_payload_access_test;
#[path = "secrets_rendering_test.rs"]
mod secrets_rendering_test;
#[path = "secrets_service_test.rs"]
mod secrets_service_test;
#[path = "secrets_state_test.rs"]
mod secrets_state_test;
#[path = "service_capability_parity_test.rs"]
mod service_capability_parity_test;
#[path = "service_history_test.rs"]
mod service_history_test;
#[path = "service_kind_exhaustiveness_test.rs"]
mod service_kind_exhaustiveness_test;
#[path = "service_navigator_projection_test.rs"]
mod service_navigator_projection_test;
#[path = "service_navigator_render_test.rs"]
mod service_navigator_render_test;
#[path = "service_registry_test.rs"]
mod service_registry_test;
#[path = "shutdown_cancellation_evidence.rs"]
mod shutdown_cancellation_evidence;
#[path = "sort_column_invariant_test.rs"]
mod sort_column_invariant_test;
#[path = "sparkline_test.rs"]
mod sparkline_test;
#[path = "state_writer_evidence.rs"]
mod state_writer_evidence;
#[path = "storage_evidence.rs"]
mod storage_evidence;
#[path = "table_cell_sanitising_evidence.rs"]
mod table_cell_sanitising_evidence;
#[path = "table_filter_evidence.rs"]
mod table_filter_evidence;
#[path = "table_layout_test.rs"]
mod table_layout_test;
#[path = "test_isolation_evidence.rs"]
mod test_isolation_evidence;
#[path = "theme_override_render_evidence.rs"]
mod theme_override_render_evidence;
#[path = "theme_overrides_test.rs"]
mod theme_overrides_test;
#[path = "tunnel_launch_test.rs"]
mod tunnel_launch_test;
#[path = "tunnel_real_child_evidence.rs"]
mod tunnel_real_child_evidence;
#[path = "tunnel_stop_failure_evidence.rs"]
mod tunnel_stop_failure_evidence;
#[path = "tunnels_projection_test.rs"]
mod tunnels_projection_test;
#[path = "tunnels_reducer_test.rs"]
mod tunnels_reducer_test;
#[path = "ui_finops_render_test.rs"]
mod ui_finops_render_test;
#[path = "ui_frame_test.rs"]
mod ui_frame_test;
#[path = "ui_help_test.rs"]
mod ui_help_test;
#[path = "ui_home_render_test.rs"]
mod ui_home_render_test;
#[path = "ui_jump_picker_render_test.rs"]
mod ui_jump_picker_render_test;
#[path = "ui_key_discoverability_test.rs"]
mod ui_key_discoverability_test;
#[path = "ui_logging_render_test.rs"]
mod ui_logging_render_test;
#[path = "ui_projection_test.rs"]
mod ui_projection_test;
#[path = "ui_tunnels_render_test.rs"]
mod ui_tunnels_render_test;
#[path = "vpc_firewall_rendering_test.rs"]
mod vpc_firewall_rendering_test;
#[path = "vpc_firewall_service_test.rs"]
mod vpc_firewall_service_test;
#[path = "vpc_firewall_state_test.rs"]
mod vpc_firewall_state_test;
#[path = "vpc_networks_rendering_test.rs"]
mod vpc_networks_rendering_test;
#[path = "vpc_networks_state_test.rs"]
mod vpc_networks_state_test;
#[path = "vpc_service_test.rs"]
mod vpc_service_test;
#[path = "vpc_subnets_rendering_test.rs"]
mod vpc_subnets_rendering_test;
#[path = "vpc_subnets_service_test.rs"]
mod vpc_subnets_service_test;
#[path = "vpc_subnets_state_test.rs"]
mod vpc_subnets_state_test;
#[path = "workspace_lint_evidence.rs"]
mod workspace_lint_evidence;
#[path = "wrap_exact_property_test.rs"]
mod wrap_exact_property_test;

/// The mod list above is what makes a test file exist as far as cargo is
/// concerned now that auto-discovery is off: a file missing from it compiles
/// nowhere and silently stops running. This pin turns that silence into a
/// failure naming the file.
mod suite_completeness {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    fn mounted_files(suite_source: &str) -> BTreeSet<String> {
        suite_source
            .lines()
            .filter_map(|line| {
                line.trim()
                    .strip_prefix("#[path = \"")?
                    .strip_suffix("\"]")
                    .map(str::to_string)
            })
            .collect()
    }

    fn files_on_disk(tests_dir: &Path) -> BTreeSet<String> {
        fs::read_dir(tests_dir)
            .expect("tests/ directory is readable")
            .map(|entry| entry.expect("tests/ entry is readable").file_name())
            .filter_map(|name| {
                let name = name
                    .to_str()
                    .expect("test file names are UTF-8")
                    .to_string();
                (name.ends_with(".rs") && name != "suite.rs").then_some(name)
            })
            .collect()
    }

    #[test]
    fn every_test_file_is_mounted_in_the_suite() {
        let tests_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
        let suite_source =
            fs::read_to_string(tests_dir.join("suite.rs")).expect("tests/suite.rs is readable");

        let mounted = mounted_files(&suite_source);
        let on_disk = files_on_disk(&tests_dir);

        let unmounted: Vec<_> = on_disk.difference(&mounted).collect();
        let ghosts: Vec<_> = mounted.difference(&on_disk).collect();

        assert!(
            unmounted.is_empty(),
            "test files not mounted in tests/suite.rs (they compile nowhere and never run); \
             add a #[path] mod line for each: {unmounted:?}"
        );
        assert!(
            ghosts.is_empty(),
            "tests/suite.rs mounts files that do not exist on disk: {ghosts:?}"
        );
    }
}
