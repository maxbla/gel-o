use evdev_rs::{enums::EventCode, enums::EV_SYN};

/// This example requires feature "serde"
/// It demonstrates how to serialize and deserialize an event using serde
fn main() -> serde_json::Result<()> {
    #[cfg(not(feature = "serde"))]
    compile_error!(
        "This example requires feature serde.
        Try rerunning with --features=serde"
    );

    for event in EventCode::EV_SYN(EV_SYN::SYN_REPORT).iter() {
        // Prints EV_SYN(SYN_REPORT)
        println!("{:?}", event);
        let json: String = serde_json::to_string(&event)?;
        // Prints {"EV_SYN":"SYN_REPORT"}
        println!("{}", json);
        let deserialized_event: EventCode = serde_json::from_str(&json)?;
        assert_eq!(deserialized_event, event);
    }
    Ok(())
}
