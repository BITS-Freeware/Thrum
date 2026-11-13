#![no_main]

use thrum_core::mls::DeviceCrypto;
use libfuzzer_sys::fuzz_target;
use std::sync::OnceLock;

/// One device pair with a joined group, shared across inputs so the
/// property includes "group state survives garbage".
struct Setup {
    alice: DeviceCrypto,
    bob: DeviceCrypto,
    conversation: String,
}

fn setup() -> &'static Setup {
    static SETUP: OnceLock<Setup> = OnceLock::new();
    SETUP.get_or_init(|| {
        let alice = DeviceCrypto::generate("alice#device-1".to_string());
        let bob = DeviceCrypto::generate("bob#device-1".to_string());
        alice.create_group("conv-fuzz").expect("create group");
        let (_commit, welcome) = alice
            .add_members("conv-fuzz", &[bob.new_key_package().expect("key package")])
            .expect("add member");
        let joined = bob
            .process_welcome(&welcome.expect("welcome"))
            .expect("join");
        assert_eq!(joined, "conv-fuzz");
        Setup {
            alice,
            bob,
            conversation: "conv-fuzz".to_string(),
        }
    })
}

fuzz_target!(|data: &[u8]| {
    let setup = setup();
    let _ = setup.bob.process_incoming(&setup.conversation, data);
    let _ = setup.alice.key_package_identity(data);
    let _ = DeviceCrypto::generate("carol#device-1".to_string()).process_welcome(data);
});
