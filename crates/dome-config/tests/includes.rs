//! One config file that pulls in others.

use std::path::{Path, PathBuf};

use dome_config::ConfigError;
use dome_config::foundation_recipe::Recipe;
use dome_config::include::load_merged;

fn scratch(name: &str) -> PathBuf {
    let unique = format!("dome-include-{name}-{}", std::process::id());
    let dir = std::env::temp_dir().join(unique);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write");
    path
}

#[test]
fn an_included_file_supplies_what_the_project_leaves_out() {
    let dir = scratch("merge");
    write(
        &dir,
        "base.toml",
        "[foundation]\nname = \"from-base\"\ninit = \"sentinel\"\n",
    );
    let project = write(
        &dir,
        "dome.toml",
        "include = [\"base.toml\"]\n\n[foundation]\narch = \"x86_64\"\n",
    );

    let merged = load_merged(&project).expect("merged");
    let recipe = Recipe::from_toml(&merged).expect("recipe");

    assert_eq!(recipe.name, "from-base");
    assert_eq!(recipe.arch, "x86_64");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_including_file_wins_for_a_value_it_sets() {
    let dir = scratch("override");
    write(
        &dir,
        "base.toml",
        "[foundation]\nname = \"base\"\ninit = \"sentinel\"\n",
    );
    let project = write(
        &dir,
        "dome.toml",
        "include = [\"base.toml\"]\n\n[foundation]\nname = \"mine\"\n",
    );

    let recipe = Recipe::from_toml(&load_merged(&project).expect("merged")).expect("recipe");
    assert_eq!(recipe.name, "mine", "the project overrides the base");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lists_are_concatenated_without_duplicates() {
    let dir = scratch("arrays");
    write(
        &dir,
        "base.toml",
        "[foundation]\nname = \"app\"\n\n[runtime]\npackages = [\"fish\", \"neovim\"]\n",
    );
    let project = write(
        &dir,
        "dome.toml",
        "include = [\"base.toml\"]\n\n[runtime]\npackages = [\"neovim\", \"bash@5.2\"]\n",
    );

    let recipe = Recipe::from_toml(&load_merged(&project).expect("merged")).expect("recipe");
    let labels = recipe.runtime.labels();

    assert_eq!(labels, vec!["fish", "neovim", "bash@5.2"]);
    assert_eq!(
        labels.iter().filter(|l| *l == "neovim").count(),
        1,
        "no duplicate"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn includes_nest() {
    let dir = scratch("nested");
    write(
        &dir,
        "kernel.toml",
        "[foundation]\nname = \"app\"\ninit = \"sentinel\"\n",
    );
    write(
        &dir,
        "base.toml",
        "include = [\"kernel.toml\"]\n\n[runtime]\npackages = [\"fish\"]\n",
    );
    let project = write(
        &dir,
        "dome.toml",
        "include = [\"base.toml\"]\n\n[shell]\ndefault = \"fish\"\npackages = [\"fish\"]\n",
    );

    let recipe = Recipe::from_toml(&load_merged(&project).expect("merged")).expect("recipe");
    assert_eq!(recipe.name, "app");
    assert!(recipe.shell.is_some());
    assert_eq!(recipe.runtime.labels(), vec!["fish".to_owned()]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_cycle_is_refused() {
    let dir = scratch("cycle");
    write(&dir, "a.toml", "include = [\"b.toml\"]\n");
    let a = write(&dir, "b.toml", "include = [\"a.toml\"]\n");

    // Two files including each other has no meaning; refusing beats hanging.
    let error = load_merged(&a).unwrap_err();
    assert!(
        matches!(error, ConfigError::IncludeCycle { .. }),
        "{error:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_include_says_which_file() {
    let dir = scratch("missing");
    let project = write(&dir, "dome.toml", "include = [\"nope.toml\"]\n");

    let error = load_merged(&project).unwrap_err();
    assert!(matches!(error, ConfigError::CannotRead { .. }), "{error:?}");
    assert!(error.to_string().contains("nope.toml"), "{error}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_include_key_does_not_leak_into_the_result() {
    let dir = scratch("leak");
    write(
        &dir,
        "base.toml",
        "[foundation]\nname = \"app\"\ninit = \"sentinel\"\n",
    );
    let project = write(&dir, "dome.toml", "include = [\"base.toml\"]\n");

    let merged = load_merged(&project).expect("merged");
    assert!(
        !merged.contains("include"),
        "the key must not survive: {merged}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
