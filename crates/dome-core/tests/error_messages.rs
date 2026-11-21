use dome_core::DomeError;

#[test]
fn dome_errors_render_stable_messages() {
    assert_eq!(
        DomeError::EmptyChamberId.to_string(),
        "chamber id cannot be empty"
    );
    assert_eq!(
        DomeError::EmptyFoundationName.to_string(),
        "foundation name cannot be empty"
    );
    assert_eq!(
        DomeError::EmptyHostPath.to_string(),
        "host aperture path cannot be empty"
    );
    assert_eq!(
        DomeError::EmptyGuestPath.to_string(),
        "guest aperture path cannot be empty"
    );
    assert_eq!(DomeError::InvalidPort(0).to_string(), "port 0 is invalid");
    assert_eq!(
        DomeError::ZeroCpuCount.to_string(),
        "cpu count must be greater than zero"
    );
    assert_eq!(
        DomeError::ZeroGpioLine.to_string(),
        "gpio line must be greater than zero"
    );
    assert_eq!(
        DomeError::ZeroMemory.to_string(),
        "memory must be greater than zero"
    );
    assert_eq!(
        DomeError::ZeroStorage.to_string(),
        "storage must be greater than zero"
    );
}

#[test]
fn dome_error_works_as_a_std_error() {
    fn assert_std_error<E: std::error::Error>() {}
    assert_std_error::<DomeError>();

    let boxed: Box<dyn std::error::Error> = Box::new(DomeError::ZeroMemory);
    assert_eq!(boxed.to_string(), "memory must be greater than zero");
}
