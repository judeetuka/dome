use std::path::PathBuf;

use dome_host_linux::{HostReport, ProbeRoot};

#[test]
fn reports_fixture_capabilities() {
    let root = fixture_root();
    let report = HostReport::probe_root(&root);

    assert_eq!(
        report.platform.kernel_version.as_deref(),
        Some("6.10.0-dome-fixture\n")
    );
    assert!(report.virtualization.cpu_virtualization_flag);
    assert!(report.virtualization.kvm_device_available);
    assert!(report.iommu.groups_detected);
    assert!(report.vfio.device_available);
    assert!(report.vfio.module_loaded);
    assert!(report.cgroups.v2_available);
    assert!(report.security.seccomp_visible);
    assert!(report.security.landlock_visible);
    assert!(report.network.namespace_visible);
}

#[test]
fn renders_toml_capabilities() {
    let root = fixture_root();
    let report = HostReport::probe_root(&root);
    let output = report.to_toml();

    assert!(output.contains("schema_version = 1"));
    assert!(output.contains("[platform]"));
    assert!(output.contains("kernel_version = \"6.10.0-dome-fixture\""));
    assert!(output.contains("native_engine_device_available = true"));
}

fn fixture_root() -> ProbeRoot {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    ProbeRoot::new(manifest_dir.join("../../tests/fixtures/linux-host"))
}
