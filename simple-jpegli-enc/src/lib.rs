pub mod encoder;
mod ffi;

pub use encoder::*;
use ffi::get_jpegli_version;
pub use ffi::{ColorSpace, Subsampling};

#[derive(Debug, Clone)]
pub struct VersionInfo<'a> {
    version: (u32, u32, u32),
    lib_version: u32,
    sha: Option<&'a str>,
}

impl<'a> VersionInfo<'a> {
    pub fn version_string(&self) -> String {
        format!("{}.{}.{}", self.version.0, self.version.1, self.version.2)
    }

    pub fn lib_version(&self) -> u32 {
        self.lib_version
    }

    pub fn git_sha(&self) -> Option<&'a str> {
        self.sha
    }
}

/// Retrieves the version of the underlying jpegli library.
///
/// # Example
/// ```rust
/// let jxl_ver = simple_jpegli_enc::get_version();
/// assert_eq!(jxl_ver.version_string(), "0.12.0");
/// assert_eq!(jxl_ver.lib_version(), 80);
/// ```
pub fn get_version() -> VersionInfo<'static> {
    let (version, lib_version) = get_jpegli_version();
    let sha = option_env!("SJPEGLI_COMMIT_SHA");
    VersionInfo {
        version,
        lib_version,
        sha,
    }
}
