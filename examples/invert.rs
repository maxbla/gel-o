use evdev_rs::enums::EventType;
use gelo::{filter_map_events, GrabStatus};
use std::thread::sleep;
use std::time::Duration;

/// Inverts all "relative" motion
/// For example mouse motion is relative, scroll wheeling is relative
/// a tablet + stylus is absolute (not relative)
/// key presses are not motion, so are unaffected
/// see https://www.kernel.org/doc/html/latest/input/event-codes.html
fn main() -> std::io::Result<()> {
    // wait for enter key to be released after starting
    // TODO: remove this hack
    sleep(Duration::from_millis(500));

    let mut event_count = 0;
    filter_map_events(move |mut event| {
        if event.event_type == EventType::EV_REL {
            event.value *= -1;
        }
        event_count += 1;
        // Ensure system doesn't become unusable by ungrabbing after many events
        if event_count >= 10000 {
            (None, GrabStatus::Stop)
        } else {
            (Some(event), GrabStatus::Continue)
        }
    })?;

    Ok(())
}
