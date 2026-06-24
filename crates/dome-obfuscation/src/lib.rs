pub const fn derive_key(file: &str, line: u32, column: u32) -> u8 {
    let bytes = file.as_bytes();
    let mut index = 0;
    let mut key = 0xA7u8
        .wrapping_add(line as u8)
        .rotate_left(1)
        .wrapping_add(column as u8)
        .rotate_left(3);

    while index < bytes.len() {
        key = key.wrapping_add(bytes[index]).rotate_left(1) ^ 0x5D;
        index += 1;
    }

    if key == 0 { 0xC3 } else { key }
}

pub const fn mask<const N: usize>(plain: &[u8], key: u8) -> [u8; N] {
    let mut output = [0u8; N];
    let mut index = 0;

    while index < N {
        output[index] = plain[index] ^ stream_byte(key, index);
        index += 1;
    }

    output
}

pub fn reveal(masked: &[u8], key: u8) -> String {
    let mut output = Vec::with_capacity(masked.len());

    for (index, byte) in masked.iter().enumerate() {
        output.push(*byte ^ stream_byte(key, index));
    }

    match String::from_utf8(output) {
        Ok(value) => value,
        Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
    }
}

const fn stream_byte(key: u8, index: usize) -> u8 {
    let mixed = key
        .wrapping_add((index as u8).wrapping_mul(31))
        .rotate_left((index % 7) as u32);
    mixed ^ 0xA5
}

#[macro_export]
macro_rules! obfstr {
    ($literal:literal) => {{
        const KEY: u8 = $crate::derive_key(file!(), line!(), column!());
        const MASKED: [u8; $literal.len()] =
            $crate::mask::<{ $literal.len() }>($literal.as_bytes(), KEY);
        $crate::reveal(&MASKED, KEY)
    }};
}
