//! Stacks: several chambers that belong together.

use dome_config::ConfigError;
use dome_config::stack::{Stack, WaitCondition};

const TWO: &str = "[stack]\nname = \"app\"\n\n[services.web]\ncell = \"web.cell.toml\"\n\n[services.api]\ncell = \"api.cell.toml\"\ndepends_on = [\"web\"]\n";

#[test]
fn a_stack_names_its_services() {
    let stack = Stack::from_toml(TWO).expect("stack");

    assert_eq!(stack.name, "app");
    assert_eq!(stack.services.len(), 2);
    assert_eq!(stack.hostnames(), vec!["api".to_owned(), "web".to_owned()]);
}

#[test]
fn dependencies_decide_the_start_order() {
    let stack = Stack::from_toml(TWO).expect("stack");

    // api depends on web, so web runs first even though api sorts first.
    assert_eq!(stack.start_order().expect("order"), vec!["web", "api"]);
}

#[test]
fn a_dependency_cycle_is_refused() {
    let input = [
        "[stack]",
        "name = \"app\"",
        "",
        "[services.a]",
        "cell = \"a.cell.toml\"",
        "depends_on = [\"b\"]",
        "",
        "[services.b]",
        "cell = \"b.cell.toml\"",
        "depends_on = [\"a\"]",
    ]
    .join("\n");

    // Without this the failure would show up as chambers waiting on each other forever.
    let error = Stack::from_toml(&input).unwrap_err();
    assert!(error.to_string().contains("cycle"), "{error}");
}

#[test]
fn a_dependency_on_an_unknown_service_is_refused() {
    let input = [
        "[stack]",
        "name = \"app\"",
        "",
        "[services.a]",
        "cell = \"a.cell.toml\"",
        "depends_on = [\"ghost\"]",
    ]
    .join("\n");

    let error = Stack::from_toml(&input).unwrap_err();
    assert!(matches!(error, ConfigError::Unknown { .. }), "{error:?}");
}

#[test]
fn a_service_cannot_depend_on_itself() {
    let input = [
        "[stack]",
        "name = \"app\"",
        "",
        "[services.a]",
        "cell = \"a.cell.toml\"",
        "depends_on = [\"a\"]",
    ]
    .join("\n");

    let error = Stack::from_toml(&input).unwrap_err();
    assert!(error.to_string().contains("itself"), "{error}");
}

#[test]
fn an_empty_stack_is_refused() {
    let error = Stack::from_toml("[stack]\nname = \"app\"\n").unwrap_err();
    assert!(matches!(error, ConfigError::MissingField(_)), "{error:?}");
}

#[test]
fn a_service_without_a_cell_is_refused() {
    let input = [
        "[stack]",
        "name = \"app\"",
        "",
        "[services.a]",
        "cell = \"  \"",
    ]
    .join("\n");

    let error = Stack::from_toml(&input).unwrap_err();
    assert!(error.to_string().contains("no cell"), "{error}");
}

// --- what "depends on" actually waits for ---

#[test]
fn a_bare_dependency_means_started() {
    let stack = Stack::from_toml(TWO).expect("stack");
    let api = stack
        .services
        .iter()
        .find(|s| s.name == "api")
        .expect("api");

    assert_eq!(api.depends_on.len(), 1);
    assert_eq!(api.depends_on[0].condition, WaitCondition::Started);
}

#[test]
fn a_dependency_can_wait_for_health() {
    // Docker's depends_on only waits for the process to start, which is the classic race: the
    // database is running long before it accepts connections.
    let input = [
        "[stack]",
        "name = \"app\"",
        "",
        "[services.db]",
        "cell = \"db.cell.toml\"",
        "",
        "[services.api]",
        "cell = \"api.cell.toml\"",
        "depends_on = [\"db:healthy\"]",
    ]
    .join("\n");

    let stack = Stack::from_toml(&input).expect("stack");
    let api = stack
        .services
        .iter()
        .find(|s| s.name == "api")
        .expect("api");

    assert_eq!(api.depends_on[0].condition, WaitCondition::Healthy);
}

#[test]
fn a_dependency_can_wait_for_a_port() {
    let input = [
        "[stack]",
        "name = \"app\"",
        "",
        "[services.db]",
        "cell = \"db.cell.toml\"",
        "",
        "[services.api]",
        "cell = \"api.cell.toml\"",
        "depends_on = [\"db:port:5432\"]",
    ]
    .join("\n");

    let stack = Stack::from_toml(&input).expect("stack");
    let api = stack
        .services
        .iter()
        .find(|s| s.name == "api")
        .expect("api");

    assert_eq!(api.depends_on[0].condition, WaitCondition::PortReady(5432));
}

#[test]
fn an_unparseable_condition_is_refused() {
    let input = [
        "[stack]",
        "name = \"app\"",
        "",
        "[services.api]",
        "cell = \"api.cell.toml\"",
        "depends_on = [\"api:eventually\"]",
    ]
    .join("\n");

    let error = Stack::from_toml(&input).unwrap_err();
    assert!(error.to_string().contains("not service"), "{error}");
}

#[test]
fn ordering_still_works_with_conditions() {
    let input = [
        "[stack]",
        "name = \"app\"",
        "",
        "[services.web]",
        "cell = \"web.cell.toml\"",
        "",
        "[services.api]",
        "cell = \"api.cell.toml\"",
        "depends_on = [\"web:healthy\"]",
    ]
    .join("\n");

    let stack = Stack::from_toml(&input).expect("stack");
    assert_eq!(stack.start_order().expect("order"), vec!["web", "api"]);
}
