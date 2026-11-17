//! The suite's agent-native mould in Local Slides: a `--script` array, `commands --search`, and
//! `mcp --connect` taking a bare loopback port while refusing any other host.

use std::path::PathBuf;
use std::process::{Command, Output};

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_localslides-cli")).args(args).output().unwrap()
}

fn stdout(o: &Output) -> String {
    assert!(o.status.success(), "exit {:?}\nstderr: {}", o.status, String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    assert!(!o.status.success(), "exit {:?}\nstdout: {}", o.status, String::from_utf8_lossy(&o.stdout));
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("localslides-cli-mould-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn a_script_may_be_a_json_array() {
    let d = dir("array");
    let script = d.join("steps.json");
    std::fs::write(&script, r#"["slide.next", {"command": "slide.next", "params": {}}]"#).unwrap();
    let save = d.join("out.deckcraft");
    let o = cli(&["run", "--sample", "--script", script.to_str().unwrap(), "--save", save.to_str().unwrap()]);
    stdout(&o);
    assert!(save.exists(), "the run saved the presentation");
}

#[test]
fn a_bad_array_step_names_its_index_and_command() {
    let d = dir("bad");
    let script = d.join("steps.json");
    std::fs::write(&script, r#"["slide.next", {"command": "nope.nothing"}]"#).unwrap();
    let e = stderr(&cli(&["run", "--sample", "--script", script.to_str().unwrap()]));
    assert!(e.contains("[1]") && e.contains("nope.nothing"), "{e}");
}

#[test]
fn commands_search_filters_the_catalogue() {
    let all = stdout(&cli(&["commands"]));
    let found = stdout(&cli(&["commands", "--search", "format"]));
    let (all, found) = (all.lines().count(), found.lines().count());
    assert!(found > 0 && found < all, "{found} of {all}");
    for line in stdout(&cli(&["commands", "--search", "format"])).lines() {
        assert!(line.to_lowercase().contains("format"), "{line}");
    }
}

#[test]
fn connect_takes_a_bare_port_and_refuses_a_public_host() {
    let e = stderr(&cli(&["mcp", "--connect", "1"]));
    assert!(e.contains("127.0.0.1:1"), "{e}");
    let e = stderr(&cli(&["mcp", "--connect", "192.168.1.5:7990"]));
    assert!(e.contains("loopback only"), "{e}");
}
