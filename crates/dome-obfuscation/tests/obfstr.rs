use dome_obfuscation::{derive_key, mask, obfstr, reveal};

#[test]
fn macro_reveals_original_text() {
    assert_eq!(obfstr!("internal-marker"), "internal-marker");
}

#[test]
fn masked_bytes_do_not_match_plaintext() {
    const TEXT: &str = "sensitive-marker";
    const KEY: u8 = derive_key(file!(), line!(), column!());
    const MASKED: [u8; TEXT.len()] = mask::<{ TEXT.len() }>(TEXT.as_bytes(), KEY);

    assert_ne!(MASKED.as_slice(), TEXT.as_bytes());
    assert_eq!(reveal(&MASKED, KEY), TEXT);
}
