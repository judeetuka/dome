use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use dome_device::{DeviceCandidate, DeviceClass, PassthroughReadiness};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeRoot {
    root: PathBuf,
}

impl ProbeRoot {
    pub fn host() -> Self {
        Self {
            root: PathBuf::from("/"),
        }
    }

    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path(&self, path: &str) -> PathBuf {
        let trimmed = path.trim_start_matches('/');
        self.root.join(trimmed)
    }

    fn read_to_string(&self, path: &str) -> Option<String> {
        fs::read_to_string(self.path(path)).ok()
    }

    fn exists(&self, path: &str) -> bool {
        self.path(path).exists()
    }

    fn has_directory_entries(&self, path: &str) -> bool {
        fs::read_dir(self.path(path))
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false)
    }

    /// Sorted entry names of a directory, or empty when it cannot be read.
    ///
    /// Sorted so probe output is stable across runs and comparable in tests.
    fn read_dir_names(&self, path: &str) -> Vec<String> {
        let Ok(entries) = fs::read_dir(self.path(path)) else {
            return Vec::new();
        };

        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();

        names.sort();
        names
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostReport {
    pub platform: PlatformReport,
    pub virtualization: VirtualizationReport,
    pub iommu: IommuReport,
    pub vfio: VfioReport,
    pub cgroups: CgroupReport,
    pub security: SecurityReport,
    pub network: NetworkReport,
    pub assembler: AssemblerReport,
    pub devices: DeviceReport,
}

impl HostReport {
    pub fn probe_host() -> Self {
        Self::probe_root(&ProbeRoot::host())
    }

    pub fn probe_root(root: &ProbeRoot) -> Self {
        let cpuinfo = root.read_to_string("/proc/cpuinfo").unwrap_or_default();
        let modules = root.read_to_string("/proc/modules").unwrap_or_default();
        let status = root.read_to_string("/proc/self/status").unwrap_or_default();

        Self {
            platform: PlatformReport {
                kernel_version: root.read_to_string("/proc/sys/kernel/osrelease"),
            },
            virtualization: VirtualizationReport {
                cpu_virtualization_flag: has_virtualization_flag(&cpuinfo),
                kvm_device_available: root.exists("/dev/kvm"),
            },
            iommu: IommuReport {
                groups_detected: root.has_directory_entries("/sys/kernel/iommu_groups"),
            },
            vfio: VfioReport {
                device_available: root.exists("/dev/vfio/vfio"),
                module_loaded: modules.lines().any(|line| line.starts_with("vfio")),
            },
            cgroups: CgroupReport {
                v2_available: root.exists("/sys/fs/cgroup/cgroup.controllers"),
            },
            security: SecurityReport {
                seccomp_visible: status.lines().any(|line| line.starts_with("Seccomp:")),
                landlock_visible: root.exists("/sys/kernel/security/landlock"),
            },
            network: NetworkReport {
                namespace_visible: root.exists("/proc/self/ns/net"),
            },
            assembler: AssemblerReport {
                gnu_as_available: executable_in_path("as"),
                nasm_available: executable_in_path("nasm"),
            },
            devices: probe_devices(root),
        }
    }

    /// Schema version of the `--format toml` report.
    ///
    /// Part of the CLI contract: consumers may add keys within a schema version but
    /// never rename one. See docs/cli-contract.md section 4.
    pub const SCHEMA_VERSION: u32 = 1;

    pub fn to_toml(&self) -> String {
        let mut output = format!(
            concat!(
                "schema_version = {}\n\n",
                "[platform]\n",
                "kernel_version = {}\n\n",
                "[virtualization]\n",
                "cpu_virtualization_flag = {}\n",
                "native_engine_device_available = {}\n\n",
                "[iommu]\n",
                "groups_detected = {}\n\n",
                "[vfio]\n",
                "device_available = {}\n",
                "module_loaded = {}\n\n",
                "[cgroups]\n",
                "v2_available = {}\n\n",
                "[security]\n",
                "seccomp_visible = {}\n",
                "landlock_visible = {}\n\n",
                "[network]\n",
                "namespace_visible = {}\n\n",
                "[assembler]\n",
                "gnu_as_available = {}\n",
                "nasm_available = {}"
            ),
            Self::SCHEMA_VERSION,
            toml_option_string(self.platform.kernel_version.as_deref()),
            self.virtualization.cpu_virtualization_flag,
            self.virtualization.kvm_device_available,
            self.iommu.groups_detected,
            self.vfio.device_available,
            self.vfio.module_loaded,
            self.cgroups.v2_available,
            self.security.seccomp_visible,
            self.security.landlock_visible,
            self.network.namespace_visible,
            self.assembler.gnu_as_available,
            self.assembler.nasm_available,
        );

        output.push_str("\n\n[devices]\n");
        output.push_str(&format!(
            "vfio_device_count = {}\nvfio_devices = [{}]\n",
            self.devices.vfio_devices.len(),
            self.devices
                .vfio_devices
                .iter()
                .map(|name| format!("\"{}\"", escape_toml(name)))
                .collect::<Vec<_>>()
                .join(", ")
        ));

        for candidate in &self.devices.candidates {
            output.push_str("\n[[devices.candidates]]\n");
            output.push_str(&format!(
                "class = \"{}\"\nsysfs_path = \"{}\"\nreadiness = \"{}\"\n",
                candidate.class().as_str(),
                escape_toml(candidate.sysfs_path()),
                candidate.readiness().as_str()
            ));
        }

        output
    }

    pub fn to_human_text(&self) -> String {
        let mut output = format!(
            concat!(
                "Dome host capability report\n",
                "kernel: {}\n",
                "native engine device: {}\n",
                "cpu virtualization flag: {}\n",
                "iommu groups: {}\n",
                "vfio device: {}\n",
                "cgroups v2: {}\n",
                "seccomp status: {}\n",
                "landlock marker: {}\n",
                "network namespace: {}\n",
                "gnu as: {}\n",
                "nasm: {}"
            ),
            self.platform.kernel_version.as_deref().unwrap_or("unknown"),
            yes_no(self.virtualization.kvm_device_available),
            yes_no(self.virtualization.cpu_virtualization_flag),
            yes_no(self.iommu.groups_detected),
            yes_no(self.vfio.device_available),
            yes_no(self.cgroups.v2_available),
            yes_no(self.security.seccomp_visible),
            yes_no(self.security.landlock_visible),
            yes_no(self.network.namespace_visible),
            yes_no(self.assembler.gnu_as_available),
            yes_no(self.assembler.nasm_available),
        );

        output.push_str(&format!(
            "\ndevices: {}\nvfio nodes: {}",
            self.devices.candidates.len(),
            self.devices.vfio_devices.len()
        ));

        output
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformReport {
    pub kernel_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualizationReport {
    pub cpu_virtualization_flag: bool,
    pub kvm_device_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IommuReport {
    pub groups_detected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VfioReport {
    pub device_available: bool,
    pub module_loaded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CgroupReport {
    pub v2_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityReport {
    pub seccomp_visible: bool,
    pub landlock_visible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkReport {
    pub namespace_visible: bool,
}

/// PCI devices visible to the host, with enough context to judge passthrough safety.
///
/// This is read-only reconnaissance: nothing is bound, unbound or reserved. A
/// candidate marked `ReadyForExplicitOptIn` still requires an explicit aperture in the
/// cell blueprint before Dome will touch it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeviceReport {
    pub candidates: Vec<DeviceCandidate>,
    pub vfio_devices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssemblerReport {
    pub gnu_as_available: bool,
    pub nasm_available: bool,
}

const PCI_DEVICES: &str = "/sys/bus/pci/devices";
const IOMMU_GROUPS: &str = "/sys/kernel/iommu_groups";
const VFIO_DIR: &str = "/dev/vfio";

fn probe_devices(root: &ProbeRoot) -> DeviceReport {
    let mut candidates: Vec<DeviceCandidate> = Vec::new();
    let mut grouped: Vec<String> = Vec::new();

    // A device is only isolated if it sits alone in its IOMMU group. Sharing a group
    // with another device means that device can DMA into the same memory, which is why
    // shared groups are reported as unsafe rather than ready.
    for group in root.read_dir_names(IOMMU_GROUPS) {
        let addresses = root.read_dir_names(&format!("{IOMMU_GROUPS}/{group}/devices"));
        let shared_group = addresses.len() > 1;

        for address in addresses {
            grouped.push(address.clone());
            let readiness = if shared_group {
                PassthroughReadiness::UnsafeIommuGroup
            } else {
                PassthroughReadiness::ReadyForExplicitOptIn
            };
            candidates.push(build_candidate(root, &address, readiness));
        }
    }

    for address in root.read_dir_names(PCI_DEVICES) {
        if grouped.contains(&address) {
            continue;
        }
        candidates.push(build_candidate(
            root,
            &address,
            PassthroughReadiness::MissingIommu,
        ));
    }

    candidates.sort_by(|left, right| left.sysfs_path().cmp(right.sysfs_path()));

    // "/dev/vfio/vfio" is the control node, not a device.
    let mut vfio_devices = root.read_dir_names(VFIO_DIR);
    vfio_devices.retain(|name| name != "vfio");

    DeviceReport {
        candidates,
        vfio_devices,
    }
}

fn build_candidate(
    root: &ProbeRoot,
    address: &str,
    readiness: PassthroughReadiness,
) -> DeviceCandidate {
    let class = root
        .read_to_string(&format!("{PCI_DEVICES}/{address}/class"))
        .map(|raw| classify_pci_class(&raw))
        .unwrap_or(DeviceClass::PciOther);

    DeviceCandidate::new(class, format!("{PCI_DEVICES}/{address}"), readiness)
}

/// Map a sysfs `class` value to a device class.
///
/// sysfs reports the 24-bit PCI class as `0xBBSSPP` where BB is the base class, so the
/// base class is the top byte. Anything unparseable degrades to `PciOther` rather than
/// failing the whole probe: one odd device must not blind the report.
fn classify_pci_class(raw: &str) -> DeviceClass {
    let trimmed = raw.trim();
    let digits = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);

    let Ok(value) = u32::from_str_radix(digits, 16) else {
        return DeviceClass::PciOther;
    };

    match (value >> 16) & 0xff {
        0x01 => DeviceClass::StorageController,
        0x02 => DeviceClass::NetworkCard,
        0x03 => DeviceClass::Gpu,
        0x0c => DeviceClass::UsbController,
        _ => DeviceClass::PciOther,
    }
}

fn has_virtualization_flag(cpuinfo: &str) -> bool {
    cpuinfo
        .split_whitespace()
        .any(|token| token == "vmx" || token == "svm")
}

fn executable_in_path(program: &str) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&path).any(|directory| path_has_program(&directory, program))
}

fn path_has_program(directory: &Path, program: &str) -> bool {
    let mut candidate = PathBuf::from(directory);
    candidate.push(OsString::from(program));
    candidate.is_file()
}

fn toml_option_string(value: Option<&str>) -> String {
    match value {
        Some(value) => format!("\"{}\"", escape_toml(value.trim())),
        None => "null".to_owned(),
    }
}

fn escape_toml(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
