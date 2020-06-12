use epoll::ControlOptions::{EPOLL_CTL_ADD, EPOLL_CTL_DEL};
use evdev_rs::{Device, InputEvent, UInputDevice};
use inotify::{Inotify, WatchMask};
use std::collections::BTreeMap;
use std::convert::{TryFrom, TryInto};
use std::ffi::{OsStr, OsString};
use std::fs::{read_dir, File};
use std::io;
use std::os::unix::{
    ffi::OsStrExt,
    fs::FileTypeExt,
    io::{AsRawFd, IntoRawFd, RawFd},
};
use std::path::Path;
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
            || file_name_bytes
                .get(0..=1)
                .map(|s| s == OsStr::new("js").as_bytes())
                .unwrap_or(false)
            || file_name_bytes
                .get(0..=4)
                .map(|s| s == OsStr::new("mouse").as_bytes())
                .unwrap_or(false)
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
        epoll::ctl(epoll_fd, EPOLL_CTL_ADD, file.as_raw_fd(), epoll_event)?;
    }
    Ok(epoll_fd)
}

fn inotify_devices() -> io::Result<Inotify> {
    let mut inotify = Inotify::init()?;
    inotify.add_watch(DEV_PATH, WatchMask::CREATE)?;
    Ok(inotify)
}

pub fn add_device_to_epoll_from_inotify_event(
    epoll_fd: RawFd,
    event: inotify::Event<&OsStr>,
    devices: &mut Vec<Device>,
) -> io::Result<()> {
    let mut device_path = OsString::from(DEV_PATH);
    device_path.push(OsString::from("/"));
    device_path.push(event.name.unwrap());
    // new plug events
    let file = File::open(device_path)?;
    let fd = file.as_raw_fd();
    let device = Device::new_from_fd(file)?;
    devices.push(device);
    let event = epoll::Event::new(epoll::Events::EPOLLIN, devices.len() as u64 - 1);
    epoll::ctl(epoll_fd, EPOLL_CTL_ADD, fd, event)?;
    Ok(())
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
/// a libevdev copy of its corresponding device.The epoll_fd is level-triggered
/// on any available data in the original devices.
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

/// Similar to Iterator's filter_map
///
/// Filter and transform events, additionally specifying an Instant at which
/// to simulate the transformed event. To stop grabbing, return
/// `GrabStatus::Stop`
pub fn filter_map_events_with_delay<F>(mut func: F) -> io::Result<()>
where
    F: FnMut(InputEvent) -> (Instant, Option<InputEvent>, GrabStatus),
{
    let (epoll_fd, mut devices, output_devices) = setup_devices()?;

    //Ensure there is space for inotify at last epoll index.
    if u64::try_from(devices.len()).unwrap_or(u64::max_value()) >= u64::max_value() {
        eprintln!("number of devices: {}", devices.len());
        Err(io::Error::new(
            io::ErrorKind::Other,
            "too many device files!",
        ))?
    }
    // Set up inotify to listen for new devices being plugged in
    let mut inotify = inotify_devices()?;
    let epoll_event = epoll::Event::new(epoll::Events::EPOLLIN, u64::max_value());
    epoll::ctl(epoll_fd, EPOLL_CTL_ADD, inotify.as_raw_fd(), epoll_event)?;

    //grab devices
    let _grab = devices
        .iter_mut()
        .map(|device| device.grab(evdev_rs::GrabMode::Grab))
        .collect::<io::Result<()>>()?;
    // create buffer for epoll to fill
    let mut epoll_buffer = [epoll::Event::new(epoll::Events::empty(), 0); 4];
    let mut inotify_buffer = vec![0_u8; 4096];
    let mut events_buffer =
        BTreeMap::<Instant, (Option<(Instant, InputEvent, usize)>, GrabStatus)>::new();
    'event_loop: loop {
        let process_later = events_buffer.split_off(&Instant::now());
        let process_now = events_buffer;
        events_buffer = process_later;
        for (opt, grab_status) in process_now.values() {
            for (recieved_inst, event, idx) in opt {
                println!(
                    "Actual time waited: {}ms",
                    recieved_inst.elapsed().as_millis()
                );
                let output_device = match output_devices.get(*idx) {
                    Some(out_dev) => out_dev,
                    None => continue, // Device was unplugged
                };
                output_device.write_event(event)?;
            }
            if grab_status == &GrabStatus::Stop {
                break 'event_loop;
            }
        }
        // wait at longest until new events need to be processed
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
        'events: for event in epoll_buffer.get(0..num_events).unwrap() {
            //inotify - new device file created
            if event.data == u64::max_value() {
                for event in inotify.read_events(&mut inotify_buffer)? {
                    if event.mask.contains(inotify::EventMask::CREATE) {
                        add_device_to_epoll_from_inotify_event(epoll_fd, event, &mut devices)?;
                    } else {
                        unreachable!("inotify is listening for events other than file creation")
                    }
                }
            } else {
                // Input device recieved event
                let device_idx = event.data as usize;
                let device = devices.get(device_idx).unwrap();
                while device.has_event_pending() {
                    //TODO: deal with EV_SYN::SYN_DROPPED
                    let (_, event) = match device.next_event(evdev_rs::ReadFlag::NORMAL) {
                        Ok(event) => event,
                        Err(_) => {
                            let device_file = device.fd().unwrap();
                            epoll::ctl(
                                epoll_fd,
                                EPOLL_CTL_DEL,
                                device_file.into_raw_fd(),
                                epoll::Event::new(epoll::Events::empty(), 0),
                            )?;
                            continue 'events;
                        }
                    };
                    let (instant, opt_event, grab_status) = func(event);
                    let value = opt_event.map(|event| (Instant::now(), event, device_idx));
                    events_buffer.insert(instant, (value, grab_status));
                }
            }
        }
    }

    for device in devices.iter_mut() {
        //ungrab devices, ignore errors
        device.grab(evdev_rs::GrabMode::Ungrab).ok();
    }

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

pub fn filter_map_events<F>(mut func: F) -> io::Result<()>
where
    F: FnMut(InputEvent) -> (Option<InputEvent>, GrabStatus),
{
    let (epoll_fd, mut devices, output_devices) = setup_devices()?;
    //Ensure there is space for inotify at last epoll index.
    if u64::try_from(devices.len()).unwrap_or(u64::max_value()) >= u64::max_value() {
        eprintln!("number of devices: {}", devices.len());
        Err(io::Error::new(
            io::ErrorKind::Other,
            "too many device files!",
        ))?
    }
    // Set up inotify to listen for new devices being plugged in
    let mut inotify = inotify_devices()?;
    let epoll_event = epoll::Event::new(epoll::Events::EPOLLIN, u64::max_value());
    epoll::ctl(epoll_fd, EPOLL_CTL_ADD, inotify.as_raw_fd(), epoll_event)?;

    //grab devices
    let _grab = devices
        .iter_mut()
        .map(|device| device.grab(evdev_rs::GrabMode::Grab))
        .collect::<io::Result<()>>()?;
    // create buffer for epoll to fill
    let mut epoll_buffer = [epoll::Event::new(epoll::Events::empty(), 0); 4];
    let mut inotify_buffer = vec![0_u8; 4096];
    'event_loop: loop {
        let num_events = epoll::wait(epoll_fd, -1, &mut epoll_buffer)?;

        //map and simulate events, dealing with
        'events: for event in &epoll_buffer[0..num_events] {
            //inotify - new device file created
            if event.data == u64::max_value() {
                for event in inotify.read_events(&mut inotify_buffer)? {
                    if event.mask.contains(inotify::EventMask::CREATE) {
                        add_device_to_epoll_from_inotify_event(epoll_fd, event, &mut devices)?;
                    } else {
                        unreachable!("inotify is listening for events other than file creation")
                    }
                }
            } else {
                // Input device recieved event
                let device_idx = event.data as usize;
                let device = devices.get(device_idx).unwrap();
                while device.has_event_pending() {
                    //TODO: deal with EV_SYN::SYN_DROPPED
                    let (_, event) = match device.next_event(evdev_rs::ReadFlag::NORMAL) {
                        Ok(event) => event,
                        Err(_) => {
                            let device_file = device.fd().unwrap();
                            epoll::ctl(
                                epoll_fd,
                                EPOLL_CTL_DEL,
                                device_file.into_raw_fd(),
                                epoll::Event::new(epoll::Events::empty(), 0),
                            )?;
                            continue 'events;
                        }
                    };
                    let (event, grab_status) = func(event);
                    for event in event.into_iter() {
                        for out_device in output_devices.get(device_idx).into_iter() {
                            out_device.write_event(&event)?;
                        }
                    }
                    if grab_status == GrabStatus::Stop {
                        break 'event_loop;
                    }
                }
            }
        }
    }

    for device in devices.iter_mut() {
        //ungrab devices, ignore errors
        device.grab(evdev_rs::GrabMode::Ungrab).ok();
    }

    epoll::close(epoll_fd)?;
    Ok(())
}