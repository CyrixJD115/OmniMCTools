//! `omt-bedrock` — Bedrock world storage for OmniMCTools.

pub mod keys;
pub mod store;
pub mod subchunk;

pub use keys::DbKey;
pub use store::BedrockWorld;
pub use subchunk::{Data3d, Subchunk};

/// Default Bedrock worlds directory on Windows.
pub const WINDOWS_WORLDS_DIR: &str = "%LOCALAPPDATA%\\Packages\\Microsoft.MinecraftUWP_8wekyb3d8bbwe\\LocalState\\games\\com.mojang\\minecraftWorlds";
