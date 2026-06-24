use dome_core::{ChamberId, DomeError, PortProtocol};
use dome_plugin::{
    LicenseState, LicenseTier, PluginCapability, PluginDecision, PluginError, PluginExecutionMode,
    PluginFailureMode, PluginId, PluginInvocation, PluginKind, PluginManifest, PluginVerdict,
    PluginVersion, TelemetryEvent,
};

#[test]
fn enterprise_manifest_declares_requested_capabilities() -> Result<(), PluginError> {
    let manifest = PluginManifest::new(
        PluginId::new("dome-enterprise-defense")?,
        PluginVersion::new("0.1.0")?,
        PluginKind::EnterpriseDefense,
        PluginExecutionMode::RemoteService,
        LicenseTier::EnterpriseService,
        PluginFailureMode::FailClosed,
    )
    .with_capability(PluginCapability::ObserveNetworkFlows)
    .with_capability(PluginCapability::DecidePerimeterTraffic);

    assert_eq!(manifest.id().as_str(), "dome-enterprise-defense");
    assert_eq!(manifest.execution(), PluginExecutionMode::RemoteService);
    assert!(
        manifest
            .capabilities()
            .contains(&PluginCapability::DecidePerimeterTraffic)
    );

    Ok(())
}

#[test]
fn invocation_keeps_chamber_identity_host_visible() -> Result<(), Box<dyn std::error::Error>> {
    let chamber_id = ChamberId::new("api-prod")?;
    let event = TelemetryEvent::NetworkFlowObserved {
        chamber_id,
        protocol: PortProtocol::Tcp,
        remote: "203.0.113.10:443".to_owned(),
        guest_port: 443,
    };
    let invocation = PluginInvocation::new(
        PluginId::new("dome-enterprise-defense")?,
        LicenseState::Active,
        event,
    );

    assert_eq!(invocation.event().chamber_id().as_str(), "api-prod");
    assert_eq!(invocation.license_state(), LicenseState::Active);

    Ok(())
}

#[test]
fn verdict_models_policy_decisions() -> Result<(), DomeError> {
    let verdict = PluginVerdict::deny("suspicious-flow");

    assert_eq!(verdict.decision(), PluginDecision::Deny);
    assert_eq!(verdict.reason_code(), "suspicious-flow");

    Ok(())
}
