use evdev_rs::{InputEvent, enums::EventCode, enums::EV_SYN};
use gelo::EventsListener;
use std::io;
use serde_json;

/// This example requires feature "serde"
/// It demonstrates how to serialize and deserialize an event using serde
fn main() -> serde_json::Result<()> {
    for event in EventCode::EV_SYN(EV_SYN::SYN_REPORT).iter() {
        println!("{:?}", event);
        let json:String = serde_json::to_string(&event)?;
        println!("{}", json);
        let deserialized_event:EventCode = serde_json::from_str(&json)?;
        assert!(deserialized_event == event);
    }
    Ok(())
}