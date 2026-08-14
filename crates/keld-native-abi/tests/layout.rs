use keld_native_abi::{
    ABI_VERSION, FaultKind, KeldEntity, KeldFault, KeldHandle, KeldLifecycle, KeldLink,
    KeldPlaceStep, KeldValue, RuntimeStatus,
};
use std::mem::{align_of, offset_of, size_of};

#[test]
fn versioned_records_have_the_frozen_windows_layout() {
    assert_eq!(ABI_VERSION, 1);
    assert_eq!(size_of::<KeldHandle>(), 8);
    assert_eq!(align_of::<KeldHandle>(), 8);
    assert_eq!(size_of::<KeldEntity>(), 24);
    assert_eq!(align_of::<KeldEntity>(), 8);
    assert_eq!(offset_of!(KeldEntity, brand), 0);
    assert_eq!(offset_of!(KeldEntity, slot), 8);
    assert_eq!(offset_of!(KeldEntity, generation), 12);
    assert_eq!(offset_of!(KeldEntity, definition), 16);
    assert_eq!(offset_of!(KeldEntity, reserved), 20);
    assert_eq!(size_of::<KeldLink>(), 24);
    assert_eq!(offset_of!(KeldLink, expected), 16);
    assert_eq!(size_of::<KeldLifecycle>(), 16);
    assert_eq!(offset_of!(KeldLifecycle, index), 8);
    assert_eq!(size_of::<KeldValue>(), 32);
    assert_eq!(offset_of!(KeldValue, words), 8);
    assert_eq!(size_of::<KeldFault>(), 8);
    assert_eq!(size_of::<KeldPlaceStep>(), 16);
    assert_eq!(offset_of!(KeldPlaceStep, value), 8);
}

#[test]
fn numeric_abi_discriminants_are_frozen() {
    assert_eq!(RuntimeStatus::Ok as u32, 0);
    assert_eq!(RuntimeStatus::LanguageFault as u32, 1);
    assert_eq!(RuntimeStatus::InternalFailure as u32, 2);
    assert_eq!(FaultKind::Arithmetic as u32, 1);
    assert_eq!(FaultKind::DivisionByZero as u32, 2);
    assert_eq!(FaultKind::Shift as u32, 3);
    assert_eq!(FaultKind::Allocation as u32, 4);
    assert_eq!(FaultKind::Capacity as u32, 5);
    assert_eq!(FaultKind::Bounds as u32, 6);
    assert_eq!(KeldHandle::EMPTY.0, 0);
}
