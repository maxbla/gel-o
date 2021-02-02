use evdev_rs::enums::EventCode;
use gelo::EventsListener;
use std::io;

/// Print linux input events as they come.
/// Stop after 10,000 events or the escape key is pressed
fn main() -> io::Result<()> {
    let mut listener = EventsListener::new(false)?;
    for (event, _) in listener.iter().take(10000) {
        // Stop listening if the escape key is pressed
        if event.event_code == EventCode::EV_KEY(evdev_rs::enums::EV_KEY::KEY_ESC) {
            break;
        }
        // Pressing an 'a' key press and release looks like this
        // EV_MSC(MSC_SCAN) value: 458756   <-- safe to ignore, related to how keyboards work
        // EV_KEY(KEY_A) value: 1           <-- the a key was pressed down
        // EV_SYN(SYN_REPORT) value: 0      <-- process events since last SYN_REPORT
        //                                      SYN_REPORT is used to batch events that occur at the same time
        //                                      e.g. a mouse moving diagonally moves in x and y simultaneously
        // aEV_MSC(MSC_SCAN) value: 458756  <-- save to ignore. Note it is the same as first event
        //                                  <-- note the a starting this line
        // EV_KEY(KEY_A) value: 0           <-- the a key was lifted back up
        // EV_SYN(SYN_REPORT) value: 0      <-- process events since last SYN_REPORT
        println!("{:?} value: {}", event.event_code, event.value);
    }

    Ok(())
}
