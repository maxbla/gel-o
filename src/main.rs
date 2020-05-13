use evdev_rs::{Device, InputEvent, UInputDevice};
use std::collections::BTreeMap;
use std::convert::TryInto;
use std::error::Error;
use std::ffi::OsStr;
use std::fs::{read_dir, File};
use std::io;
use std::os::unix::{ffi::OsStrExt, fs::FileTypeExt, io::AsRawFd, io::RawFd};
use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, Instant};

static DEV_PATH: &str = "/dev/input";

fn get_device_files<T>(path: T) -> io::Result<Vec<File>>
where
    T: AsRef<Path>,
{
    let mut res = Vec::new();
    for entry in read_dir(path)? {
        let entry = entry?;
        // /dev/input files are character devices
        if !entry.file_type()?.is_char_device() {
            continue;
        }

        let path = entry.path();
        let file_name_bytes = match path.file_name() {
            Some(file_name) => file_name.as_bytes(),
            None => continue, // file_name was "..", should be impossible
        };
        // skip filenames matching "mouse.* or mice".
        // these files don't play nice with libevdev, not sure why
        // see: https://askubuntu.com/questions/1043832/difference-between-dev-input-mouse0-and-dev-input-mice
        if file_name_bytes == OsStr::new("mice").as_bytes()
            || &file_name_bytes[0..=4] == OsStr::new("mouse").as_bytes()
        {
            continue;
        }
        res.push(File::open(path)?);
    }
    Ok(res)
}

fn epoll_watch_all<'a, T>(device_files: T) -> io::Result<RawFd>
where
    T: Iterator<Item = &'a File>,
{
    let epoll_fd = epoll::create(true)?;
    // add file descriptors to epoll
    for (file_idx, file) in device_files.enumerate() {
        let epoll_event = epoll::Event::new(epoll::Events::EPOLLIN, file_idx as u64);
        epoll::ctl(
            epoll_fd,
            epoll::ControlOptions::EPOLL_CTL_ADD,
            file.as_raw_fd(),
            epoll_event,
        )?;
    }
    Ok(epoll_fd)
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub enum GrabStatus {
    /// ignore event
    Continue,
    /// ungrab events
    Stop,
}


/// returns tuple of epoll_fd, all devices, and uinput devices, where
/// uinputdevices is the same length as devices, and each uinput device is
/// a libevdev copy of its corresponding device.
fn setup_devices() -> io::Result<(RawFd, Vec<Device>, Vec<UInputDevice>)> {
    let device_files = get_device_files(DEV_PATH)?;
    let epoll_fd = epoll_watch_all(device_files.iter())?;
    let devices = device_files
        .into_iter()
        .map(|file| Device::new_from_fd(file))
        .collect::<io::Result<Vec<Device>>>()?;
    let output_devices = devices
        .iter()
        .map(|device| UInputDevice::create_from_device(device))
        .collect::<io::Result<Vec<UInputDevice>>>()?;
    Ok((epoll_fd, devices, output_devices))
}

pub fn filter_map_events_with_delay_noreturn<F>(mut func: F) -> io::Result<()>
where
    F: FnMut(InputEvent) -> (Instant, Option<InputEvent>),
{
    filter_map_events_with_delay(|input_event| {
        let (instant, output_event) = func(input_event);
        (instant, output_event, GrabStatus::Continue)
    })
}

// apply function to all InputEvents
// Function takes InputEvent as input and returns a Result of tuple of (time at
// which to simulate input, event to simulate). If the Result is Err, the event
// is discarded. Otherwise, the event is simulated.
pub fn filter_map_events_with_delay<F>(mut func: F) -> io::Result<()>
where
    F: FnMut(InputEvent) -> (Instant, Option<InputEvent>, GrabStatus),
{
    let (epoll_fd, mut devices, output_devices) = setup_devices()?;
    //grab devices
    let _grab = devices
        .iter_mut()
        .map(|device| device.grab(evdev_rs::GrabMode::Grab))
        .collect::<io::Result<()>>()?;
    // create buffer for epoll to fill
    let mut epoll_buffer = [epoll::Event::new(epoll::Events::empty(), 0); 4];
    let mut events_buffer =
        BTreeMap::<Instant, (Option<(Instant, InputEvent, usize)>, GrabStatus)>::new();
    'event_loop: loop {
        let process_later = events_buffer.split_off(&Instant::now());
        let process_now = events_buffer;
        events_buffer = process_later;
        for (opt, grab_status) in process_now.values() {
            opt.into_iter().for_each(|(recieved_inst, event, idx)| {
                println!(
                    "Actual time waited: {}",
                    recieved_inst.elapsed().as_millis()
                );
                let output_device = output_devices.get(*idx).unwrap();
                output_device.write_event(event).unwrap();
            });
            if grab_status == &GrabStatus::Stop {
                break 'event_loop;
            }
        }
        //wait until new events need to be processed or new events arrive
        let wait_ms: i32 = events_buffer
            .keys()
            .nth(0)
            .and_then(|sim_inst| {
                sim_inst
                    .checked_duration_since(Instant::now())
                    .unwrap_or_default()
                    .checked_add(Duration::from_millis(1))?
                    .as_millis()
                    .try_into()
                    .ok()
            })
            .unwrap_or(-1i32);
        println!("Waiting for {}ms for new events...", wait_ms);
        let num_events = epoll::wait(epoll_fd, wait_ms, &mut epoll_buffer)?;
        //add new events to queue
        for event in &epoll_buffer[0..num_events] {
            let device_idx = event.data as usize;
            let device = devices.get(device_idx).unwrap();
            while device.has_event_pending() {
                //TODO: deal with EV_SYN::SYN_DROPPED
                let (_, event) = device.next_event(evdev_rs::ReadFlag::NORMAL)?;
                let (instant, opt_event, grab_status) = func(event);
                let value = opt_event.map(|event| (Instant::now(), event, device_idx));
                if grab_status == GrabStatus::Stop {
                    println!("Stopping Soon...");
                }
                events_buffer.insert(instant, (value, grab_status));
            }
        }
    }

    //cleanup
    let _ungrab = devices
        .iter_mut()
        .map(|device| device.grab(evdev_rs::GrabMode::Ungrab))
        .collect::<io::Result<()>>()?;

    epoll::close(epoll_fd)?;
    Ok(())
}

pub fn filter_map_events_noreturn<F>(func: F) -> io::Result<()>
where
    F: Fn(InputEvent) -> Option<InputEvent>,
{
    filter_map_events(|input_event| {
        let output_event = func(input_event);
        (output_event, GrabStatus::Continue)
    })
}

pub fn filter_map_events<F>(func: F) -> io::Result<()>
where
    F: Fn(InputEvent) -> (Option<InputEvent>, GrabStatus),
{
    let (epoll_fd, mut devices, output_devices) = setup_devices()?;
    //grab devices
    let _grab = devices
        .iter_mut()
        .map(|device| device.grab(evdev_rs::GrabMode::Grab))
        .collect::<io::Result<()>>()?;
    // create buffer for epoll to fill
    let mut epoll_buffer = [epoll::Event::new(epoll::Events::empty(), 0); 4];
    'event_loop: loop {
        let num_events = epoll::wait(epoll_fd, -1, &mut epoll_buffer)?;
        //map and simulate events
        for event in &epoll_buffer[0..num_events] {
            let device_idx = event.data as usize;
            let device = devices.get(device_idx).unwrap();
            while device.has_event_pending() {
                //TODO: deal with EV_SYN::SYN_DROPPED
                let (_, event) = device.next_event(evdev_rs::ReadFlag::NORMAL)?;
                let (event, grab_status) = func(event);
                for event in event.into_iter() {
                    output_devices
                        .get(device_idx)
                        .unwrap()
                        .write_event(&event)?
                }
                if grab_status == GrabStatus::Stop {
                    break 'event_loop;
                }
            }
        }
    }

    let _ungrab = devices
        .iter_mut()
        .map(|device| device.grab(evdev_rs::GrabMode::Ungrab))
        .collect::<io::Result<()>>()?;

    epoll::close(epoll_fd)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let ms_delay: u32 = args.get(1).map(|arg| arg.parse().unwrap()).unwrap_or(60);
    println!("Delay set to {}ms", ms_delay);

    //TODO: remove this hack
    //wait for enter key to be released after starting
    sleep(Duration::from_millis(500));

    // println!("Waiting for enter to be released");
    // filter_map_events(|event| {
    //     match event {
    //         evdev_rs::InputEvent{time:_,
    //             event_type:evdev_rs::EventType::EV_KEY,
    //             event_code:evdev_rs::EventCode::EV_KEY(evdev_rs::EV_KEY::KEY_ENTER),
    //             value:1
    //         } => {
    //             (Some(event), GrabStatus::Stop)
    //         }
    //         _ => (Some(event), GrabStatus::Continue)
    //     }
    // })?;
    // println!("Enter has been released");

    let mut event_count = 0;
    filter_map_events_with_delay(move |event| {
        event_count += 1;
        if event_count == 10000 {
            return (Instant::now(), None, GrabStatus::Stop);
        }
        let sim_inst = Instant::now() + Duration::from_millis(ms_delay.into());
        (sim_inst, Some(event), GrabStatus::Continue)
    })?;
    Ok(())
}
