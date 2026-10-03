//! Merge generated Rust Analyzer settings without replacing unrelated JSONC text.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    ops::Range,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::{Value, json};

/// 进程内临时文件序号，与进程 ID 共同避免配置写入文件名冲突。
static TEMPORARIES: AtomicU64 = AtomicU64::new(0);

/// 合并生成设置并保留 JSONC 文本；写入前备份一次，再核对原文件未被并发修改。
pub(crate) fn configure(
    workspace_root: &Path,
    project_path: &Path,
    check_command: Vec<String>,
    check_env: &BTreeMap<String, String>,
    macro_server: &Path,
) -> Result<PathBuf, String> {
    let path = workspace_root.join(".vscode/settings.json");
    let original = match fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let source = match &original {
        Some(bytes) => std::str::from_utf8(bytes).map_err(|error| {
            format!(
                "{} is not UTF-8; settings were not changed: {error}",
                path.display()
            )
        })?,
        None => "{}\n",
    };
    let utf8 = |value: &Path| {
        value
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("IDE paths must be UTF-8: {}", value.display()))
    };
    let environment = merge_check_environment(source, check_env).map_err(|error| {
        format!(
            "cannot merge {}: {error}; settings were not changed",
            path.display()
        )
    })?;
    let desired = [
        ("rust-analyzer.linkedProjects", json!([utf8(project_path)?])),
        ("rust-analyzer.check.overrideCommand", json!(check_command)),
        ("rust-analyzer.check.extraEnv", environment),
        ("rust-analyzer.cargo.cfgs", json!([])),
        ("rust-analyzer.cfg.setTest", json!(false)),
        ("rust-analyzer.procMacro.enable", json!(true)),
        ("rust-analyzer.procMacro.server", json!(utf8(macro_server)?)),
        ("rust-analyzer.checkOnSave", json!(true)),
    ];
    let updated = merge(source, &desired).map_err(|error| {
        format!(
            "cannot merge {}: {error}; settings were not changed",
            path.display()
        )
    })?;
    if updated == source {
        return Ok(path);
    }
    let directory = path.parent().expect("settings file has a parent");
    fs::create_dir_all(directory)
        .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;

    if let Some(bytes) = &original {
        let backup = directory.join("settings.json.nestrs.bak");
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup)
        {
            Ok(mut file) => file
                .write_all(bytes)
                .and_then(|()| file.sync_all())
                .map_err(|error| format!("cannot back up {}: {error}", backup.display()))?,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(format!("cannot back up {}: {error}", backup.display())),
        }
    }
    let temporary = directory.join(format!(
        "settings.json.nestrs.{}.{}.tmp",
        std::process::id(),
        TEMPORARIES.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        if original.is_some() {
            file.set_permissions(
                fs::metadata(&path)
                    .map_err(|error| error.to_string())?
                    .permissions(),
            )
            .map_err(|error| error.to_string())?;
        }
        file.write_all(updated.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| error.to_string())?;
        drop(file);
        let current = match fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => return Err(error.to_string()),
        };
        if current != original {
            return Err("settings changed while the IDE configuration was being generated; retry the command".into());
        }
        fs::rename(&temporary, &path).map_err(|error| error.to_string())
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(format!("cannot write {}: {error}", path.display()));
    }
    Ok(path)
}

/// 固定所选工具路径，同时保留已有 check.extraEnv 中的其他变量。
fn merge_check_environment(
    source: &str,
    check_env: &BTreeMap<String, String>,
) -> Result<Value, String> {
    let (sanitized, _) = sanitize(source)?;
    let data: Value = serde_json::from_slice(&sanitized)
        .map_err(|error| format!("invalid JSON/JSONC: {error}"))?;
    let settings = data.as_object().ok_or("settings must be a JSON object")?;
    let mut environment = match settings.get("rust-analyzer.check.extraEnv") {
        Some(Value::Object(values)) => values.clone(),
        Some(_) => return Err("rust-analyzer.check.extraEnv must be a JSON object".into()),
        None => serde_json::Map::new(),
    };
    environment.extend(
        check_env
            .iter()
            .map(|(key, value)| (key.clone(), Value::String(value.clone()))),
    );
    Ok(Value::Object(environment))
}

/// 按原始字节范围修改指定设置，保留无关文本与注释并校验最终 JSONC。
fn merge(source: &str, desired: &[(&str, Value)]) -> Result<String, String> {
    let (sanitized, comments) = sanitize(source)?;
    let data: Value = serde_json::from_slice(&sanitized)
        .map_err(|error| format!("invalid JSON/JSONC: {error}"))?;
    let object = data.as_object().ok_or("settings must be a JSON object")?;
    let open = whitespace(&sanitized, 0);
    let mut cursor = whitespace(&sanitized, open + 1);
    let mut fields = BTreeMap::new();
    while sanitized[cursor] != b'}' {
        let key_end = string_end(&sanitized, cursor)?;
        let key: String = serde_json::from_slice(&sanitized[cursor..key_end])
            .map_err(|error| error.to_string())?;
        cursor = whitespace(&sanitized, key_end);
        let start = whitespace(&sanitized, cursor + 1);
        let end = value_end(&sanitized, start)?;
        if fields.insert(key.clone(), start..end).is_some() {
            return Err(format!("duplicate setting {key:?} is ambiguous"));
        }
        cursor = whitespace(&sanitized, end);
        if sanitized[cursor] == b',' {
            cursor = whitespace(&sanitized, cursor + 1);
        }
    }
    let mut edits = Vec::new();
    let mut missing = Vec::new();
    let mut names = BTreeSet::new();
    for &(name, ref value) in desired {
        if !names.insert(name) {
            return Err(format!("duplicate generated setting {name:?}"));
        }
        if object.get(name) == Some(value) {
            continue;
        }
        let encoded = serde_json::to_string(value).map_err(|error| error.to_string())?;
        if let Some(range) = fields.get(name) {
            // Preserve comments even when replacing an existing array/object.
            // Leading/trailing comments outside the value remain byte-for-byte.
            let mut replacement = String::new();
            for comment in &comments {
                if range.start <= comment.start && comment.end <= range.end {
                    replacement.push_str(&source[comment.clone()]);
                    replacement.push_str("\n  ");
                }
            }
            replacement.push_str(&encoded);
            edits.push((range.clone(), replacement));
        } else {
            missing.push(format!(
                "  {}: {encoded}",
                serde_json::to_string(name).unwrap()
            ));
        }
    }
    if !missing.is_empty() {
        let mut insertion = format!("\n{}", missing.join(",\n"));
        if !fields.is_empty() {
            insertion.push(',');
        }
        insertion.push('\n');
        edits.push((open + 1..open + 1, insertion));
    }
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut result = source.to_owned();
    for (range, replacement) in edits {
        result.replace_range(range, &replacement);
    }
    // Validate our own edits before any backup or settings file is written.
    let (verified, _) = sanitize(&result)?;
    let _: Value = serde_json::from_slice(&verified).map_err(|error| error.to_string())?;
    Ok(result)
}

/// Replace JSONC comments and trailing commas with whitespace, preserving byte
/// offsets. serde_json then remains responsible for every actual JSON rule.
fn sanitize(source: &str) -> Result<(Vec<u8>, Vec<Range<usize>>), String> {
    let mut bytes = source.as_bytes().to_vec();
    let mut comments = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] == b'"' {
            cursor = string_end(&bytes, cursor)?;
        } else if bytes[cursor..].starts_with(b"//") {
            let start = cursor;
            while cursor < bytes.len() && bytes[cursor] != b'\n' {
                cursor += 1;
            }
            comments.push(start..cursor);
        } else if bytes[cursor..].starts_with(b"/*") {
            let start = cursor;
            cursor += 2;
            while cursor < bytes.len() && !bytes[cursor..].starts_with(b"*/") {
                cursor += 1;
            }
            if cursor == bytes.len() {
                return Err("unterminated JSONC block comment".into());
            }
            cursor += 2;
            comments.push(start..cursor);
        } else {
            cursor += 1;
        }
    }
    for range in &comments {
        for byte in &mut bytes[range.clone()] {
            if *byte != b'\n' && *byte != b'\r' {
                *byte = b' ';
            }
        }
    }
    cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] == b'"' {
            cursor = string_end(&bytes, cursor)?;
        } else {
            if bytes[cursor] == b',' {
                let next = whitespace(&bytes, cursor + 1);
                if bytes
                    .get(next)
                    .is_some_and(|byte| *byte == b'}' || *byte == b']')
                {
                    bytes[cursor] = b' ';
                }
            }
            cursor += 1;
        }
    }
    Ok((bytes, comments))
}

/// 跳过 JSON 允许的 ASCII 空白并返回下一个字节位置。
fn whitespace(bytes: &[u8], mut cursor: usize) -> usize {
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    cursor
}

/// 定位 JSON 字符串结束位置，跳过转义字符且拒绝未闭合字符串。
fn string_end(bytes: &[u8], start: usize) -> Result<usize, String> {
    let mut cursor = start + 1;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'"' => return Ok(cursor + 1),
            b'\\' => cursor += 2,
            _ => cursor += 1,
        }
    }
    Err("unterminated JSON string".into())
}

/// 定位单个 JSON 值的结束边界，使用显式深度计数扫描数组和对象。
fn value_end(bytes: &[u8], start: usize) -> Result<usize, String> {
    if bytes[start] == b'"' {
        return string_end(bytes, start);
    }
    if bytes[start] != b'{' && bytes[start] != b'[' {
        let mut cursor = start;
        while cursor < bytes.len()
            && !bytes[cursor].is_ascii_whitespace()
            && !b",}]".contains(&bytes[cursor])
        {
            cursor += 1;
        }
        return Ok(cursor);
    }
    let mut depth = 0usize;
    let mut cursor = start;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'"' => {
                cursor = string_end(bytes, cursor)?;
                continue;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(cursor + 1);
                }
            }
            _ => {}
        }
        cursor += 1;
    }
    Err("unterminated JSON value".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Workspace(PathBuf);

    impl Workspace {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "nestrs IDE workspace {} {}",
                std::process::id(),
                TEMPORARIES.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(path.join(".vscode")).unwrap();
            Self(path)
        }

        fn settings(&self) -> PathBuf {
            self.0.join(".vscode/settings.json")
        }

        fn configure(&self, check: &str) -> Result<PathBuf, String> {
            configure(
                &self.0,
                &self.0.join("target/project with spaces.json"),
                vec![
                    self.0
                        .join("tool path/cargo-nestrs")
                        .to_str()
                        .unwrap()
                        .into(),
                    check.into(),
                ],
                &BTreeMap::from([
                    ("NESTRS_DRIVER".into(), "/tools/nestrs-driver".into()),
                    (
                        "NESTRS_MACRO_BRIDGE".into(),
                        "/tools/private-bridge.so".into(),
                    ),
                    ("NESTRS_RUSTC".into(), "/tools/rustc".into()),
                ]),
                &self.0.join("tool path/rust-analyzer-proc-macro-srv"),
            )
        }
    }

    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn preserves_unrelated_jsonc_settings_and_all_comments() {
        let workspace = Workspace::new();
        let original = r#"{
  // Keep this user note.
  "editor.tabSize": 4,
  "files.exclude": { "**/generated": true, },
  "rust-analyzer.diagnostics.enable": true,
  "rust-analyzer.linkedProjects": [
    /* project explanation */ "old.json", // inline note
  ],
  "path-looking-like-comment": "https://example.test/a/*literal*/",
}
"#;
        fs::write(workspace.settings(), original).unwrap();
        workspace.configure("check").unwrap();
        let text = fs::read_to_string(workspace.settings()).unwrap();
        for retained in [
            "// Keep this user note.",
            "/* project explanation */",
            "// inline note",
            "\"editor.tabSize\": 4,",
            "\"files.exclude\": { \"**/generated\": true, },",
            "\"rust-analyzer.diagnostics.enable\": true,",
            "https://example.test/a/*literal*/",
        ] {
            assert!(text.contains(retained), "lost {retained}: {text}");
        }
        let data: Value = serde_json::from_slice(&sanitize(&text).unwrap().0).unwrap();
        assert_eq!(data["rust-analyzer.checkOnSave"], true);
        assert_eq!(data["rust-analyzer.procMacro.enable"], true);
        assert_eq!(data["rust-analyzer.cargo.cfgs"], json!([]));
        assert_eq!(data["rust-analyzer.cfg.setTest"], false);
        assert_eq!(
            data["rust-analyzer.linkedProjects"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(!text.contains("old.json"));
        assert_eq!(
            fs::read_to_string(workspace.0.join(".vscode/settings.json.nestrs.bak")).unwrap(),
            original
        );
    }

    #[test]
    fn creates_valid_settings_with_space_paths_and_preserves_argv_boundaries() {
        let workspace = Workspace::new();
        let output = workspace.configure("check").unwrap();
        let data: Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
        assert_eq!(
            data["rust-analyzer.linkedProjects"][0],
            workspace
                .0
                .join("target/project with spaces.json")
                .to_str()
                .unwrap()
        );
        assert_eq!(
            data["rust-analyzer.procMacro.server"],
            workspace
                .0
                .join("tool path/rust-analyzer-proc-macro-srv")
                .to_str()
                .unwrap()
        );
        assert_eq!(
            data["rust-analyzer.check.overrideCommand"],
            json!([
                workspace.0.join("tool path/cargo-nestrs").to_str().unwrap(),
                "check"
            ])
        );
        assert_eq!(
            data["rust-analyzer.check.extraEnv"]["NESTRS_DRIVER"],
            "/tools/nestrs-driver"
        );
        assert!(data.get("rust-analyzer.diagnostics.enable").is_none());
        assert!(
            !workspace
                .0
                .join(".vscode/settings.json.nestrs.bak")
                .exists()
        );
    }

    #[test]
    fn malformed_or_ambiguous_settings_are_not_overwritten_or_backed_up() {
        let workspace = Workspace::new();
        for original in [
            "{ broken",
            "[]",
            "{ /* missing end",
            r#"{"editor.tabSize": 2, "editor.tabSize": 4}"#,
            r#"{"rust-analyzer.check.extraEnv": "not-an-object"}"#,
            r#"{"rust-analyzer.check.extraEnv": null}"#,
            r#"{"rust-analyzer.check.extraEnv": []}"#,
        ] {
            fs::write(workspace.settings(), original).unwrap();
            let error = workspace.configure("check").unwrap_err();
            assert!(error.contains("settings were not changed"), "{error}");
            assert_eq!(fs::read_to_string(workspace.settings()).unwrap(), original);
            assert!(
                !workspace
                    .0
                    .join(".vscode/settings.json.nestrs.bak")
                    .exists()
            );
        }
    }

    #[test]
    fn pins_selected_tools_without_removing_user_environment_or_comments() {
        let workspace = Workspace::new();
        let original = r#"{
  "rust-analyzer.check.extraEnv": {
    // The application still needs its custom mode.
    "APP_MODE": "local",
    "NESTRS_DRIVER": "old-driver", /* replace just the tool choice */
  },
  "rust-analyzer.cargo.extraEnv": { "CARGO_NET_OFFLINE": "true" },
}
"#;
        fs::write(workspace.settings(), original).unwrap();
        workspace.configure("check").unwrap();
        let text = fs::read_to_string(workspace.settings()).unwrap();
        assert!(text.contains("// The application still needs its custom mode."));
        assert!(text.contains("/* replace just the tool choice */"));
        let data: Value = serde_json::from_slice(&sanitize(&text).unwrap().0).unwrap();
        assert_eq!(data["rust-analyzer.check.extraEnv"]["APP_MODE"], "local");
        assert_eq!(
            data["rust-analyzer.check.extraEnv"]["NESTRS_DRIVER"],
            "/tools/nestrs-driver"
        );
        assert_eq!(
            data["rust-analyzer.check.extraEnv"]["NESTRS_MACRO_BRIDGE"],
            "/tools/private-bridge.so"
        );
        assert_eq!(
            data["rust-analyzer.check.extraEnv"]["NESTRS_RUSTC"],
            "/tools/rustc"
        );
        assert_eq!(
            data["rust-analyzer.cargo.extraEnv"]["CARGO_NET_OFFLINE"],
            "true"
        );
        workspace.configure("check").unwrap();
        assert_eq!(fs::read_to_string(workspace.settings()).unwrap(), text);
    }

    #[test]
    fn backup_is_created_once_and_repeated_configuration_is_idempotent() {
        let workspace = Workspace::new();
        let original = "{\"editor.tabSize\": 4}\n";
        fs::write(workspace.settings(), original).unwrap();
        workspace.configure("check").unwrap();
        let once = fs::read(workspace.settings()).unwrap();
        workspace.configure("check").unwrap();
        assert_eq!(fs::read(workspace.settings()).unwrap(), once);
        workspace.configure("check-again").unwrap();
        assert_eq!(
            fs::read_to_string(workspace.0.join(".vscode/settings.json.nestrs.bak")).unwrap(),
            original
        );
    }
}
