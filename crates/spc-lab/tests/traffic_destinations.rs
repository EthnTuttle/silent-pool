//! Controller and core share the same bounded address corpus and policy.
#[path = "../../spc-lab-txd/tests/common/destinations.rs"]
mod destinations;

use spc_lab::ids::IdentityId;
use spc_lab::traffic::commands::SendUserPayment;

#[test]
fn controller_destination_admission_matches_core_address_corpus() {
    for (addresses, admitted) in [
        (destinations::admitted_addresses(), true),
        (destinations::rejected_addresses(), false),
    ] {
        for destination_address in addresses {
            let command = SendUserPayment {
                user: IdentityId::trusted("alice"),
                destination_address,
                amount_sats: 100_000,
                fee_rate_sats_vb: 3,
            };
            assert_eq!(
                command.validate().is_ok(),
                admitted,
                "{}",
                command.destination_address
            );
        }
    }
}
