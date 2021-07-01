#[cfg(not(feature = "arc"))]
fn main() {
    eprintln!(
        "This example requires feature arc.
        Try rerunning with --features=arc"
    );
}

#[cfg(feature = "arc")]
fn main() -> std::io::Result<()> {
    use evdev_rs::{enums::InputProp, Device, DeviceWrapper, InputEvent, UInputDevice};
    use gelo::EventsListener;
    use std::{
        collections::BTreeMap,
        env,
        sync::mpsc::{channel, Receiver},
        sync::{Arc, Mutex},
        thread::sleep,
        time::{Duration, Instant},
    };

    const DEFAULT_MS_DELAY: u32 = 60;
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

    //TODO: remove this hack
    //wait for enter key to be released after starting
    sleep(Duration::from_millis(500));

    let (send, recv): (_, Receiver<(_, _, Arc<_>)>) = channel();

    std::thread::spawn(move || -> std::io::Result<()> {
        // Listen on mice, but not keyboards
        let mut listener = EventsListener::new_with_filter(true, |device: &Device| {
            device.has_event_type(&evdev_rs::enums::EventType::EV_REL)
                && device.has_event_code(&EventCode::EV_KEY(EV_KEY::BTN_LEFT))
        })?;
        let delay = Duration::from_millis(ms_delay.into());
        for (event, device) in listener.iter().take(10000) {
            send.send((Instant::now() + delay, event.clone(), device.clone()))
                .unwrap()
        }
        Ok(())
    });

    let mut events_buffer = BTreeMap::<Instant, (InputEvent, Arc<Mutex<UInputDevice>>)>::new();
    for (recv_instant, event, device) in recv.iter() {
        let process_later = events_buffer.split_off(&Instant::now());
        let process_now = events_buffer;
        events_buffer = process_later;

        for (event, device) in process_now.values() {
            device.lock().unwrap().write_event(event)?;
        }
        events_buffer.insert(recv_instant, (event, device));
    }
    Ok(())
}
