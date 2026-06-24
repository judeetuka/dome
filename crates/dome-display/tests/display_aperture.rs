use dome_core::DisplayMode;
use dome_display::DisplayAperture;

#[test]
fn vnc_display_aperture_binds_localhost() {
    let aperture = DisplayAperture::vnc_localhost();

    assert_eq!(aperture.mode(), DisplayMode::Vnc);
    assert_eq!(aperture.listen_addr(), "127.0.0.1");
}
