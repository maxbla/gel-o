use gelo::{for_each_event, GrabStatus};
use std::io;

fn main() -> io::Result<()> {
    let mut event_count = 0;
    for_each_event(|event| {
        println!("{:?}", event);
        event_count += 1;
        if event_count >= 10000 {
            GrabStatus::Stop
        } else {
            GrabStatus::Continue
        }
    })?;
    Ok(())
}
