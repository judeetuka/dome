use dome_core::{DomeError, FoundationName, StorageSize};
use dome_image::{FoundationFormat, FoundationManifest};

#[test]
fn dome_foundation_has_canonical_extension() {
    assert_eq!(
        FoundationFormat::DomeFoundation.extension(),
        ".foundation.dome"
    );
}

#[test]
fn manifest_uses_foundation_names() -> Result<(), DomeError> {
    let manifest = FoundationManifest::new(
        FoundationName::new("dome/buildroot:minimal")?,
        FoundationFormat::DomeFoundation,
        StorageSize::from_mib(256)?,
        "foundations/buildroot.foundation.dome",
    );

    assert_eq!(manifest.name().as_str(), "dome/buildroot:minimal");
    assert_eq!(manifest.format(), FoundationFormat::DomeFoundation);

    Ok(())
}
