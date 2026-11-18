// Shared helpers for the consolidated test suite. Before tests/suite.rs
// existed each test binary mounted these files with its own #[path] mod;
// inside one crate that pattern compiles every helper once per includer and
// trips clippy::duplicate_mod, so the suite mounts them here exactly once.
pub(crate) mod architecture_layout;
pub(crate) mod frame_svg;
pub(crate) mod jump_rules;
pub(crate) mod keymap_context;
pub(crate) mod process;
pub(crate) mod remembered_view;
pub(crate) mod theme;
pub(crate) mod user_docs;
