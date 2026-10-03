//! 用固定 rustc 的选项表识别编译输入，不根据文件名或文件是否存在猜测位置参数。

use std::path::PathBuf;

/// `arguments` 不包含可执行文件名。调用者先移除 rustdoc 专用参数，其余参数仍按
/// rustc 的真实 arity 解析，因而 `--out-dir looks.rs` 等选项值不会成为源码候选。
pub fn source_file(arguments: &[String]) -> Result<Option<PathBuf>, String> {
    let mut options = rustc_session::getopts::Options::new();
    for option in rustc_session::config::rustc_optgroups() {
        option.apply(&mut options);
    }
    let matches = options
        .parse(arguments)
        .map_err(|error| format!("无法解析 rustc 输入参数：{error}"))?;
    match matches.free.as_slice() {
        [] => Ok(None),
        [source] if source == "-" => Ok(None),
        [source] => Ok(Some(PathBuf::from(source))),
        sources => Err(format!(
            "rustc 编译需要唯一输入文件，发现 {} 个位置参数：{}",
            sources.len(),
            sources.join("、")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiler_options_do_not_compete_with_extensionless_or_unicode_sources() {
        for source in ["entry", "entry.code", "目录 空格/入口.code", "source.rs"] {
            let args = [
                "--crate-name=example",
                "--crate-type",
                "lib",
                "--out-dir",
                "output.rs",
                "--cfg",
                "feature=\"value.rs\"",
                "--extern=alias=dependency.rs",
                "-Ldependency=search.rs",
                "--remap-path-prefix",
                "before.rs=after.rs",
                source,
            ]
            .map(str::to_owned);
            assert_eq!(source_file(&args).unwrap(), Some(PathBuf::from(source)));
        }
    }

    #[test]
    fn input_count_and_option_arity_follow_rustc() {
        assert_eq!(source_file(&["--print=cfg".into()]).unwrap(), None);
        assert_eq!(source_file(&["-".into()]).unwrap(), None);
        assert!(source_file(&["first".into(), "second".into()]).is_err());
        assert!(source_file(&["--out-dir".into()]).is_err());
        assert!(source_file(&["--unknown-option".into(), "entry".into()]).is_err());
        assert_eq!(
            source_file(&["--".into(), "-entry".into()]).unwrap(),
            Some(PathBuf::from("-entry"))
        );
    }
}
