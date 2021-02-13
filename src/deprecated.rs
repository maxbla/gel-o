use crate::{EventsListener, InputEvent, Ptr};
use evdev_rs::UInputDevice;
use std::collections::BTreeMap;
use std::io;
use std::time::{Duration, Instant};

/// Whether to continue grabbing events or to stop
/// Used in `filter_map_events` (and others)
#[deprecated(since = "0.1.0", note = "This is only used with deprecated methods")]
#[derive(Debug, Eq, PartialEq, Hash)]
pub enum GrabStatus {
    /// Stop grabbing
    Continue,
    /// ungrab events
    Stop,
}

#[deprecated(
    since = "0.1.0",
    note = "Use EventListener.iter().filter_map with your own closure that
    schedules delayed events. See examples/delay.rs"
)]
// TODO: return the never type, `!` when it is stabalized
/// Like `filter_map_events_with_delay`, only never returns
pub fn filter_map_events_with_delay_noreturn<F>(mut func: F) -> io::Result<()>
where
    F: FnMut(InputEvent) -> (Instant, Option<InputEvent>),
{
    #[allow(deprecated)]
    filter_map_events_with_delay(|input_event| {
        let (instant, output_event) = func(input_event);
        (instant, output_event, GrabStatus::Continue)
    })
}

#[deprecated(
    since = "0.1.0",
    note = "Use EventListener.iter().take_while().filter_map with your own
    closure that schedules delayed events. See examples/delay.rs"
)]
#[allow(deprecated)]
/// Similar to Iterator's filter_map, only this blocks waiting for input
///
/// Filter and transform events, additionally specifying an Instant at which
/// to simulate the transformed event. To stop grabbing, return
/// `GrabStatus::Stop`
pub fn filter_map_events_with_delay<F>(mut func: F) -> io::Result<()>
where
    F: FnMut(InputEvent) -> (Instant, Option<InputEvent>, GrabStatus),
{
    let mut listener = EventsListener::new(true)?;
    let mut event_iter = listener.iter();
    let mut events_buffer = BTreeMap::<(Instant, u64), (InputEvent, Ptr<UInputDevice>)>::new();
    // Generation acts as a key for events_buffer, so that if two events occur
    // at the same Instant, they can co-exist in the BTreeMap
    let mut generation: u64 = 0;
    'outer: loop {
        let process_later = events_buffer.split_off(&(Instant::now(), 0));
        let process_now = events_buffer;
        events_buffer = process_later;

        for (event, device) in process_now.values() {
            #[cfg(feature = "arc")]
            device.lock().unwrap().write_event(event)?;
            #[cfg(not(feature = "arc"))]
            device.write_event(event)?;
        }

        while events_buffer
            .keys()
            .next()
            .map(|(instant, _)| &Instant::now() < instant)
            .unwrap_or(true)
        {
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
            if let Some((event, device)) = event {
                let (instant, event, grab) = func(event);
                if grab == GrabStatus::Stop {
                    break 'outer Ok(());
                }
                if let Some(event) = event {
                    let entry = events_buffer.insert((instant, generation), (event, device));
                    assert!(
                        entry.is_none(),
                        "Two events occured at the same instant with same generation"
                    );
                    generation = generation.wrapping_add(1);
                }
            }
        }
    }
}

// TODO: return the never type, `!` when it is stabalized
/// Like `filter_map_events`, only never returns
#[deprecated(since = "0.1.0", note = "Use EventListener.iter().filter_map instead")]
pub fn filter_map_events_noreturn<F>(func: F) -> io::Result<()>
where
    F: Fn(InputEvent) -> Option<InputEvent>,
{
    #[allow(deprecated)]
    filter_map_events(|input_event| {
        let output_event = func(input_event);
        (output_event, GrabStatus::Continue)
    })
}

#[deprecated(
    since = "0.1.0",
    note = "Use EventListener.iter().take_while().filter_map instead"
)]
#[allow(deprecated)]
/// Similar to Iterator's filter_map, only this blocks waiting for input
///
/// Filter and transform events. To stop grabbing, return `GrabStatus::Stop`
pub fn filter_map_events<F>(mut func: F) -> io::Result<()>
where
    F: FnMut(InputEvent) -> (Option<InputEvent>, GrabStatus),
{
    let mut listener = EventsListener::new(true)?;
    listener
        .iter()
        .map(|(event, device)| {
            let (new_event, status) = func(event);
            (new_event, device, status)
        })
        .take_while(|(_, _, status)| status != &GrabStatus::Stop)
        .filter_map(|(event, device, _)| event.map(|event| (event, device)))
        .try_for_each(|(event, device)| {
            #[cfg(feature = "arc")]
            return device.lock().unwrap().write_event(&event);
            #[cfg(not(feature = "arc"))]
            device.write_event(&event)
        })
}

#[deprecated(since = "0.1.0", note = "Use EventListener.iter().for_each instead")]
#[allow(deprecated)]
/// Similar to Iterator's for_each, only this blocks waiting for input
///
/// Calls a closure on each event. To stop grabbing, return `GrabStatus::Stop`
pub fn for_each_event<F>(mut func: F) -> io::Result<()>
where
    F: FnMut(InputEvent) -> GrabStatus,
{
    let mut listener = EventsListener::new(false)?;
    for (event, _) in listener.iter() {
        if func(event) == GrabStatus::Stop {
            break;
        }
    }
    Ok(())
}