use std::thread::sleep;
use std::error::Error;
use std::time::{Duration, Instant};
use libgelo::{filter_map_events_with_delay, GrabStatus};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let ms_delay: u32 = args.get(1).map(|arg| arg.parse().unwrap()).unwrap_or(60);
    println!("Delay set to {}ms", ms_delay);

    //TODO: remove this hack
    //wait for enter key to be released after starting
    sleep(Duration::from_millis(500));

    let mut event_count = 0;
    filter_map_events_with_delay(move |event| {
        event_count += 1;
        if event_count == 10000 {
            return (Instant::now(), None, GrabStatus::Stop);
        }
        let sim_inst = Instant::now() + Duration::from_millis(ms_delay.into());
        (sim_inst, Some(event), GrabStatus::Continue)
    })?;

    // filter_map_events(move |event| {
    //     event_count += 1;
    //     if event_count == 10000 {
    //         return (None, GrabStatus::Stop);
    //     }
    //     (Some(event), GrabStatus::Continue)
    // })?;
    Ok(())
}
