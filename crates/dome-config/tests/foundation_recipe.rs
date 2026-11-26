//! Foundation recipes: sections, pins, linking and cost.

use dome_config::ConfigError;
use dome_config::foundation_recipe::{
    InitChoice, Manager, PackageSpec, Recipe, ServiceManager, UserSpec,
};

#[test]
fn a_recipe_can_name_nothing_at_all() {
    let recipe = Recipe::from_toml(
        "[foundation]\nname = \"app\"\nworkload = \"/usr/bin/app\"\ninit = \"none\"\n",
    )
    .expect("a recipe with no packages is valid");

    assert!(recipe.runtime.packages.is_empty());
    assert!(recipe.static_linking, "static is the default");
    assert_eq!(recipe.plan().kernel_cmdline, "init=/usr/bin/app");
}

#[test]
fn a_bare_recipe_is_a_kernel_and_an_init() {
    let recipe =
        Recipe::from_toml("[foundation]\nname = \"app\"\ninit = \"sentinel\"\n").expect("recipe");
    let plan = recipe.plan();

    assert!(plan.runtime_packages.is_empty());
    assert!(plan.rootfs_is_empty);
    assert!(!plan.image_has_package_manager);
    // Static, so no C library is dragged in.
    assert!(
        !plan
            .runtime_packages
            .iter()
            .any(|p| p == "glibc" || p == "musl")
    );
}

// --- sections ---

#[test]
fn the_sections_answer_different_questions() {
    let input = [
        "[foundation]",
        "name = \"app\"",
        "",
        "[runtime]",
        "packages = [\"fish@3.7.1\", \"neovim\"]",
        "",
        "[build]",
        "packages = [\"rust@1.80.0\", \"gcc\"]",
        "artifacts = [\"/build/target/release/app\"]",
        "",
        "[shell]",
        "default = \"fish\"",
        "packages = [\"fish@3.7.1\"]",
    ]
    .join("\n");

    let recipe = Recipe::from_toml(&input).expect("recipe");

    assert_eq!(recipe.runtime.packages.len(), 2);
    assert_eq!(recipe.build.packages.len(), 2);
    assert_eq!(
        recipe.shell.as_ref().map(|s| s.default.as_str()),
        Some("fish")
    );

    let plan = recipe.plan();
    // Runtime keeps the pin, and the shell does not duplicate it.
    assert!(plan.runtime_packages.contains(&"fish@3.7.1".to_owned()));
    assert_eq!(
        plan.runtime_packages
            .iter()
            .filter(|p| p.starts_with("fish"))
            .count(),
        1
    );
    assert!(plan.build_packages.contains(&"rust@1.80.0".to_owned()));
}

#[test]
fn versions_are_pinned_and_reported() {
    let recipe = Recipe::from_toml(
        "[foundation]\nname = \"app\"\n\n[runtime]\npackages = [\"fish@3.7.1\"]\n",
    )
    .expect("recipe");

    let spec = PackageSpec::parse("fish@3.7.1").expect("spec");
    assert_eq!(spec.name, "fish");
    assert_eq!(spec.version.as_deref(), Some("3.7.1"));
    assert_eq!(spec.label(), "fish@3.7.1");

    let plan = recipe.plan();
    assert!(
        plan.notes.iter().any(|note| note.contains("pinned")),
        "pins must be reported: {:?}",
        plan.notes
    );
}

#[test]
fn an_empty_pin_is_refused() {
    let error =
        Recipe::from_toml("[foundation]\nname = \"app\"\n\n[runtime]\npackages = [\"fish@\"]\n")
            .unwrap_err();

    assert!(error.to_string().contains("empty version"), "{error}");
}

#[test]
fn a_shell_needs_a_name() {
    // The shell is resolved from a profile the builder looks up, so the rule is about the name,
    // not about whether the package list happens to contain it.
    let error = Recipe::from_toml(
        "[foundation]\nname = \"app\"\ninit = \"sentinel\"\n\n[shell]\ndefault = \"  \"\n",
    )
    .unwrap_err();

    assert!(error.to_string().contains("needs a name"), "{error}");
}

#[test]
fn static_linking_is_the_default_and_needs_nothing() {
    let recipe = Recipe::from_toml("[foundation]\nname = \"app\"\n").expect("recipe");
    assert!(recipe.static_linking);
    assert!(
        !recipe.plan().runtime_packages.iter().any(|p| p == "glibc"),
        "a static image must not carry glibc"
    );
}

#[test]
fn disabling_static_linking_adds_the_c_library_automatically() {
    let recipe = Recipe::from_toml("[foundation]\nname = \"app\"\nstatic_linking = false\n")
        .expect("recipe");

    let plan = recipe.plan();

    // Nobody should have to remember this: a dynamic binary cannot run without a loader.
    assert!(
        plan.runtime_packages.iter().any(|p| p == "glibc"),
        "glibc must be added: {:?}",
        plan.runtime_packages
    );
    assert!(
        plan.notes
            .iter()
            .any(|note| note.contains("dynamic linking")),
        "{:?}",
        plan.notes
    );
}

#[test]
fn musl_can_be_the_c_library_instead() {
    let recipe = Recipe::from_toml(
        "[foundation]\nname = \"app\"\nstatic_linking = false\nlibc = \"musl\"\n",
    )
    .expect("recipe");

    let plan = recipe.plan();
    assert!(plan.runtime_packages.iter().any(|p| p == "musl"));
    assert!(!plan.runtime_packages.iter().any(|p| p == "glibc"));
}

// --- stages ---

#[test]
fn build_packages_never_reach_the_release() {
    let input = [
        "[foundation]",
        "name = \"app\"",
        "",
        "[runtime]",
        "packages = [\"fish\"]",
        "",
        "[build]",
        "packages = [\"rust\", \"gcc\", \"git\"]",
    ]
    .join("\n");

    let plan = Recipe::from_toml(&input).expect("recipe").plan();

    for tool in ["rust", "gcc", "git"] {
        assert!(
            !plan.runtime_packages.iter().any(|p| p == tool),
            "{tool} reached the release"
        );
    }
    assert_eq!(plan.packages_dropped_after_build.len(), 3);
}

#[test]
fn a_package_in_both_stages_is_refused() {
    let input = [
        "[foundation]",
        "name = \"app\"",
        "",
        "[runtime]",
        "packages = [\"fish\"]",
        "",
        "[build]",
        "packages = [\"fish\"]",
    ]
    .join("\n");

    let error = Recipe::from_toml(&input).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("both a runtime and a build package"),
        "{error}"
    );
}

#[test]
fn a_duplicate_in_one_section_is_refused() {
    let error = Recipe::from_toml(
        "[foundation]\nname = \"app\"\n\n[runtime]\npackages = [\"fish\", \"fish\"]\n",
    )
    .unwrap_err();

    assert!(error.to_string().contains("listed twice"), "{error}");
}

#[test]
fn a_relative_artifact_path_is_refused() {
    let error =
        Recipe::from_toml("[foundation]\nname = \"app\"\n\n[build]\nartifacts = [\"app\"]\n")
            .unwrap_err();

    assert!(error.to_string().contains("absolute"), "{error}");
}

// --- init and the package manager ---

#[test]
fn the_kernel_is_dome_own_build() {
    let recipe = Recipe::from_toml("[foundation]\nname = \"app\"\n").expect("recipe");
    assert_eq!(recipe.kernel, "dome-minimal");
}

#[test]
fn every_init_choice_yields_a_command_line() {
    for (name, expected) in [
        ("sentinel", "init=/sbin/sentinel"),
        ("busybox", "init=/sbin/busybox"),
        ("systemd", "init=/sbin/systemd"),
        ("openrc", "init=/sbin/openrc"),
        ("runit", "init=/sbin/runit"),
        ("s6", "init=/sbin/s6"),
        ("dinit", "init=/sbin/dinit"),
    ] {
        let input = format!("[foundation]\nname = \"app\"\ninit = \"{name}\"\n");
        let recipe = Recipe::from_toml(&input).expect(name);
        assert_eq!(recipe.plan().kernel_cmdline, expected);
    }
}

#[test]
fn systemd_adds_its_dependencies_and_its_measured_cost() {
    let recipe =
        Recipe::from_toml("[foundation]\nname = \"app\"\ninit = \"systemd\"\n").expect("recipe");
    let plan = recipe.plan();

    assert!(plan.runtime_packages.iter().any(|p| p == "systemd-libs"));
    assert!(plan.runtime_packages.iter().any(|p| p == "dbus"));
    assert!(plan.notes.iter().any(|note| note.contains("MiB")));
}

#[test]
fn an_unknown_init_is_refused() {
    let error = Recipe::from_toml("[foundation]\nname = \"app\"\ninit = \"magic\"\n").unwrap_err();
    assert!(matches!(error, ConfigError::Unknown { .. }), "{error:?}");
}

#[test]
fn init_none_needs_a_workload() {
    let error = Recipe::from_toml("[foundation]\nname = \"app\"\ninit = \"none\"\n").unwrap_err();
    assert!(matches!(error, ConfigError::MissingField(_)), "{error:?}");
}

#[test]
fn no_package_manager_by_default() {
    let plan = Recipe::from_toml("[foundation]\nname = \"app\"\n")
        .expect("recipe")
        .plan();

    assert!(!plan.image_has_package_manager);
    assert!(
        plan.notes
            .iter()
            .any(|note| note.contains("no package manager"))
    );
}

#[test]
fn a_package_manager_can_be_asked_for() {
    let plan = Recipe::from_toml("[foundation]\nname = \"app\"\npackage_manager = \"apk\"\n")
        .expect("recipe")
        .plan();

    assert!(plan.image_has_package_manager);
    assert!(plan.runtime_packages.iter().any(|p| p == "apk-tools"));
}

#[test]
fn service_managers_are_recognised_by_name() {
    assert_eq!(ServiceManager::parse("runit"), Some(ServiceManager::Runit));
    assert_eq!(ServiceManager::parse("nosuch"), None);
    assert!(matches!(InitChoice::parse("none"), Ok(InitChoice::None)));
}

// --- who the chamber runs as, and where the project lives ---

#[test]
fn root_is_the_default_user() {
    let recipe = Recipe::from_toml("[foundation]\nname = \"app\"\n").expect("recipe");
    assert_eq!(recipe.user, UserSpec::Root);
    assert!(recipe.workspace.is_none());
}

#[test]
fn the_host_user_can_be_copied() {
    let recipe = Recipe::from_toml("[foundation]\nname = \"app\"\n\n[user]\nmode = \"host\"\n")
        .expect("recipe");

    assert_eq!(recipe.user, UserSpec::Host);
    // This is what makes Windows and macOS behave like Linux: the engine maps the host identity
    // to a numeric uid inside, so bind-mounted files keep their ownership.
    assert!(
        recipe
            .plan()
            .notes
            .iter()
            .any(|note| note.contains("host user"))
    );
}

#[test]
fn a_custom_user_carries_its_uid() {
    let recipe = Recipe::from_toml(
        "[foundation]\nname = \"app\"\n\n[user]\nmode = \"custom\"\nname = \"dev\"\nuid = 1234\n",
    )
    .expect("recipe");

    match &recipe.user {
        UserSpec::Custom { name, uid, .. } => {
            assert_eq!(name, "dev");
            assert_eq!(*uid, 1234);
        }
        other => panic!("expected a custom user, got {other:?}"),
    }
}

#[test]
fn a_relative_home_is_refused() {
    let error = Recipe::from_toml(
        "[foundation]\nname = \"app\"\n\n[user]\nmode = \"custom\"\nhome = \"home/dev\"\n",
    )
    .unwrap_err();

    assert!(error.to_string().contains("absolute"), "{error}");
}

#[test]
fn an_unknown_user_mode_is_refused() {
    let error = Recipe::from_toml("[foundation]\nname = \"app\"\n\n[user]\nmode = \"wizard\"\n")
        .unwrap_err();
    assert!(matches!(error, ConfigError::Unknown { .. }), "{error:?}");
}

#[test]
fn a_workspace_names_the_project_directory() {
    let recipe = Recipe::from_toml(
        "[foundation]\nname = \"app\"\n\n[workspace]\npath = \"/home/dev/project\"\nhost = \"/srv/project\"\n",
    )
    .expect("recipe");

    let workspace = recipe.workspace.clone().expect("workspace");
    assert_eq!(workspace.path, "/home/dev/project");
    assert_eq!(workspace.host.as_deref(), Some("/srv/project"));
    assert!(
        recipe
            .plan()
            .notes
            .iter()
            .any(|note| note.contains("mapped from"))
    );
}

#[test]
fn a_relative_workspace_path_is_refused() {
    let error =
        Recipe::from_toml("[foundation]\nname = \"app\"\n\n[workspace]\npath = \"project\"\n")
            .unwrap_err();

    assert!(error.to_string().contains("absolute"), "{error}");
}

#[test]
fn a_package_can_name_its_own_manager() {
    let spec = PackageSpec::parse("pacman:fish@3.7.1").expect("spec");
    assert_eq!(spec.label(), "pacman:fish@3.7.1");
    assert_eq!(spec.version.as_deref(), Some("3.7.1"));

    let aur = PackageSpec::parse("yay:visual-studio-code-bin").expect("spec");
    assert_eq!(aur.label(), "yay:visual-studio-code-bin");

    // Without a prefix the builder decides.
    assert_eq!(PackageSpec::parse("fish").expect("spec").label(), "fish");
}

#[test]
fn a_prefix_without_a_package_is_refused() {
    let error = PackageSpec::parse("pacman:").unwrap_err();
    assert!(error.to_string().contains("no package"), "{error}");
}

#[test]
fn mixed_managers_survive_into_the_plan() {
    let recipe = Recipe::from_toml(
        "[foundation]\nname = \"app\"\n\n[runtime]\npackages = [\"pacman:fish\", \"yay:neovim-git@0.10\"]\n",
    )
    .expect("recipe");

    let plan = recipe.plan();
    assert!(plan.runtime_packages.contains(&"pacman:fish".to_owned()));
    assert!(
        plan.runtime_packages
            .contains(&"yay:neovim-git@0.10".to_owned())
    );
}

#[test]
fn mise_is_a_package_source() {
    // No distribution involved: a mise toolchain is a directory, which is what lets it be copied
    // into an image without a base system underneath it.
    let spec = PackageSpec::parse("mise:node@20.11.0").expect("spec");
    assert_eq!(spec.label(), "mise:node@20.11.0");
    assert_eq!(spec.name, "node");
    assert_eq!(spec.version.as_deref(), Some("20.11.0"));

    let recipe = Recipe::from_toml(
        "[foundation]\nname = \"app\"\n\n[runtime]\npackages = [\"mise:python@3.12\"]\n",
    )
    .expect("recipe");

    assert!(
        recipe
            .plan()
            .runtime_packages
            .contains(&"mise:python@3.12".to_owned())
    );
}

// --- mise is implied, backends named the way mise names them, options pass through ---

#[test]
fn mise_packages_do_not_need_to_be_declared() {
    // A user writes what they would write in a mise.toml; the image works out that it needs mise.
    let recipe = Recipe::from_toml(
        "[foundation]\nname = \"app\"\ninit = \"sentinel\"\n\n[runtime]\npackages = [\"mise:node@20\"]\n",
    )
    .expect("recipe");

    assert_eq!(recipe.managers(), vec![Manager::Mise]);
    assert!(
        recipe
            .plan()
            .notes
            .iter()
            .any(|note| note.contains("installs its own packages with mise"))
    );
}

#[test]
fn a_backend_prefix_is_still_a_mise_package() {
    let spec = PackageSpec::parse("cargo:ast-grep").expect("spec");
    assert_eq!(spec.label(), "cargo:ast-grep");
    assert_eq!(spec.manager(), Manager::Mise);

    let npm = PackageSpec::parse("npm:@openai/codex@1.2.3").expect("spec");
    assert_eq!(npm.name, "@openai/codex");
    assert_eq!(npm.version.as_deref(), Some("1.2.3"));
    assert_eq!(npm.manager(), Manager::Mise);
}

#[test]
fn several_managers_can_be_needed_at_once() {
    let recipe = Recipe::from_toml(
        "[foundation]\nname = \"app\"\ninit = \"sentinel\"\n\n[runtime]\npackages = [\"node\", \"pacman:docker\"]\n",
    )
    .expect("recipe");

    // Sorted and deduped: the image installs with both, and nothing asks the user to say so.
    assert_eq!(recipe.managers(), vec![Manager::Mise, Manager::Pacman]);
}

#[test]
fn tools_are_a_table_with_versions_and_options() {
    // Shapes taken from a real mise.toml: a plain version, a table of options, and a backend key.
    let input = [
        "[foundation]",
        "name = \"app\"",
        "init = \"sentinel\"",
        "",
        "[runtime.tools]",
        "node = \"lts\"",
        "rust = { version = \"stable\", components = [\"clippy\", \"rustfmt\"] }",
        "\"npm:@openai/codex\" = \"latest\"",
        "\"npm:@opencode/cli\" = { version = \"latest\", npm_args = \"--ignore-scripts=false\" }",
        "\"github:koalaman/shellcheck\" = \"latest\"",
    ]
    .join("\n");

    let recipe = Recipe::from_toml(&input).expect("recipe");
    let tools = &recipe.runtime.packages;

    assert_eq!(tools.len(), 5);

    let rust = tools.iter().find(|t| t.name == "rust").expect("rust");
    assert_eq!(rust.version.as_deref(), Some("stable"));
    assert_eq!(
        rust.options
            .get("components")
            .and_then(|value| value.as_array())
            .map(|values| values.len()),
        Some(2)
    );

    let codex = tools
        .iter()
        .find(|t| t.name == "@opencode/cli")
        .expect("codex");
    assert_eq!(
        codex
            .options
            .get("npm_args")
            .and_then(|value| value.as_str()),
        Some("--ignore-scripts=false"),
        "tool options must survive verbatim"
    );

    // Every one of them is installed by mise, including the ones naming a backend.
    assert_eq!(recipe.managers(), vec![Manager::Mise]);
}

#[test]
fn build_tools_are_parsed_the_same_way() {
    let input = [
        "[foundation]",
        "name = \"app\"",
        "init = \"sentinel\"",
        "",
        "[build.tools]",
        "\"cargo:just\" = \"latest\"",
        "go = \"latest\"",
    ]
    .join("\n");

    let recipe = Recipe::from_toml(&input).expect("recipe");
    assert_eq!(recipe.build.packages.len(), 2);
    assert_eq!(recipe.managers(), vec![Manager::Mise]);
}

#[test]
fn a_backend_mise_does_not_know_yet_is_still_accepted() {
    // mise owns the vocabulary of backends and keeps adding to it. A whitelist here would turn
    // every new backend into a package name, so any word-like prefix is passed through.
    let futuristic = PackageSpec::parse("ubi:BurntSushi/ripgrep").expect("spec");
    assert_eq!(futuristic.label(), "ubi:BurntSushi/ripgrep");
    assert_eq!(futuristic.name, "BurntSushi/ripgrep");
    assert_eq!(futuristic.manager(), Manager::Mise);

    // And a non-word prefix is not a backend, so the colon stays part of the name.
    let plain = PackageSpec::parse("weird name:thing").expect("spec");
    assert_eq!(plain.name, "weird name:thing");
}
