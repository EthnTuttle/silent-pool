use spc_lab_txd::bootstrap::{BatchConsumption, HashAuthorization, HashReservationStatus};
#[test]
fn duplicate_lost_ack_and_old_incarnation_never_grant_another_execution() {
    let authorization = HashAuthorization {
        operation_id: "operation".into(),
        driver_incarnation: "inc-1".into(),
        batch_id: 1,
        reserved_attempts: 20,
        cumulative_reserved: 20,
    };
    let status = HashReservationStatus {
        authorization: authorization.clone(),
        duplicate: false,
        reconciled: false,
        measured_attempts: None,
    };
    let mut actor = BatchConsumption::new("operation".into(), "inc-1".into());
    assert!(actor.consume(&status).unwrap());
    assert!(!actor.consume(&status).unwrap());
    let mut restarted = BatchConsumption::new("operation".into(), "inc-2".into());
    assert!(restarted.consume(&status).is_err());
    let mut lost = BatchConsumption::new("operation".into(), "inc-1".into());
    let duplicate = HashReservationStatus {
        duplicate: true,
        ..status.clone()
    };
    assert!(!lost.consume(&duplicate).unwrap());
    assert!(!lost.consume(&status).unwrap());
    let conflict = HashReservationStatus {
        authorization: HashAuthorization {
            reserved_attempts: 19,
            ..authorization
        },
        ..status
    };
    assert!(actor.consume(&conflict).is_err());
}
