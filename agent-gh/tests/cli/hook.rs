use crate::support::Sandbox;
use crate::support::run;
use crate::support::text;
use std::path::Path;

#[test]
fn hook_fixtures_produce_their_named_decisions() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hooks");
    let sandbox = Sandbox::new();
    let mut checked = 0;
    for entry in fs_err::read_dir(&dir).expect("fixture directory is readable") {
        let path = entry.expect("fixture entry is readable").path();
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("fixture name is UTF-8")
            .to_owned();
        let blocks = if name.ends_with("-block") {
            true
        } else if name.ends_with("-allow") {
            false
        } else {
            panic!("{name} must end in -block or -allow");
        };
        let payload = fs_err::read_to_string(&path).expect("fixture is readable");

        let output = run(&mut sandbox.command(&["self", "hook-check"]), &payload);

        let stderr = text(&output.stderr);
        assert!(output.stdout.is_empty(), "{name}");
        if blocks {
            assert_eq!(output.status.code(), Some(2), "{name}: {stderr}");
            assert!(stderr.starts_with("agent-gh: "), "{name}: {stderr}");
        } else {
            assert_eq!(output.status.code(), Some(0), "{name}: {stderr}");
            assert!(stderr.is_empty(), "{name}: {stderr}");
        }
        checked += 1;
    }
    assert!(checked > 0, "no fixtures in {}", dir.display());
}
