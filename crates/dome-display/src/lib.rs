use dome_core::DisplayMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayAperture {
    mode: DisplayMode,
    listen_addr: String,
}

impl DisplayAperture {
    pub fn disabled() -> Self {
        Self {
            mode: DisplayMode::None,
            listen_addr: "127.0.0.1".to_owned(),
        }
    }

    pub fn vnc_localhost() -> Self {
        Self {
            mode: DisplayMode::Vnc,
            listen_addr: "127.0.0.1".to_owned(),
        }
    }

    pub fn mode(&self) -> DisplayMode {
        self.mode
    }

    pub fn listen_addr(&self) -> &str {
        &self.listen_addr
    }
}
