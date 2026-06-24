use dome_guest_agent::{SentinelCapability, SentinelContract};

#[test]
fn sentinel_contract_lists_capabilities() {
    let contract = SentinelContract::new(1, vec![SentinelCapability::FilesystemFreeze]);

    assert_eq!(contract.protocol_version(), 1);
    assert_eq!(
        contract.capabilities(),
        &[SentinelCapability::FilesystemFreeze]
    );
}
