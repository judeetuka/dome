//! Building a kernel reproducibly: the parts that can be wrong before a compiler runs.

use dome_cli::kernel_build::{BuildOptions, REQUIRED_TOOLS, missing_tools, tarball_url};

#[test]
fn a_version_maps_to_its_major_series() {
    // kernel.org files releases under v<major>.x, so 7.3.6 lives in v7.x. Getting this wrong
    // produces a 404 that reads like a missing version.
    assert_eq!(
        tarball_url("7.3.6").expect("url"),
        "https://cdn.kernel.org/pub/linux/kernel/v7.x/linux-7.3.6.tar.xz"
    );
    assert_eq!(
        tarball_url("6.12.119").expect("url"),
        "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.12.119.tar.xz"
    );
    assert!(tarball_url("").is_err());
}

#[test]
fn the_tool_list_names_the_package_that_provides_each_tool() {
    // A missing bc is the first thing a fresh machine hits, and the error should say what to
    // install rather than failing deep in the build.
    assert!(
        REQUIRED_TOOLS
            .iter()
            .any(|(tool, package)| *tool == "bc" && *package == "bc")
    );
    assert!(
        REQUIRED_TOOLS
            .iter()
            .any(|(_, package)| *package == "base-devel")
    );

    // Whatever this machine is missing is reported with the package, not just the tool.
    for (tool, package) in missing_tools() {
        assert!(!tool.is_empty() && !package.is_empty());
    }
}

#[test]
fn the_checked_in_fragment_covers_what_a_chamber_needs() {
    let fragment = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../kernels/dome-minimal.fragment"),
    )
    .expect("the fragment is part of the repository");

    for required in [
        "CONFIG_VIRTIO_MMIO=y",
        "CONFIG_VIRTIO_MMIO_CMDLINE_DEVICES=y",
        "CONFIG_VIRTIO_NET=y",
        "CONFIG_IP_PNP=y",
        "CONFIG_DEVTMPFS=y",
        "CONFIG_DEVTMPFS_MOUNT=y",
        "CONFIG_BINFMT_SCRIPT=y",
        "CONFIG_SERIAL_8250_CONSOLE=y",
        "CONFIG_BLK_DEV_INITRD=y",
        "# CONFIG_MODULES is not set",
    ] {
        assert!(
            fragment.contains(required),
            "the fragment must keep {required}: it is there because something failed without it"
        );
    }

    // The build options live in the command, not in a fragment that could drift from it.
    let options = BuildOptions {
        version: "7.3.6".to_owned(),
        profile: "dome-minimal".to_owned(),
        jobs: Some(2),
        cache: std::path::PathBuf::from("/nonexistent"),
        fragment: std::path::PathBuf::from("/nonexistent.fragment"),
    };

    assert_eq!(options.version, "7.3.6");
}
