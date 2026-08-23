#![cfg(feature = "native-abi")]

use keld_runtime::{EntityId, Link, RuntimeLifecycleId};

#[test]
fn native_identity_records_round_trip_through_raw_parts() {
    let entity = EntityId::from_raw_parts(7, 11, 13, 17);
    assert_eq!(entity.raw_parts(), (7, 11, 13, 17));
    let link = Link::from_raw_parts(7, 11, 13, 17);
    assert_eq!(link.raw_parts(), (7, 11, 13, 17));
    let lifecycle = RuntimeLifecycleId::from_raw_parts(7, 19);
    assert_eq!(lifecycle.raw_parts(), (7, 19));
}
