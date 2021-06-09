use evdev_rs::{Device, InputEvent, UInputDevice, LibevdevWrapper};
use gelo::{EventsListener, Ptr};
use std::collections::BTreeMap;
use std::env;
use std::thread::sleep;
use std::time::{Duration, Instant};

const DEFAULT_MS_DELAY: u32 = 60;

fn main() -> std::io::Result<()> {
    let ms_delay = env::args().nth(1).map(|arg| arg.parse().ok()).flatten();
    let ms_delay = match ms_delay {
        None => {
            println!("No delay set, using default of {}", DEFAULT_MS_DELAY);
            DEFAULT_MS_DELAY
        }
        Some(delay) => {
            println!("Delay set to {}ms", delay);
            delay
        }
    };
    let delay_dur = Duration::from_millis(ms_delay.into());

    // TODO: remove this hack
    // wait for enter key to be released after starting
    sleep(Duration::from_millis(100));

    // Listen on mice (EV_REL), but not keyboards
    let mut listener = EventsListener::new_with_filter(
        true,
        |device: &Device| device.has_event_type(&evdev_rs::enums::EventType::EV_REL)
    )?;
    let mut event_iter = listener.iter();
    // A Map of (time to simulate event, generation) -> (Event to simulate, device on which to simulate it)
    let mut events_buffer = BTreeMap::<(Instant, u64), (InputEvent, Ptr<UInputDevice>)>::new();
    // Generation acts as a key for events_buffer, so that if two events occur
    // at the same Instant, they can co-exist in the BTreeMap
    let mut generation: u64 = 0;

    loop {
        let process_later = events_buffer.split_off(&(Instant::now(), 0));
        let process_now = events_buffer;
        events_buffer = process_later;

        // send events to devices if their key (time to simulate event) is in the past
        for (event, device) in process_now.values() {
            #[cfg(feature = "arc")]
            device.lock().unwrap().write_event(event)?;
            #[cfg(not(feature = "arc"))]
            device.write_event(event)?;
        }

        // while the next event to simulate is in the future or there is no next event
        while events_buffer
            .keys()
            .next()
            .map(|(instant, _)| &Instant::now() < instant)
            .unwrap_or(true)
        {
            // Wait for a new event just long enough that you won't delay the
            // next event that needs to be sent to a device
            let event = match events_buffer.keys().next() {
                None => event_iter.next_timeout(None)?,
                Some((send_instant, _)) => {
                    let now = Instant::now();
                    if &now < send_instant {
                        event_iter.next_timeout(Some(send_instant.duration_since(now)))?
                    } else {
                        event_iter.next_timeout(Some(Duration::from_millis(0)))?
                    }
                }
            };
            // Add newly recieved events to the event buffer
            if let Some(event) = event {
                let entry = events_buffer.insert((Instant::now() + delay_dur, generation), event);
                assert!(
                    entry.is_none(),
                    "Two events occured at the same instant with same generation"
                );
                generation = generation.wrapping_add(1);
            }
        }
    }
}
