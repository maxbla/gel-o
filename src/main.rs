use evdev_rs::{Device, InputEvent, UInputDevice};
use std::collections::VecDeque;
use std::convert::TryInto;
use std::error::Error;
use std::ffi::OsStr;
use std::fs::{read_dir, File};
use std::io;
use std::os::unix::{ffi::OsStrExt, fs::FileTypeExt, io::AsRawFd, io::RawFd};
use std::thread::sleep;
use std::time::{Duration, Instant};

fn get_all_device_files() -> io::Result<Vec<File>> {
    let mut res = Vec::new();
    for entry in read_dir("/dev/input")? {
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

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let ms_delay: u32 = args.get(1).map(|arg| arg.parse().unwrap()).unwrap_or(60);
    println!("Delay set to {}ms", ms_delay);

    //TODO: remove this hack
    //wait for enter key to be released after starting
    sleep(Duration::from_millis(500));

    println!("opening /dev/input files...");
    let device_files = get_all_device_files()?;
    let epoll_fd = epoll_watch_all(device_files.iter())?;

    let mut devices = device_files
        .into_iter()
        .map(|file| Device::new_from_fd(file))
        .collect::<io::Result<Vec<Device>>>()?;

    let output_devices = devices
        .iter()
        .map(|device| UInputDevice::create_from_device(device))
        .collect::<io::Result<Vec<UInputDevice>>>()?;

    //grab devices
    let _grab = devices
        .iter_mut()
        .map(|device| device.grab(evdev_rs::GrabMode::Grab))
        .collect::<io::Result<()>>()?;

    let mut events_buffer =
        VecDeque::<(Instant, usize, InputEvent)>::with_capacity(ms_delay.try_into()?);
    // create buffer for epoll to fill
    let mut epoll_buffer = [epoll::Event::new(epoll::Events::empty(), 0); 4];
    loop {
        //process queued events...
        while events_buffer
            .front()
            .map(|(in_time, _, _)| in_time.elapsed() > Duration::from_millis(ms_delay.into()))
            .unwrap_or(false)
        {
            let (in_time, idx, event) = events_buffer.pop_front().unwrap();
            let output_device = output_devices.get(idx).unwrap();
            let time_waited = in_time.elapsed();
            println!(
                "Actual amount of time waited: {}.{} ms",
                time_waited.as_millis(),
                time_waited.as_micros()
            );
            output_device.write_event(&event).unwrap();
        }
        //wait until new events need to be processed or new events arrive
        let wait_ms = events_buffer
            .front()
            .and_then(|(in_time, _, _)| {
                Duration::from_millis(u64::from(ms_delay))
                    .checked_add(Duration::from_micros(250))?
                    .checked_sub(in_time.elapsed())?
                    .as_millis()
                    .try_into()
                    .ok()
            })
            .unwrap_or(-1i32);
        println!("waiting... {:?}ms", wait_ms);
        let num_events = epoll::wait(epoll_fd, wait_ms, &mut epoll_buffer)?;
        //add new events to queue
        for event in &epoll_buffer[0..num_events] {
            let device_idx = event.data as usize;
            let device = devices.get(device_idx).unwrap();
            while device.has_event_pending() {
                //TODO: deal with EV_SYN::SYN_DROPPED
                let (_, event) = device.next_event(evdev_rs::ReadFlag::NORMAL)?;
                events_buffer.push_back((Instant::now(), device_idx, event));
            }
        }
    }
}
