mod core;
pub(crate) mod tile_manager;

#[cfg(feature = "ai")]
pub use core::PersistedAiSettings;
#[cfg(not(target_arch = "wasm32"))]
pub use core::start_with_options;
pub use core::{StartOptions, start};
