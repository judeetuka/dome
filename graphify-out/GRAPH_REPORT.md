# Graph Report - dome  (2026-06-24)

## Corpus Check
- 39 files · ~6,679 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 403 nodes · 655 edges · 38 communities (28 shown, 10 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS · INFERRED: 1 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- [[_COMMUNITY_Community 0|Community 0]]
- [[_COMMUNITY_Community 1|Community 1]]
- [[_COMMUNITY_Community 2|Community 2]]
- [[_COMMUNITY_Community 3|Community 3]]
- [[_COMMUNITY_Community 4|Community 4]]
- [[_COMMUNITY_Community 5|Community 5]]
- [[_COMMUNITY_Community 6|Community 6]]
- [[_COMMUNITY_Community 7|Community 7]]
- [[_COMMUNITY_Community 8|Community 8]]
- [[_COMMUNITY_Community 9|Community 9]]
- [[_COMMUNITY_Community 10|Community 10]]
- [[_COMMUNITY_Community 11|Community 11]]
- [[_COMMUNITY_Community 12|Community 12]]
- [[_COMMUNITY_Community 13|Community 13]]
- [[_COMMUNITY_Community 14|Community 14]]
- [[_COMMUNITY_Community 15|Community 15]]
- [[_COMMUNITY_Community 16|Community 16]]
- [[_COMMUNITY_Community 17|Community 17]]
- [[_COMMUNITY_Community 18|Community 18]]
- [[_COMMUNITY_Community 19|Community 19]]
- [[_COMMUNITY_Community 20|Community 20]]
- [[_COMMUNITY_Community 21|Community 21]]
- [[_COMMUNITY_Community 22|Community 22]]
- [[_COMMUNITY_Community 23|Community 23]]
- [[_COMMUNITY_Community 27|Community 27]]
- [[_COMMUNITY_Community 28|Community 28]]
- [[_COMMUNITY_Community 29|Community 29]]
- [[_COMMUNITY_Community 30|Community 30]]
- [[_COMMUNITY_Community 31|Community 31]]
- [[_COMMUNITY_Community 32|Community 32]]
- [[_COMMUNITY_Community 33|Community 33]]
- [[_COMMUNITY_Community 34|Community 34]]
- [[_COMMUNITY_Community 35|Community 35]]
- [[_COMMUNITY_Community 36|Community 36]]
- [[_COMMUNITY_Community 37|Community 37]]

## God Nodes (most connected - your core abstractions)
1. `Self` - 20 edges
2. `CellSpec` - 19 edges
3. `PluginManifest` - 18 edges
4. `GpioAperture` - 16 edges
5. `DomeError` - 14 edges
6. `PortMap` - 13 edges
7. `HostReport` - 13 edges
8. `AperturePolicy` - 12 edges
9. `Result` - 11 edges
10. `ResourceLimits` - 11 edges

## Surprising Connections (you probably didn't know these)
- `masked_bytes_do_not_match_plaintext()` --calls--> `derive_key()`  [INFERRED]
  crates/dome-obfuscation/tests/obfstr.rs → crates/dome-obfuscation/src/lib.rs

## Import Cycles
- 1-file cycle: `crates/dome-cli/src/main.rs -> crates/dome-cli/src/main.rs`
- 1-file cycle: `crates/dome-display/src/lib.rs -> crates/dome-display/src/lib.rs`
- 1-file cycle: `crates/dome-host-linux/src/lib.rs -> crates/dome-host-linux/src/lib.rs`
- 1-file cycle: `crates/dome-image/src/lib.rs -> crates/dome-image/src/lib.rs`
- 1-file cycle: `crates/dome-supervisor/src/lib.rs -> crates/dome-supervisor/src/lib.rs`

## Communities (38 total, 10 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.08
Nodes (29): AssemblerReport, CgroupReport, Into, Option, Path, PathBuf, Self, escape_toml() (+21 more)

### Community 1 - "Community 1"
Cohesion: 0.09
Nodes (13): AperturePolicy, NonZeroU64, CellSpec, ChamberId, CpuCount, DisplayMode, EgressPolicy, FoundationName (+5 more)

### Community 2 - "Community 2"
Cohesion: 0.28
Nodes (6): String, derive_key(), mask(), reveal(), stream_byte(), masked_bytes_do_not_match_plaintext()

### Community 4 - "Community 4"
Cohesion: 0.20
Nodes (9): Into, Path, PathBuf, Self, FoundationName, FoundationFormat, FoundationManifest, SealMode (+1 more)

### Community 5 - "Community 5"
Cohesion: 0.31
Nodes (6): Into, Self, String, DeviceCandidate, DeviceClass, PassthroughReadiness

### Community 6 - "Community 6"
Cohesion: 0.29
Nodes (5): Self, Vec, EgressPolicy, PortMap, PerimeterPolicy

### Community 7 - "Community 7"
Cohesion: 0.10
Nodes (17): Display, Error, Formatter, Into, Option, Result, Self, String (+9 more)

### Community 8 - "Community 8"
Cohesion: 0.24
Nodes (4): Self, String, DisplayMode, DisplayAperture

### Community 9 - "Community 9"
Cohesion: 0.39
Nodes (4): ChamberId, Self, ChamberRecord, ChamberState

### Community 10 - "Community 10"
Cohesion: 0.39
Nodes (4): Self, Vec, SentinelCapability, SentinelContract

### Community 11 - "Community 11"
Cohesion: 0.22
Nodes (5): Self, String, RoutinePurpose, AsmRoutine, RoutinePurpose

### Community 12 - "Community 12"
Cohesion: 0.20
Nodes (9): Apertures, Architecture baseline, Current command, Dome, Foundation format, Plugins, Roadmap, Verification (+1 more)

### Community 13 - "Community 13"
Cohesion: 0.48
Nodes (5): DomeError, Result, apertures_are_explicit(), builds_resource_limits(), local_tcp_port_map_is_explicit()

### Community 14 - "Community 14"
Cohesion: 0.60
Nodes (4): ProbeRoot, fixture_root(), renders_toml_capabilities(), reports_fixture_capabilities()

### Community 15 - "Community 15"
Cohesion: 0.20
Nodes (17): AperturePolicy, CellSpec, Option, Self, escape_toml(), String, FileApertureMode, GpioDirection (+9 more)

### Community 16 - "Community 16"
Cohesion: 0.21
Nodes (15): CliError, Display, Error, Formatter, Result, Self, String, Vec (+7 more)

### Community 17 - "Community 17"
Cohesion: 0.19
Nodes (12): CellSpec, ChamberId, Self, String, Vec, AccelerationMode, CpuArch, Engine (+4 more)

### Community 18 - "Community 18"
Cohesion: 0.53
Nodes (5): Box, Error, Result, doctor_defaults_to_toml(), help_mentions_read_only_milestone()

### Community 19 - "Community 19"
Cohesion: 0.40
Nodes (3): DomeError, Result, launch_plan_uses_chamber_and_cell_names()

### Community 20 - "Community 20"
Cohesion: 0.40
Nodes (3): DomeError, Result, manifest_uses_foundation_names()

### Community 21 - "Community 21"
Cohesion: 0.40
Nodes (3): DomeError, Result, publish_adds_explicit_port()

### Community 22 - "Community 22"
Cohesion: 0.50
Nodes (3): DomeError, Result, renders_blueprint_as_toml()

### Community 23 - "Community 23"
Cohesion: 0.50
Nodes (3): DomeError, Result, chamber_record_tracks_state()

### Community 35 - "Community 35"
Cohesion: 0.08
Nodes (25): ChamberId, Display, Error, Formatter, Into, PluginError, Result, Self (+17 more)

### Community 36 - "Community 36"
Cohesion: 0.28
Nodes (8): Box, DomeError, Error, PluginError, Result, enterprise_manifest_declares_requested_capabilities(), invocation_keeps_chamber_identity_host_visible(), verdict_models_policy_decisions()

## Knowledge Gaps
- **85 isolated node(s):** `Self`, `String`, `RoutinePurpose`, `Vec`, `Self` (+80 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **10 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `GpioAperture` connect `Community 7` to `Community 1`?**
  _High betweenness centrality (0.008) - this node is a cross-community bridge._
- **Why does `CellSpec` connect `Community 1` to `Community 7`?**
  _High betweenness centrality (0.007) - this node is a cross-community bridge._
- **Why does `PortMap` connect `Community 1` to `Community 7`?**
  _High betweenness centrality (0.005) - this node is a cross-community bridge._
- **What connects `Self`, `String`, `RoutinePurpose` to the rest of the system?**
  _85 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.07926829268292683 - nodes in this community are weakly interconnected._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.0931174089068826 - nodes in this community are weakly interconnected._
- **Should `Community 7` be split into smaller, more focused modules?**
  _Cohesion score 0.0966183574879227 - nodes in this community are weakly interconnected._