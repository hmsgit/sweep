//! End-to-end tests: run the sweep binary over fixture directories.
//!
//! Each directory under tests/fixtures/ holds a config (sweep.toml or
//! pyproject.toml), an input.py and an expected.py. The test copies the
//! fixture to a temp dir, runs `sweep check . --fix`, and asserts the
//! result matches expected.py and that a second --fix run is a no-op
//! (fixes are idempotent).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture_dirs() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .expect("fixtures dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty(), "no fixtures found in {}", root.display());
    dirs
}

fn run_sweep(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sweep"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("failed to run sweep binary")
}

fn setup(fixture: &Path, temp: &Path) {
    for entry in std::fs::read_dir(fixture).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "expected.py" {
            continue;
        }
        std::fs::copy(entry.path(), temp.join(&name)).unwrap();
    }
}

#[test]
fn fixtures_fix_to_expected_and_are_idempotent() {
    for fixture in fixture_dirs() {
        let name = fixture.file_name().unwrap().to_string_lossy().to_string();
        let temp = tempfile::tempdir().unwrap();
        setup(&fixture, temp.path());

        let expected = std::fs::read_to_string(fixture.join("expected.py")).unwrap();

        let output = run_sweep(temp.path(), &["check", ".", "--fix"]);
        assert!(
            output.status.code().is_some_and(|c| c <= 1),
            "[{name}] sweep crashed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let fixed = std::fs::read_to_string(temp.path().join("input.py")).unwrap();
        assert_eq!(
            fixed,
            expected,
            "[{name}] --fix output does not match expected.py\nstdout:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );

        // Idempotency: a second --fix run must change nothing.
        let second = run_sweep(temp.path(), &["check", ".", "--fix"]);
        let refixed = std::fs::read_to_string(temp.path().join("input.py")).unwrap();
        assert_eq!(
            refixed,
            expected,
            "[{name}] second --fix run changed the file again\nstdout:\n{}",
            String::from_utf8_lossy(&second.stdout)
        );
    }
}

#[test]
fn fixed_fixtures_pass_check() {
    // A file already in expected shape must produce no fixable findings.
    for fixture in fixture_dirs() {
        let name = fixture.file_name().unwrap().to_string_lossy().to_string();
        let temp = tempfile::tempdir().unwrap();
        setup(&fixture, temp.path());
        std::fs::copy(fixture.join("expected.py"), temp.path().join("input.py")).unwrap();

        let output = run_sweep(temp.path(), &["check", "."]);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            !stdout.contains("[*]"),
            "[{name}] expected.py still has fixable findings:\n{stdout}"
        );
    }
}

#[test]
fn check_mode_reports_without_touching_files() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hoist");
    let temp = tempfile::tempdir().unwrap();
    setup(&fixture, temp.path());
    let before = std::fs::read_to_string(temp.path().join("input.py")).unwrap();

    let output = run_sweep(temp.path(), &["check", "."]);
    assert_eq!(output.status.code(), Some(1), "diagnostics must exit 1");
    let after = std::fs::read_to_string(temp.path().join("input.py")).unwrap();
    assert_eq!(before, after, "check without --fix must not modify files");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[local-imports]"), "stdout:\n{stdout}");
}

#[test]
fn select_limits_rules() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hoist");
    let temp = tempfile::tempdir().unwrap();
    setup(&fixture, temp.path());

    let output = run_sweep(
        temp.path(),
        &["check", ".", "--select", "string-annotations"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("[local-imports]"),
        "--select must exclude other rules:\n{stdout}"
    );

    let bad = run_sweep(temp.path(), &["check", ".", "--select", "nope"]);
    assert_eq!(bad.status.code(), Some(2), "unknown rule must exit 2");
}

#[test]
fn config_is_resolved_per_file_for_monorepos() {
    // pre-commit runs at the repo root; each app carries its own
    // pyproject.toml. Every file must be judged by its nearest config.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir_all(root.join("apps/alpha")).unwrap();
    std::fs::create_dir_all(root.join("apps/beta")).unwrap();

    std::fs::write(
        root.join("apps/alpha/pyproject.toml"),
        "[tool.sweep.python]\ndocstring-style = \"google\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("apps/beta/pyproject.toml"),
        "[tool.sweep.python]\ndocstring-style = \"rest\"\n",
    )
    .unwrap();

    // Both files carry the same reST docstring: wrong for alpha
    // (google), correct for beta (rest).
    let body = "def f(x):\n    \"\"\"Do.\n\n    :param x: input\n    \"\"\"\n    return x\n";
    std::fs::write(root.join("apps/alpha/m.py"), body).unwrap();
    std::fs::write(root.join("apps/beta/m.py"), body).unwrap();

    let output = run_sweep(root, &["check", "."]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("alpha") && stdout.contains("docstring is reST-style"),
        "alpha must be flagged against its google config:\n{stdout}"
    );
    assert!(
        !stdout.contains("beta"),
        "beta's reST docstring matches its own config:\n{stdout}"
    );

    // Explicit --config overrides discovery for every file.
    let output = run_sweep(
        root,
        &["check", ".", "--config", "apps/alpha/pyproject.toml"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("beta"),
        "--config must apply the google convention to beta too:\n{stdout}"
    );
}

#[test]
fn term_modes_control_escape_sequences() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hoist");
    let temp = tempfile::tempdir().unwrap();
    setup(&fixture, temp.path());

    // Piped output in auto mode carries no escape sequences.
    let auto = run_sweep(temp.path(), &["check", "."]);
    let stdout = String::from_utf8_lossy(&auto.stdout);
    assert!(
        !stdout.contains('\x1b'),
        "auto+pipe must be plain:\n{stdout}"
    );

    // --term hyper forces colors and OSC 8 file hyperlinks.
    let hyper = run_sweep(temp.path(), &["check", ".", "--term", "hyper"]);
    let stdout = String::from_utf8_lossy(&hyper.stdout);
    assert!(
        stdout.contains("\x1b]8;;file://"),
        "hyper must emit OSC 8 links:\n{stdout}"
    );
    assert!(stdout.contains("\x1b[31m"), "hyper must emit colors");

    // --term plain strips everything even when forced elsewhere.
    let plain = run_sweep(temp.path(), &["check", ".", "--term", "plain"]);
    let stdout = String::from_utf8_lossy(&plain.stdout);
    assert!(!stdout.contains('\x1b'), "plain must have no escapes");
}

#[test]
fn concise_format_is_one_line_per_finding() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hoist");
    let temp = tempfile::tempdir().unwrap();
    setup(&fixture, temp.path());

    let output = run_sweep(temp.path(), &["check", ".", "--output-format", "concise"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains(" | "),
        "concise must not print snippet blocks:\n{stdout}"
    );
    // Every finding line still carries location, severity[rule], message.
    let finding_lines: Vec<&str> = stdout
        .lines()
        .filter(|l| l.contains("[local-imports]"))
        .collect();
    assert!(!finding_lines.is_empty());
    assert!(
        finding_lines
            .iter()
            .all(|l| l.contains("input.py:") && l.contains("error[local-imports]")),
        "stdout:\n{stdout}"
    );
}

#[test]
fn line_length_defaults_to_info_only() {
    // No config: limit 79, level info — the long docstring line is
    // reported as info, never fixed, and does not fail the run.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("sweep.toml"), "").unwrap();
    let long_line = "x".repeat(90);
    let source = format!("def f():\n    \"\"\"Summary.\n\n    {long_line}\n    \"\"\"\n");
    std::fs::write(temp.path().join("input.py"), &source).unwrap();

    let output = run_sweep(temp.path(), &["check", ".", "--fix"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("info[docstring-line-length]") && stdout.contains("(79 allowed)"),
        "stdout:\n{stdout}"
    );
    assert!(!stdout.contains("[*]"), "stdout:\n{stdout}");
    assert_eq!(output.status.code(), Some(0), "info must not fail the run");
    let after = std::fs::read_to_string(temp.path().join("input.py")).unwrap();
    assert_eq!(after, source, "info level must not rewrite");
}

#[test]
fn warns_pass_unless_strict() {
    // A warn-level finding is reported and fixable, but only fails the
    // exit code under --strict.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("sweep.toml"),
        "[rules.local-imports]\nlevel = \"warn\"\n",
    )
    .unwrap();
    std::fs::write(
        temp.path().join("input.py"),
        "def f():\n    import os\n    return os.sep\n",
    )
    .unwrap();

    let output = run_sweep(temp.path(), &["check", "."]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("warning[local-imports]"),
        "stdout:\n{stdout}"
    );
    assert_eq!(output.status.code(), Some(0), "warn alone must pass");

    let output = run_sweep(temp.path(), &["check", ".", "--strict"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "--strict must promote warnings to failures"
    );
}

#[test]
fn explicit_file_paths_are_checked() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hoist");
    let temp = tempfile::tempdir().unwrap();
    setup(&fixture, temp.path());

    // pre-commit style: pass the file, not the directory.
    let output = run_sweep(temp.path(), &["check", "input.py"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[local-imports]"), "stdout:\n{stdout}");
}
