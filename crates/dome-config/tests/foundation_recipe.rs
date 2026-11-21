//! Foundation recipes: what an image contains, and what that costs.

use dome_config::ConfigError;
use dome_config::foundation_recipe::{InitChoice, Recipe, ServiceManager};

#[test]
fn a_recipe_can_name_nothing_at_all() {
    let recipe = Recipe::from_toml(
        "[foundation]\nname = \"app\"\nworkload = \"/usr/bin/app\"\ninit = \"none\"\n",
    )
    .expect("a recipe with no packages is valid");

    assert!(recipe.packages.is_empty());
    assert!(
        recipe.static_linking,
        "static is the default, because it removes glibc"
    );

    let plan = recipe.plan();
    assert!(plan.rootfs_is_empty);
    assert_eq!(plan.kernel_cmdline, "init=/usr/bin/app");
}

#[test]
fn packages_are_exactly_what_was_asked_for() {
    let recipe =
        Recipe::from_toml("[foundation]\nname = \"app\"\npackages = [\"fish\", \"neovim\"]\n")
            .expect("recipe");

    let plan = recipe.plan();
    assert_eq!(plan.packages, vec!["fish".to_owned(), "neovim".to_owned()]);
    assert!(!plan.rootfs_is_empty);
}

#[test]
fn a_static_recipe_is_told_static_linking_removed_glibc() {
    let recipe = Recipe::from_toml(
        "[foundation]\nname = \"app\"\npackages = [\"fish\"]\nworkload = \"/app\"\ninit = \"none\"\n",
    )
    .expect("recipe");

    // Static is the default, so nothing should mention glibc.
    assert!(recipe.static_linking);
    assert!(
        !recipe
            .plan()
            .notes
            .iter()
            .any(|note| note.contains("glibc")),
        "static recipes should not be warned about glibc"
    );
}

#[test]
fn dynamic_linking_is_reported_as_what_it_costs() {
    let recipe = Recipe::from_toml(
        "[foundation]\nname = \"app\"\npackages = [\"bash\"]\nstatic_linking = false\n",
    )
    .expect("recipe");

    let plan = recipe.plan();
    assert!(
        plan.notes.iter().any(|note| note.contains("glibc")),
        "dynamic linking must be reported: {:?}",
        plan.notes
    );
}

#[test]
fn choosing_systemd_adds_its_dependencies_and_says_what_it_weighs() {
    let recipe = Recipe::from_toml(
        "[foundation]\nname = \"app\"\ninit = \"systemd\"\npackages = [\"fish\"]\n",
    )
    .expect("recipe");

    let plan = recipe.plan();

    // systemd needs its libraries and dbus; the closure is the truth about the image.
    assert!(plan.packages.contains(&"systemd-libs".to_owned()));
    assert!(plan.packages.contains(&"dbus".to_owned()));
    assert!(
        plan.notes.iter().any(|note| note.contains("MiB")),
        "the cost must be measured, not implied: {:?}",
        plan.notes
    );
    assert_eq!(plan.kernel_cmdline, "init=/sbin/systemd");
}

#[test]
fn a_lightweight_service_manager_is_a_first_class_choice() {
    for name in ["openrc", "runit", "s6", "dinit"] {
        let input = format!("[foundation]\nname = \"app\"\ninit = \"{name}\"\n");
        let recipe = Recipe::from_toml(&input).expect(name);

        assert_eq!(
            recipe.init,
            InitChoice::ServiceManager(ServiceManager::parse(name).expect("manager"))
        );
        assert_eq!(recipe.plan().kernel_cmdline, format!("init=/sbin/{name}"));
    }
}

#[test]
fn an_unknown_init_is_refused() {
    let error = Recipe::from_toml("[foundation]\nname = \"app\"\ninit = \"magic\"\n").unwrap_err();
    assert!(matches!(error, ConfigError::Unknown { .. }), "{error:?}");
}

#[test]
fn init_none_needs_something_to_run() {
    // With init = none the kernel execs the workload directly, so there must be one.
    let error = Recipe::from_toml("[foundation]\nname = \"app\"\ninit = \"none\"\n").unwrap_err();
    assert!(matches!(error, ConfigError::MissingField(_)), "{error:?}");
}

#[test]
fn a_package_listed_twice_is_a_mistake() {
    let error =
        Recipe::from_toml("[foundation]\nname = \"app\"\npackages = [\"fish\", \"fish\"]\n")
            .unwrap_err();

    assert!(error.to_string().contains("listed twice"), "{error}");
}

#[test]
fn an_unknown_architecture_is_refused() {
    let error = Recipe::from_toml("[foundation]\nname = \"app\"\narch = \"riscv\"\n").unwrap_err();
    assert!(matches!(error, ConfigError::Unknown { .. }), "{error:?}");
}

#[test]
fn an_explicit_init_path_is_allowed() {
    let recipe =
        Recipe::from_toml("[foundation]\nname = \"app\"\ninit = \"path:/usr/local/sbin/myinit\"\n")
            .expect("recipe");

    assert_eq!(recipe.plan().kernel_cmdline, "init=/usr/local/sbin/myinit");
}

#[test]
fn a_relative_init_path_is_refused() {
    let error =
        Recipe::from_toml("[foundation]\nname = \"app\"\ninit = \"path:myinit\"\n").unwrap_err();
    assert!(
        matches!(error, ConfigError::InvalidValue { .. }),
        "{error:?}"
    );
}

// --- the build stage never reaches the release ---

const TWO_STAGE: &str = "[foundation]\nname = \"app\"\ninit = \"sentinel\"\npackages = [\"fish\"]\n\n[build]\npackages = [\"rust\", \"gcc\", \"git\"]\nartifacts = [\"/build/target/release/app\"]\n";

#[test]
fn build_packages_are_not_part_of_the_release() {
    let recipe = Recipe::from_toml(TWO_STAGE).expect("recipe");
    let plan = recipe.plan();

    assert_eq!(plan.build_packages, vec!["rust", "gcc", "git"]);
    for tool in ["rust", "gcc", "git"] {
        assert!(
            !plan.runtime_packages.iter().any(|package| package == tool),
            "{tool} must not reach the release: {:?}",
            plan.runtime_packages
        );
    }

    assert_eq!(plan.runtime_packages, vec!["fish".to_owned()]);
}

#[test]
fn artifacts_are_copied_into_the_release() {
    let recipe = Recipe::from_toml(TWO_STAGE).expect("recipe");
    let plan = recipe.plan();

    assert_eq!(plan.artifacts, vec!["/build/target/release/app".to_owned()]);
    assert!(
        plan.notes.iter().any(|note| note.contains("artifact")),
        "the copy must be reported: {:?}",
        plan.notes
    );
}

#[test]
fn a_package_in_both_stages_is_refused() {
    let error = Recipe::from_toml(
        "[foundation]\nname = \"app\"\npackages = [\"fish\"]\n\n[build]\npackages = [\"fish\"]\n",
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("both a runtime and a build package"),
        "{error}"
    );
}

#[test]
fn a_relative_artifact_path_is_refused() {
    let error =
        Recipe::from_toml("[foundation]\nname = \"app\"\n\n[build]\nartifacts = [\"app\"]\n")
            .unwrap_err();

    assert!(error.to_string().contains("absolute"), "{error}");
}

// --- the kernel and the package manager ---

#[test]
fn the_kernel_is_dome_own_build_by_default() {
    let recipe = Recipe::from_toml("[foundation]\nname = \"app\"\n").expect("recipe");

    assert_eq!(recipe.kernel, "dome-minimal");
    assert!(
        recipe
            .plan()
            .notes
            .iter()
            .any(|note| note.contains("not a distribution package")),
        "no distribution kernel must be claimed"
    );
}

#[test]
fn no_package_manager_is_the_default() {
    let recipe = Recipe::from_toml("[foundation]\nname = \"app\"\n").expect("recipe");
    let plan = recipe.plan();

    assert!(
        !plan.image_has_package_manager,
        "nothing to install anything with"
    );
    assert!(
        plan.notes
            .iter()
            .any(|note| note.contains("no package manager")),
        "{:?}",
        plan.notes
    );
}

#[test]
fn a_package_manager_can_be_asked_for() {
    for (name, package) in [("apk", "apk-tools"), ("pacman", "pacman")] {
        let input = format!("[foundation]\nname = \"app\"\npackage_manager = \"{name}\"\n");
        let recipe = Recipe::from_toml(&input).expect(name);
        let plan = recipe.plan();

        assert!(plan.image_has_package_manager);
        assert!(
            plan.runtime_packages.contains(&package.to_owned()),
            "{plan:?}"
        );
    }
}

#[test]
fn an_unknown_package_manager_is_refused() {
    let error =
        Recipe::from_toml("[foundation]\nname = \"app\"\npackage_manager = \"dnf\"\n").unwrap_err();
    assert!(matches!(error, ConfigError::Unknown { .. }), "{error:?}");
}

#[test]
fn a_truly_empty_recipe_is_a_kernel_and_an_init() {
    let recipe = Recipe::from_toml(
        "[foundation]\nname = \"app\"\ninit = \"sentinel\"\nstatic_linking = true\n",
    )
    .expect("recipe");
    let plan = recipe.plan();

    assert!(plan.runtime_packages.is_empty());
    assert!(plan.build_packages.is_empty());
    assert!(plan.artifacts.is_empty());
    assert!(plan.rootfs_is_empty, "nothing but the init");
    assert!(!plan.image_has_package_manager);
}
