#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct AsmRoutine {
    architecture: &'static str,
    path: &'static str,
    purpose: RoutinePurpose,
}

impl AsmRoutine {
    pub const fn new(
        architecture: &'static str,
        path: &'static str,
        purpose: RoutinePurpose,
    ) -> Self {
        Self {
            architecture,
            path,
            purpose,
        }
    }

    pub const fn architecture(self) -> &'static str {
        self.architecture
    }

    pub const fn path(self) -> &'static str {
        self.path
    }

    pub fn purpose(self) -> String {
        match self.purpose {
            // obfuscation: anti-strings hygiene only; not a security boundary; reason: internal release-only routine marker.
            RoutinePurpose::CpuProbePlaceholder => {
                dome_obfuscation::obfstr!(
                    "placeholder CPU probe entrypoint; not linked in milestone 1"
                )
            }
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RoutinePurpose {
    CpuProbePlaceholder,
}

pub const DECLARED_ROUTINES: &[AsmRoutine] = &[AsmRoutine::new(
    "x86_64",
    "asm/x86_64/cpuid_vendor.S",
    RoutinePurpose::CpuProbePlaceholder,
)];
