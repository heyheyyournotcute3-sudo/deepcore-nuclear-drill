//! Loading, validating and looking up game content.

pub mod defs;
pub mod ids;
pub mod registry;

pub use defs::*;
pub use ids::*;
pub use registry::{Content, ContentError, CONTENT_FILES};
