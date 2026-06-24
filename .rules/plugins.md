# Plugin rules

Dome plugins extend chamber telemetry, policy decisions, enterprise defense, and audit export.

Plugin forms:

- local signed plugin: proprietary code runs on the customer's host after signature and license checks
- remote service plugin: Dome sends approved telemetry to a remote decision service and receives policy decisions
- attested plugin: stronger mode that requires measured boot, TPM/secure enclave, or confidential-computing support

Plugins must preserve host transparency. Do not implement process hiding, host-security bypass, stealth persistence, covert traffic, audit suppression, or anti-debug behavior against the machine owner/admin.

Every plugin must declare:

- plugin identity and version
- execution mode
- license tier
- capabilities requested
- telemetry event types consumed
- decisions it may return
- failure mode
- data retention posture

Enterprise defense plugins must live behind a signed plugin or remote service boundary. Protect proprietary logic with code signing, encrypted model weights, license leases, attestation, measured boot, confidential computing, or model-as-a-service where justified.

Sentinel telemetry is useful but not authoritative. A compromised chamber can lie, so host-side perimeter and aperture brokers must remain independent.

Traffic monitoring must use explicit perimeter or broker mechanisms. Log policy decisions, connection metadata where allowed, aperture access, sentinel events, plugin decisions, license checks, and administrative actions.
