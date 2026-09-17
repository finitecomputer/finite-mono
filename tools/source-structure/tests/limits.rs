use source_structure::{Config, StructException, check_repository};
use std::collections::BTreeMap;
use tempfile::TempDir;

fn fixture(source: &str) -> (TempDir, Config) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), source).unwrap();
    let config = Config {
        rust_roots: vec!["src".into()],
        max_rust_lines: 100,
        max_struct_fields: 2,
        guides: BTreeMap::new(),
        struct_exceptions: BTreeMap::new(),
    };
    (dir, config)
}

#[test]
fn syntax_handles_comments_literals_generics_and_external_modules() {
    let (dir, config) = fixture(
        r####"
// mod fake { struct TooBig { a: u8, b: u8, c: u8 } }
const SQL: &str = r###"mod also_fake { } struct Row { a, b, c }"###;
/* outer /* mod fake {} */ comment */
mod external;
pub struct Named<'a, const N: usize> {
    pub callback: fn(&'a str, [u8; N]) -> Result<(), (u8, u8)>,
    pub map: std::collections::BTreeMap<String, (usize, usize)>,
}
pub struct Tuple<'a>(&'a str, Result<(u8, u8), (u8, u8)>);
pub struct Unit;
"####,
    );
    assert!(check_repository(dir.path(), &config).unwrap().is_empty());
}

#[test]
fn rejects_inline_tests_even_when_cfg_disabled() {
    let (dir, config) = fixture("#[cfg(test)] mod tests { #[cfg(any())] mod nested {} }");
    let errors = check_repository(dir.path(), &config).unwrap();
    assert_eq!(errors.len(), 2);
    assert!(errors.iter().any(|e| e.contains("inline module tests")));
    assert!(errors.iter().any(|e| e.contains("inline module nested")));
}

#[test]
fn counts_named_tuple_and_local_structs() {
    let (dir, config) = fixture(
        "struct Named { a: u8, b: u8, c: u8 }\nstruct Tuple(u8,u8,u8);\nfn local() { struct Local(u8,u8,u8); }",
    );
    let errors = check_repository(dir.path(), &config).unwrap();
    assert_eq!(errors.len(), 3);
    assert!(
        errors
            .iter()
            .all(|e| e.contains("has 3 fields, limit is 2"))
    );
}

#[test]
fn counts_cfg_fields_conservatively_and_ignores_enum_variants() {
    let (dir, config) = fixture(
        "struct Row { a: u8, #[cfg(unix)] b: u8, #[cfg(windows)] c: u8 }\nenum Shape { Variant { a: u8, b: u8, c: u8 } }",
    );
    let errors = check_repository(dir.path(), &config).unwrap();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("Row has 3 fields"));
}

#[test]
fn documented_exceptions_reject_growth_shrinkage_and_stale_paths() {
    let (dir, mut config) = fixture("struct Contract { a: u8, b: u8, c: u8 }");
    config.struct_exceptions.insert(
        "src/lib.rs::Contract".into(),
        StructException {
            fields: 3,
            reason: "Flat, deployed wire contract; nesting changes its JSON shape.".into(),
        },
    );
    assert!(check_repository(dir.path(), &config).unwrap().is_empty());
    for source in [
        "struct Contract(u8,u8,u8,u8);",
        "struct Contract(u8,u8);",
        "struct Renamed;",
    ] {
        std::fs::write(dir.path().join("src/lib.rs"), source).unwrap();
        assert!(!check_repository(dir.path(), &config).unwrap().is_empty());
    }
    config
        .struct_exceptions
        .get_mut("src/lib.rs::Contract")
        .unwrap()
        .reason
        .clear();
    assert!(check_repository(dir.path(), &config).is_err());
}

#[test]
fn checks_nested_files_and_physical_lines_including_comments() {
    let (dir, mut config) = fixture("// one\n// two\n");
    config.max_rust_lines = 2;
    assert!(check_repository(dir.path(), &config).unwrap().is_empty());
    std::fs::create_dir(dir.path().join("src/nested")).unwrap();
    std::fs::write(dir.path().join("src/nested/large.rs"), "// one\n\n// three").unwrap();
    let errors = check_repository(dir.path(), &config).unwrap();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("src/nested/large.rs:1: 3 lines exceeds 2"));
}

#[test]
fn rejects_parse_errors_and_include_evasion_but_allows_include_str() {
    let (dir, config) =
        fixture("include!(\"hidden.rs\"); const SQL: &str = include_str!(\"migration.sql\");");
    let errors = check_repository(dir.path(), &config).unwrap();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("include! hides source structure"));
    std::fs::write(dir.path().join("src/lib.rs"), "struct Broken {").unwrap();
    assert!(check_repository(dir.path(), &config).unwrap()[0].contains("cannot parse Rust"));
}

#[test]
fn qualified_include_cannot_hide_source_either() {
    for invocation in ["std::include!", "::core::include!"] {
        let (dir, config) = fixture(&format!("{invocation}(\"hidden.inc\");"));
        let errors = check_repository(dir.path(), &config).unwrap();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("include! hides source structure"));
    }
}

#[test]
fn guide_limits_are_enforced_and_missing_scopes_fail_closed() {
    let (dir, mut config) = fixture("");
    config.guides.insert("AGENTS.md".into(), 1);
    assert!(check_repository(dir.path(), &config).is_err());
    std::fs::write(dir.path().join("AGENTS.md"), "# Guide\nToo much\n").unwrap();
    assert!(check_repository(dir.path(), &config).unwrap()[0].contains("2 lines exceeds 1"));
    config.guides.clear();
    config.rust_roots = vec!["missing".into()];
    assert!(check_repository(dir.path(), &config).is_err());
    config.rust_roots = vec!["../outside".into()];
    assert!(check_repository(dir.path(), &config).is_err());
    config.rust_roots.clear();
    assert!(check_repository(dir.path(), &config).is_err());
}
