//! 项目配置的独立回归：固定默认值、package 边界和无歧义的错误反馈。

use cargo_nestrs::project_config::{DiConfig, parse_manifest, read_manifest};
use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};

fn parse_di(body: &str) -> Result<DiConfig, String> {
    parse_manifest(&format!("[nestrs-cli]\n{body}"))
}

#[test]
fn absent_or_empty_config_uses_lazy_and_32() {
    for manifest in ["", "[package]\nname = 'example'", "[nestrs-cli]"] {
        assert_eq!(parse_manifest(manifest).unwrap(), DiConfig::default());
    }
}

#[test]
fn both_initialization_modes_and_limits_are_supported() {
    for (name, eager) in [("lazy", false), ("eager", true)] {
        for (scope_name, scope_eager) in [("lazy", false), ("eager", true)] {
            for limit in [1, 32, 128] {
                let config = parse_di(&format!(
                    "initialization = '{name}'\nscope-initialization = '{scope_name}'\nmax-concurrent-activations = {limit}"
                ))
                .unwrap();
                assert_eq!(
                    config,
                    DiConfig {
                        eager,
                        scope_eager,
                        max_concurrent_activations: limit,
                    }
                );
            }
        }
    }
}

#[test]
fn individual_fields_keep_other_defaults() {
    assert_eq!(
        parse_di("initialization = 'eager'").unwrap(),
        DiConfig {
            eager: true,
            ..DiConfig::default()
        }
    );
    assert_eq!(
        parse_di("scope-initialization = 'eager'").unwrap(),
        DiConfig {
            scope_eager: true,
            ..DiConfig::default()
        }
    );
    assert_eq!(
        parse_di("max-concurrent-activations = 7").unwrap(),
        DiConfig {
            max_concurrent_activations: 7,
            ..DiConfig::default()
        }
    );
}

#[test]
fn workspace_and_dependency_config_are_not_inherited() {
    let workspace = "\
[workspace.nestrs-cli]
initialization = 'eager'
scope-initialization = 'eager'
max-concurrent-activations = 5
[dependencies.library]
path = '../library'
[dependencies.library.nestrs-cli]
initialization = 'eager'
scope-initialization = 'eager'
";
    assert_eq!(parse_manifest(workspace).unwrap(), DiConfig::default());
    assert_eq!(
        parse_manifest(&format!(
            "{workspace}\n[nestrs-cli]\nmax-concurrent-activations = 9"
        ))
        .unwrap(),
        DiConfig {
            max_concurrent_activations: 9,
            ..DiConfig::default()
        }
    );
}

#[test]
fn unknown_cli_config_keys_are_rejected() {
    for key in [
        "max_concurrent_activations",
        "initialisation",
        "scope_initialization",
        "future-option",
    ] {
        let error = parse_di(&format!("{key} = 10")).unwrap_err();
        assert!(error.contains(&format!("nestrs-cli.{key}")));
        assert!(error.contains("未知 nestrs-cli 配置项"));
    }
}

#[test]
fn initialization_requires_exact_string_enum() {
    for key in ["initialization", "scope-initialization"] {
        for value in ["'Lazy'", "'EAGER'", "'auto'", "''", "true", "0", "[]", "{}"] {
            let error = parse_di(&format!("{key} = {value}")).unwrap_err();
            assert!(error.contains(&format!("nestrs-cli.{key}")));
            assert!(error.contains("\"lazy\" 或 \"eager\""));
        }
    }
}

#[test]
fn activation_limit_requires_positive_integer() {
    for value in ["0", "-1", "1.5", "'32'", "true", "[]", "{}"] {
        let error = parse_di(&format!("max-concurrent-activations = {value}")).unwrap_err();
        assert!(error.contains("nestrs-cli.max-concurrent-activations"));
        assert!(error.contains("大于零"));
    }
}

#[test]
fn non_table_config_reports_the_correct_path() {
    for value in ["'invalid'", "false", "[]", "1"] {
        assert_eq!(
            parse_manifest(&format!("nestrs-cli = {value}")).unwrap_err(),
            "nestrs-cli 必须是 TOML 表"
        );
    }
}

#[test]
fn malformed_toml_is_rejected() {
    let error = parse_manifest("[nestrs-cli\n").unwrap_err();
    assert!(error.contains("TOML 语法错误"));
}

#[test]
fn read_manifest_reports_paths_for_io_and_configuration_errors() {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
    let directory = std::env::temp_dir().join(format!(
        "nestrs-project-config-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("Cargo.toml");
    let missing = read_manifest(&path).unwrap_err();
    assert!(missing.contains(&path.display().to_string()));
    assert!(missing.contains("无法读取项目配置"));

    fs::write(&path, "[nestrs-cli]\ninitialization = 'eager'").unwrap();
    assert!(read_manifest(&path).unwrap().eager);

    fs::write(&path, "[nestrs-cli]\ninitialization = 'invalid'").unwrap();
    let invalid = read_manifest(&path).unwrap_err();
    assert!(invalid.contains(&path.display().to_string()));
    assert!(invalid.contains("initialization"));
    fs::remove_dir_all(directory).unwrap();
}
