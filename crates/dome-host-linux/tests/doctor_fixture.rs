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

#[test]
fn reports_pci_candidates_with_passthrough_readiness() {
    let report = HostReport::probe_root(&fixture_root());

    let summary: Vec<(String, &str, &str)> = report
        .devices
        .candidates
        .iter()
        .map(|candidate| {
            (
                candidate.sysfs_path().to_owned(),
                candidate.class().as_str(),
                candidate.readiness().as_str(),
            )
        })
        .collect();

    assert_eq!(
        summary,
        vec![
            (
                "/sys/bus/pci/devices/0000:01:00.0".to_owned(),
                "gpu",
                "ready-for-explicit-opt-in"
            ),
            (
                "/sys/bus/pci/devices/0000:02:00.0".to_owned(),
                "network-card",
                "unsafe-iommu-group"
            ),
            (
                "/sys/bus/pci/devices/0000:02:00.1".to_owned(),
                "usb-controller",
                "unsafe-iommu-group"
            ),
            (
                "/sys/bus/pci/devices/0000:03:00.0".to_owned(),
                "storage-controller",
                "missing-iommu"
            ),
        ]
    );
}

#[test]
fn vfio_control_node_is_not_a_device() {
    let report = HostReport::probe_root(&fixture_root());

    assert_eq!(report.devices.vfio_devices, vec!["42".to_owned()]);
}

#[test]
fn renders_devices_as_toml() {
    let output = HostReport::probe_root(&fixture_root()).to_toml();

    assert!(output.contains("[devices]"));
    assert!(output.contains("vfio_devices = [\"42\"]"));
    assert!(output.contains("[[devices.candidates]]"));
    assert!(output.contains("class = \"gpu\""));
    assert!(output.contains("readiness = \"unsafe-iommu-group\""));
}
