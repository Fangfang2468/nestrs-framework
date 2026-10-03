//! Editor projects derived from Cargo's actual compilation units.
//!
//! The private macro bridge is an editor dependency, never a Cargo dependency
//! of the application. Compiler capture is opt-in and records only build data.

mod capture;

#[doc(hidden)]
pub mod constructor;

mod project;

mod settings;

pub use capture::capture_rustc;
pub(crate) use project::generate;
pub(crate) use settings::configure;

use std::{fs, path::Path};

/// 内容未变时不写文件；变化时以同目录临时文件替换，避免无效 IDE 重载。
pub(crate) fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), String> {
    // A project-file write triggers a rust-analyzer reload. Unchanged save-time
    // checks must not restart analysis or create a check/reload feedback loop.
    if fs::read(path).is_ok_and(|previous| previous == contents) {
        return Ok(());
    }
    let parent = path.parent().ok_or("output path has no parent")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary, contents).map_err(|error| error.to_string())?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("cannot replace {}: {error}", path.display()));
    }
    Ok(())
}
