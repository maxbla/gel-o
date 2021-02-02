use evdev_rs::enums::EventType;
use gelo::EventsListener;
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

    let mut listener = EventsListener::new(true)?;
    for (mut event, device) in listener.iter().take(1) {
        if event.event_type() == Some(EventType::EV_REL) {
            event.value *= -1;
        }
        #[cfg(feature = "arc")]
        device.lock().unwrap().write_event(&event)?;
        #[cfg(not(feature = "arc"))]
        device.write_event(&event)?;
    }

    Ok(())
}
