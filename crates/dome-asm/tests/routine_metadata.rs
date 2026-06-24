use dome_asm::DECLARED_ROUTINES;

#[test]
fn declared_asm_routine_is_metadata_only_for_m1() {
    let routine = DECLARED_ROUTINES[0];

    assert_eq!(routine.architecture(), "x86_64");
    assert!(routine.purpose().contains("not linked in milestone 1"));
}
