use dome_core::{AperturePolicy, CellSpec, FileApertureMode, GpioDirection, PortProtocol};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blueprint {
    cell: CellSpec,
}

impl Blueprint {
    pub fn new(cell: CellSpec) -> Self {
        Self { cell }
    }

    pub fn cell(&self) -> &CellSpec {
        &self.cell
    }

    pub fn to_toml(&self) -> String {
        let limits = self.cell.limits();
        let mut output = String::new();

        push_line(&mut output, "[cell]");
        push_string(&mut output, "foundation", self.cell.foundation().as_str());
        push_line(&mut output, "");

        push_line(&mut output, "[limits]");
        push_number(&mut output, "memory_mib", limits.memory().as_mib());
        push_number(&mut output, "storage_mib", limits.storage().as_mib());
        push_number(&mut output, "cpus", u64::from(limits.cpus().get()));

        if !self.cell.ports().is_empty() {
            push_line(&mut output, "");
            for port in self.cell.ports() {
                push_line(&mut output, "[[perimeter.ports]]");
                push_string(&mut output, "host_addr", port.host_addr());
                push_number(&mut output, "host_port", u64::from(port.host_port()));
                push_number(&mut output, "guest_port", u64::from(port.guest_port()));
                push_string(&mut output, "protocol", port.protocol().as_str());
            }
        }

        push_apertures(&mut output, self.cell.apertures());

        output
    }
}

fn push_apertures(output: &mut String, apertures: &AperturePolicy) {
    if !apertures.files().is_empty() {
        push_line(output, "");
        for aperture in apertures.files() {
            push_line(output, "[[apertures.files]]");
            push_string(output, "host", aperture.host_path());
            push_string(output, "guest", aperture.guest_path());
            push_file_mode(output, aperture.mode());
        }
    }

    if !apertures.gpio().is_empty() {
        push_line(output, "");
        for aperture in apertures.gpio() {
            push_line(output, "[[apertures.gpio]]");
            push_string(output, "name", aperture.name());
            push_string(output, "guest_chip", aperture.guest_chip());
            push_number(output, "guest_line", u64::from(aperture.guest_line()));
            push_string(output, "host", aperture.host_line());
            push_gpio_direction(output, aperture.direction());
            if let Some(rate_limit_hz) = aperture.rate_limit_hz() {
                push_number(output, "rate_limit_hz", u64::from(rate_limit_hz));
            }
        }
    }
}

fn push_file_mode(output: &mut String, mode: FileApertureMode) {
    push_string(output, "mode", mode.as_str());
}

fn push_gpio_direction(output: &mut String, direction: GpioDirection) {
    push_string(output, "direction", direction.as_str());
}

fn push_string(output: &mut String, key: &str, value: &str) {
    push_line(output, &format!("{key} = \"{}\"", escape_toml(value)));
}

fn push_number(output: &mut String, key: &str, value: u64) {
    push_line(output, &format!("{key} = {value}"));
}

fn push_line(output: &mut String, line: &str) {
    output.push_str(line);
    output.push('\n');
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

pub fn protocol_from_str(value: &str) -> Option<PortProtocol> {
    match value {
        "tcp" => Some(PortProtocol::Tcp),
        "udp" => Some(PortProtocol::Udp),
        _ => None,
    }
}
