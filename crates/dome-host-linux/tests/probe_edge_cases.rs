//! Probe edge cases that the main fixture cannot express.
//!
//! Each test builds a throwaway host root in the system temp directory, so the shape of
//! one scenario cannot leak into another.

use std::path::PathBuf;
use std::process;

use dome_host_linux::{HostReport, ProbeRoot};

struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("dome-probe-{name}-{}", process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp root");

        Self { path }
    }

    fn write(&self, relative: &str, contents: &str) {
        let full = self.path.join(relative.trim_start_matches('/'));

        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("create fixture parent");
        }

        std::fs::write(full, contents).expect("write fixture file");
    }

    fn probe(&self) -> HostReport {
        HostReport::probe_root(&ProbeRoot::new(self.path.clone()))
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn empty_root_reports_nothing_and_does_not_panic() {
    let root = TempRoot::new("empty");
    let report = root.probe();

    assert!(!report.virtualization.cpu_virtualization_flag);
    assert!(!report.virtualization.kvm_device_available);
    assert!(!report.virtualization.kvm_device_readable);
    assert!(!report.cgroups.v2_available);
    assert!(!report.network.namespace_visible);
    assert!(report.platform.kernel_version.is_none());
    assert!(report.devices.candidates.is_empty());
    assert!(report.devices.vfio_devices.is_empty());

    // Rendering must also survive an empty report.
    assert!(
        report
            .to_toml()
            .contains("native_engine_device_available = false")
    );
    assert!(report.to_human_text().contains("native engine device: no"));
}

#[test]
fn nested_virtualization_reports_cpu_flag_without_a_device() {
    // The shape of a container or a VM: the host CPU advertises virtualization, but the
    // KVM device is not passed through. Reporting the CPU flag as "ready" would be a lie.
    let root = TempRoot::new("nested");
    root.write(
        "/proc/cpuinfo",
        "processor\t: 0\nflags\t\t: fpu vme sse vmx ssse3\n",
    );
    root.write("/proc/sys/kernel/osrelease", "6.10.0-nested\n");

    let report = root.probe();

    assert!(report.virtualization.cpu_virtualization_flag);
    assert!(!report.virtualization.kvm_device_available);
    assert!(!report.virtualization.kvm_device_readable);
    assert_eq!(
        report.platform.kernel_version.as_deref(),
        Some("6.10.0-nested\n")
    );
}

#[test]
fn amd_virtualization_flag_is_recognised() {
    let root = TempRoot::new("svm");
    root.write("/proc/cpuinfo", "flags\t\t: fpu svm sse\n");

    assert!(root.probe().virtualization.cpu_virtualization_flag);
}

#[test]
fn cgroups_v1_only_host_is_not_reported_as_v2() {
    // Hybrid or legacy hosts have no cgroup.controllers marker at the root.
    let root = TempRoot::new("cgroups-v1");
    root.write("/proc/cpuinfo", "flags\t\t: fpu\n");
    root.write("/sys/fs/cgroup/cpu/tasks", "");
    root.write("/sys/fs/cgroup/memory/tasks", "");

    assert!(!root.probe().cgroups.v2_available);
}

#[test]
fn cgroups_v2_host_is_recognised() {
    let root = TempRoot::new("cgroups-v2");
    root.write("/sys/fs/cgroup/cgroup.controllers", "cpu memory pids\n");

    assert!(root.probe().cgroups.v2_available);
}

#[cfg(unix)]
#[test]
fn device_present_but_not_writable_is_reported_as_such() {
    use std::os::unix::fs::PermissionsExt;

    let root = TempRoot::new("kvm-unreadable");
    root.write("/dev/kvm", "");
    let kvm = root.path.join("dev/kvm");

    // Make it unreadable/unwritable by anyone.
    std::fs::set_permissions(&kvm, std::fs::Permissions::from_mode(0o000)).expect("chmod kvm node");

    // A privileged runner ignores those bits, which would make the assertion below a
    // lie. Detect that and step aside rather than reporting a false pass.
    if std::fs::OpenOptions::new().write(true).open(&kvm).is_ok() {
        eprintln!("skipping: running with privileges that bypass file mode bits");
        return;
    }

    let report = root.probe();

    assert!(
        report.virtualization.kvm_device_available,
        "the path exists, so availability is true"
    );
    assert!(
        !report.virtualization.kvm_device_readable,
        "but the engine could not open it read-write"
    );
}

#[test]
fn device_writable_is_reported_as_readable() {
    let root = TempRoot::new("kvm-readable");
    root.write("/dev/kvm", "");

    let report = root.probe();

    assert!(report.virtualization.kvm_device_available);
    assert!(report.virtualization.kvm_device_readable);
}

#[test]
fn unbound_pci_device_without_iommu_group_is_missing_iommu() {
    let root = TempRoot::new("pci-no-iommu");
    root.write("/sys/bus/pci/devices/0000:04:00.0/class", "0x020000\n");

    let report = root.probe();
    assert_eq!(report.devices.candidates.len(), 1);
    assert_eq!(
        report.devices.candidates[0].readiness().as_str(),
        "missing-iommu"
    );
    assert_eq!(
        report.devices.candidates[0].class().as_str(),
        "network-card"
    );
}

#[test]
fn unknown_pci_class_degrades_instead_of_failing() {
    let root = TempRoot::new("pci-odd-class");
    root.write("/sys/bus/pci/devices/0000:05:00.0/class", "not-a-class\n");

    let report = root.probe();
    assert_eq!(report.devices.candidates.len(), 1);
    assert_eq!(report.devices.candidates[0].class().as_str(), "pci-other");
}
