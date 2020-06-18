use gelo::{filter_map_events_with_delay, GrabStatus};
use std::thread::sleep;
use std::time::{Duration, Instant};
use std::env;

const DEFAULT_MS_DELAY:u32 = 60;

fn main() -> std::io::Result<()> {
    let ms_delay = env::args().nth(1).map(|arg| arg.parse().unwrap());
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

    let mut event_count = 0;
    filter_map_events_with_delay(move |event| {
        event_count += 1;
        // Ensure system doesn't become unusable by ungrabbing after many events
        if event_count == 10000 {
            (Instant::now(), None, GrabStatus::Stop)
        } else {
            let sim_inst = Instant::now() + Duration::from_millis(ms_delay.into());
            (sim_inst, Some(event), GrabStatus::Ungrab)
        }
    })?;
    Ok(())
}
