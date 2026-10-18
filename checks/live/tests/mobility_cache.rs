use std::{fs, path::PathBuf, process::Command};
fn run(kind: &str, input: &std::path::Path, output: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_poznan-live-probe"))
        .args([kind, input.to_str().unwrap(), output.to_str().unwrap()])
        .output()
        .unwrap()
}
#[test]
fn valid_collections_replace_atomically_and_invalid_refresh_preserves_previous_file() {
    let dir = std::env::temp_dir().join(format!("poznan-mobility-cache-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let output = dir.join("cache.ndjson");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/stops.json");
    let result = run("parse-stops", &fixture, &output);
    assert!(result.status.success(), "{:?}", result);
    let first = fs::read(&output).unwrap();
    let text = std::str::from_utf8(&first).unwrap();
    assert!(text.contains("Próba 🚋"));
    assert!(text.contains("\"items\":2"));
    let bad = dir.join("bad.json");
    fs::write(&bad, b"{\"type\":\"FeatureCollection\",\"features\":[").unwrap();
    assert!(!run("parse-stops", &bad, &output).status.success());
    assert_eq!(fs::read(&output).unwrap(), first);
    let bad_crs = fs::read_to_string(fixture).unwrap().replace("4326", "2177");
    fs::write(&bad, bad_crs).unwrap();
    assert!(!run("parse-stops", &bad, &output).status.success());
    assert_eq!(fs::read(&output).unwrap(), first);
    assert!(
        fs::read_dir(&dir).unwrap().all(|p| !p
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("partial"))
    );
    let meters = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/meters.json");
    assert!(run("parse-meters", &meters, &output).status.success());
    let text = fs::read_to_string(&output).unwrap();
    assert!(text.contains("\"peka\":null"));
    assert!(text.contains("\"coins\":true,\"card\":false"));
    fs::remove_dir_all(dir).unwrap();
}
