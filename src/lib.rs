pub use evdev_rs;

use epoll::ControlOptions::{EPOLL_CTL_ADD, EPOLL_CTL_DEL};
use evdev_rs::{Device, InputEvent, ReadFlag, UInputDevice};
use inotify::{EventMask, Inotify, WatchMask};
use std::convert::{TryFrom, TryInto};
use std::ffi::{CString, OsStr, OsString};
use std::fs::{read_dir, File};
use std::io;
use std::os::unix::{
    ffi::OsStrExt,
    fs::FileTypeExt,
    io::{AsRawFd, FromRawFd, RawFd},
};
use std::path::Path;
#[cfg(feature = "arc")]
use std::sync::Mutex;
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    time::{Duration, Instant},
};

static DEV_PATH: &str = "/dev/input";
const INOTIFY_DATA: u64 = u64::max_value();
const EPOLLIN: epoll::Events = epoll::Events::EPOLLIN;

#[cfg(feature = "arc")]
pub type Ptr<T> = std::sync::Arc<Mutex<T>>;
#[cfg(not(feature = "arc"))]
pub type Ptr<T> = std::rc::Rc<T>;

/// path to a file and the file itself
/// generic so you can use file-backed types which are not `std::io::File`s,
/// for example evdev::Device
pub type PathFile<T> = (PathBuf, T);

pub type DeviceFilter = fn(&Device) -> bool;

/// A handle to all resources needed to listen for and simulate input events
///
/// Creating multiple `EventsListener`s is fraught, as the order of event
/// delivery to `EventsListener`s is unspecified
pub struct EventsListener {
    epoll_fd: RawFd,
    epoll_buffer: Option<[epoll::Event; 1]>,
    /// monotonically increasing index for inserting into hashmap
    idx: u64,
    /// corresponding device path, input and output devices
    devices: HashMap<u64, (PathFile<Device>, PathFile<Ptr<UInputDevice>>)>,
    grab: bool,
    inotify: Inotify,
    inotify_buffer: Vec<u8>,
    filter: DeviceFilter,
}

impl Drop for EventsListener {
    fn drop(&mut self) {
        if self.grab {
            for ((_path, device), (_, _)) in self.devices.values_mut() {
                //ungrab devices, ignore errors
                device.grab(evdev_rs::GrabMode::Ungrab).ok();
            }
        }
        epoll::close(self.epoll_fd).ok();
    }
}

impl EventsListener {
    /// Gets a new events listener.
    ///
    /// Grab determines whether the events are automatically delivered further
    /// up the stack. If grabbing, you must simulate any event you recieve
    /// gor the OS to see it.
    pub fn new(grab: bool) -> io::Result<EventsListener> {
        let filter = |_: &Device| true;
        Self::new_with_filter(grab, filter)
    }

    /// Like new, only allows a filter, so some devices are not grabbed.
    ///
    /// filter must return the same value when called multiple times on the
    /// same device
    ///
    /// This method is immensely usefuly for testing, as it allows capturing
    /// only mouse input (in the event of a bug, you can press ctrl+c or even
    /// switch to the linux virtual terminal with ctrl+alt+F6 then use kill/htop)
    pub fn new_with_filter(grab: bool, filter: DeviceFilter) -> io::Result<EventsListener> {
        let (epoll_fd, devices) = setup_devices_2(filter)?;
        let mut devices: HashMap<u64, _> = devices
            .into_iter()
            .map(|(idx, ((path, dev), (ui_path, ui_dev)))| {
                #[cfg(feature = "arc")]
                return (idx, ((path, dev), (ui_path, Ptr::new(Mutex::new(ui_dev)))));
                #[cfg(not(feature = "arc"))]
                (idx, ((path, dev), (ui_path, Ptr::new(ui_dev))))
            })
            .collect();
        let inotify = setup_inotify_2()?;

        if grab {
            for ((_, device), _) in devices.values_mut() {
                device.grab(evdev_rs::GrabMode::Grab)?
            }
        }
        let inotify_buffer = vec![0_u8; 4096];

        Ok(EventsListener {
            epoll_fd,
            epoll_buffer: None,
            idx: devices.len().try_into().unwrap(),
            devices,
            grab,
            inotify,
            inotify_buffer,
            filter,
        })
    }

    pub fn iter(&mut self) -> InputIter {
        InputIter {
            listener: self,
            last_device_idx: None,
            syncing: false,
        }
    }

    /// Simulate an event on a device
    ///
    /// Panics if you give it an index that does not correspond to a device
    /// Usual EV_SYN caveat applies - events not processed until EV_SYN/SYN_REPORT is sent
    /// Prefer iter()
    pub fn simulate(&self, event: InputEvent, device_idx: u64) -> io::Result<()> {
        let output_device = self
            .devices
            .get(&device_idx)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "index out of range"))?
            .1
            .1
            .clone();
        #[cfg(feature = "arc")]
        output_device
            .try_lock()
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Could not aquire lock"))?
            .write_event(&event)?;
        #[cfg(not(feature = "arc"))]
        output_device.write_event(&event)?;
        Ok(())
    }
}

/// An iterator over evdev InputEvents
pub struct InputIter<'a> {
    listener: &'a mut EventsListener,
    last_device_idx: Option<u64>,
    syncing: bool,
}

impl<'a> InputIter<'a> {
    /// Returns true if a new event is queued up (calling next() would not block)
    pub fn ready(&mut self) -> io::Result<bool> {
        match self.listener.epoll_buffer {
            Some(_) => Ok(true),
            None => {
                let mut epoll_buffer = [epoll::Event::new(epoll::Events::empty(), 0); 1];
                let num_events = epoll::wait(self.listener.epoll_fd, 0, &mut epoll_buffer)?;
                if num_events > 0 {
                    self.listener.epoll_buffer = Some(epoll_buffer);
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    /// gets next event from a device, taking self.syncing into account.
    /// non-blocking.
    /// Return value of Ok(None) indicates no event was available
    fn device_next(
        &mut self,
        device_idx: u64,
    ) -> io::Result<Option<(&Device, InputEvent, Ptr<UInputDevice>)>> {
        let listener = &mut self.listener;
        if let Some(((_path, device), (_ui_path, uinput_device))) =
            listener.devices.get(&device_idx)
        {
            if device.has_event_pending() {
                let (_read_status, event) = if self.syncing {
                    match device.next_event(ReadFlag::SYNC) {
                        Ok(event) => event,
                        Err(code) if code.raw_os_error() == Some(libc::EAGAIN) => {
                            self.syncing = false;
                            device.next_event(ReadFlag::NORMAL)?
                        }
                        Err(err) => return Err(err),
                    }
                } else {
                    device.next_event(ReadFlag::NORMAL)?
                };
                self.last_device_idx = Some(device_idx);
                return Ok(Some((device, event, uinput_device.clone())));
            }
        };
        Ok(None)
    }

    /// Blocks until either an Input event occurs or timeout time passes
    ///
    /// A timeout of None indicates infinite timeout.
    /// Panics if timeout is longer than `i32::MAX` milliseconds
    /// returning Ok(None) indicates the timeout expired
    ///
    pub fn next_timeout(
        &mut self,
        timeout: Option<Duration>,
    ) -> io::Result<Option<(InputEvent, Ptr<UInputDevice>)>> {
        // determine if previous device has more events
        // necessary because evdev batch reads events off the fd
        if let Some(last_idx) = self.last_device_idx {
            if let Ok(Some(event)) = self.device_next(last_idx) {
                return Ok(Some((event.1, event.2)));
            }
        };

        let end_instant = timeout.map(|timeout| Instant::now() + timeout);
        while end_instant
            .map(|end_instant| Instant::now() < end_instant)
            .unwrap_or(true)
        {
            // eagerly update map of devcie files
            // previously, this was done through epoll,
            // but that caused bugs, as if a device was un plugged, then plugged back it
            // it might have been ignored by device de-dupe logic
            for event in self
                .listener
                .inotify
                .read_events(&mut self.listener.inotify_buffer)?
            {
                if event.mask.contains(EventMask::CREATE | EventMask::DELETE) {
                    panic!("Both created and deleted. Logic does not properly handle this case");
                } else if event.mask.contains(EventMask::CREATE) {
                    let is_uidevice = self
                        .listener
                        .devices
                        .values()
                        .any(|(_, (ui_path, _))| ui_path.file_name() == event.name);
                    if !is_uidevice {
                        let maybe_device = get_device_from_inotify_event(event);
                        let (path, mut device) = match maybe_device {
                            None => continue, //file cannot be used to initalize a Device. Skip.
                            Some(device) => device?,
                        };
                        let filter: DeviceFilter = self.listener.filter;
                        if !filter(&device) {
                            continue;
                        }
                        if self.listener.grab {
                            device.grab(evdev_rs::GrabMode::Grab)?;
                        }
                        let out_device = UInputDevice::create_from_device(&device)?;
                        let ui_path = PathBuf::from(out_device.devnode().unwrap_or(""));
                        let fd = device.file().as_raw_fd();
                        let idx = &mut self.listener.idx;
                        let event = epoll::Event::new(EPOLLIN, *idx);
                        #[cfg(not(feature = "arc"))]
                        self.listener.devices.insert(
                            *idx,
                            ((path, device), (ui_path, Ptr::new(out_device)))
                        );
                        #[cfg(feature = "arc")]
                        self.listener.devices.insert(
                            *idx, ((path, device),
                            (ui_path, Ptr::new(Mutex::new(out_device))))
                        );
                        *idx += 1;
                        epoll::ctl(self.listener.epoll_fd, EPOLL_CTL_ADD, fd, event)?;
                    }
                } else if event.mask.contains(EventMask::DELETE) {
                    if let Some(((_, device), _)) = self
                        .listener
                        .devices
                        .values()
                        .find(|((path, _), ..)| path.file_name() == event.name)
                    {
                        epoll::ctl(
                            self.listener.epoll_fd,
                            EPOLL_CTL_DEL,
                            device.file().as_raw_fd(),
                            epoll::Event::new(epoll::Events::empty(), 0),
                        )?;
                    } else {
                        // simply ignore. It would be nice to run a sanity check here,
                        // bit that's not possible
                        // panic!("Device that doesn't exist was unplugged")
                    }
                    // remove all devices with same path as deleted file
                    self.listener
                        .devices
                        .retain(|_idx, ((path, _), ..)| path.file_name() != event.name);
                } else {
                    panic!("inotify is listening for events other than file creation");
                }
            }

            let event = match self.listener.epoll_buffer {
                Some(epoll_buffer) => {
                    let event = epoll_buffer[0];
                    self.listener.epoll_buffer = None;
                    event
                }
                None => {
                    let mut epoll_buffer = [epoll::Event::new(epoll::Events::empty(), 0); 1];
                    let timeout_millis = if let Some(end_instant) = end_instant {
                        let now = Instant::now();
                        if now < end_instant {
                            end_instant
                                .duration_since(now)
                                .as_millis()
                                .try_into()
                                .map_err(|_| {
                                    io::Error::new(io::ErrorKind::InvalidInput, "timeout too long")
                                })?
                        } else {
                            0
                        }
                    } else {
                        -1
                    };
                    let num_events =
                        epoll::wait(self.listener.epoll_fd, timeout_millis, &mut epoll_buffer)?;
                    match num_events {
                        0 => return Ok(None),
                        1 => epoll_buffer[0],
                        _ => panic!("epoll_wait didn't work as expected"),
                    }
                }
            };

            let device_idx = event.data;
            return self
                .device_next(device_idx)
                .map(|event| event.map(|inner| (inner.1, inner.2)));
        }
        Ok(None)
    }
}

impl<'a> std::iter::Iterator for InputIter<'a> {
    type Item = (InputEvent, Ptr<UInputDevice>);

    fn size_hint(&self) -> (usize, Option<usize>) {
        (std::usize::MAX, None)
    }

    /// Get the next event from all devices included in the DeviceListener
    /// Returning None indicates an io Error. To get the full io error, use next_timeout
    fn next(&mut self) -> Option<Self::Item> {
        self.next_timeout(None).ok().flatten()
    }
}

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

fn get_device_paths<T: AsRef<Path>>(path: T) -> io::Result<Vec<PathBuf>> {
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
        res.push(path);
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
        let epoll_event = epoll::Event::new(EPOLLIN, file_idx as u64);
        epoll::ctl(epoll_fd, EPOLL_CTL_ADD, file.as_raw_fd(), epoll_event)?;
    }
    Ok(epoll_fd)
}

fn add_device_to_epoll_from_inotify_event(
    epoll_fd: RawFd,
    event: inotify::Event<&OsStr>,
    devices: &mut Vec<Device>,
    grab: bool,
) -> io::Result<()> {
    let maybe_device = get_device_from_inotify_event(event);
    let (_path, mut device) = match maybe_device {
        None => return Ok(()), //do nothing
        Some(device) => device?,
    };
    if grab {
        device.grab(evdev_rs::GrabMode::Grab)?;
    }
    let fd = device.file().as_raw_fd();
    let event = epoll::Event::new(EPOLLIN, devices.len() as u64);
    devices.push(device);
    epoll::ctl(epoll_fd, EPOLL_CTL_ADD, fd, event)?;
    Ok(())
}

fn get_device_from_inotify_event(
    event: inotify::Event<&OsStr>,
) -> Option<io::Result<(PathBuf, Device)>> {
    if event.mask != EventMask::CREATE {
        return None;
    }
    // skip filenames matching "mouse.* or mice".
    // these files don't play nice with libevdev, not sure why
    // see: https://askubuntu.com/questions/1043832/difference-between-dev-input-mouse0-and-dev-input-mice
    let new_dev_filename = event.name?;
    if new_dev_filename == OsStr::new("mice")
        || new_dev_filename
            .as_bytes()
            .get(0..=1)
            .map(|s| s == OsStr::new("js").as_bytes())
            .unwrap_or(false)
        || new_dev_filename
            .as_bytes()
            .get(0..=4)
            .map(|s| s == OsStr::new("mouse").as_bytes())
            .unwrap_or(false)
    {
        return None;
    }

    let mut device_path = OsString::from(DEV_PATH);
    device_path.push(OsString::from("/"));
    device_path.push(new_dev_filename);
    let device_path: PathBuf = device_path.into();
    // new plug events
    let file = match open_file_nonblock(&device_path) {
        Ok(file) => file,
        Err(err) => return Some(Err(err)),
    };
    Some(Device::new_from_file(file).map(|device| (device_path, device)))
}

/// Returns tuple of epoll_fd, all devices, and uinput devices, where
/// uinputdevices is the same length as devices, and each uinput device is
/// a libevdev copy of its corresponding device.The epoll_fd is level-triggered
/// on any available data in the original devices.
fn setup_devices() -> io::Result<(RawFd, Vec<Device>, Vec<UInputDevice>)> {
    let device_files = get_device_paths(DEV_PATH)?
        .into_iter()
        .map(File::open)
        .collect::<io::Result<Vec<File>>>()?;
    let epoll_fd = epoll_watch_all(device_files.iter())?;
    let devices = device_files
        .into_iter()
        .map(Device::new_from_file)
        .collect::<io::Result<Vec<Device>>>()?;
    let output_devices = devices
        .iter()
        .map(|device| UInputDevice::create_from_device(device))
        .collect::<io::Result<Vec<UInputDevice>>>()?;
    Ok((epoll_fd, devices, output_devices))
}

/// Open an fd with O_RDONLY | O_NONBLOCK
fn open_file_nonblock<P: AsRef<OsStr>>(path: P) -> io::Result<File> {
    let path = CString::new(path.as_ref().as_bytes())
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    let fd = match unsafe { libc::open(path.into_raw(), libc::O_RDONLY | libc::O_NONBLOCK) } {
        -1 => return Err(io::Error::last_os_error()),
        res => res,
    };
    let mut buf = [0u8; 128];
    let mut res = 0;
    while res >= 0 {
        res = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, 128) };
    }
    assert!(io::Error::last_os_error().kind() == io::ErrorKind::WouldBlock);
    Ok(unsafe { File::from_raw_fd(fd) })
}

/// Returns tuple of epoll_fd and a hashmap containing a device and a
/// UInputDevice that any events read from the device can be simulated on.
/// The epoll_fd is level-triggered on the device file.
fn setup_devices_2(
    include: DeviceFilter,
) -> io::Result<(
    RawFd,
    HashMap<u64, (PathFile<Device>, PathFile<UInputDevice>)>,
)> {
    let device_paths = get_device_paths(DEV_PATH)?;
    let epoll_fd = epoll::create(true)?;
    let mut ret = HashMap::with_capacity(device_paths.len());
    for (idx, path) in device_paths.into_iter().enumerate() {
        let file = open_file_nonblock(path.clone())?;
        let device = Device::new_from_file(file)?;
        if !include(&device) {
            continue;
        }
        let idx = idx.try_into().unwrap();
        let epoll_event = epoll::Event::new(EPOLLIN, idx);
        epoll::ctl(
            epoll_fd,
            EPOLL_CTL_ADD,
            device.file().as_raw_fd(),
            epoll_event,
        )?;
        let output_device = UInputDevice::create_from_device(&device)?;
        let devnode = output_device.devnode().unwrap();
        let uidev_path = PathBuf::from(devnode);
        ret.insert(idx, ((path, device), (uidev_path, output_device)));
    }
    Ok((epoll_fd, ret))
}

/// Creates an inotify instance looking at /dev/input and adds it to an epoll instance.
/// Ensures devices isnt too long, which would make the epoll data ambigious.
fn setup_inotify(epoll_fd: RawFd, devices: &[Device]) -> io::Result<Inotify> {
    //Ensure there is space for inotify at last epoll index.
    if u64::try_from(devices.len()).unwrap_or(std::u64::MAX) == INOTIFY_DATA {
        eprintln!("number of devices: {}", devices.len());
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "too many device files!",
        ));
    }
    // Set up inotify to listen for new devices being plugged in
    let mut inotify = Inotify::init()?;
    inotify.add_watch(DEV_PATH, WatchMask::CREATE)?;
    let epoll_event = epoll::Event::new(EPOLLIN, INOTIFY_DATA);
    epoll::ctl(epoll_fd, EPOLL_CTL_ADD, inotify.as_raw_fd(), epoll_event)?;
    Ok(inotify)
}

/// Creates an inotify instance looking at /dev/input and adds it to an epoll instance.
fn setup_inotify_2() -> io::Result<Inotify> {
    // Set up inotify to listen for new devices being plugged in
    let mut inotify = Inotify::init()?;
    inotify.add_watch(DEV_PATH, WatchMask::CREATE | WatchMask::DELETE)?;
    Ok(inotify)
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
/// Similar to Iterator's filter_map, only this blocks waiting for input
///
/// Filter and transform events, additionally specifying an Instant at which
/// to simulate the transformed event. To stop grabbing, return
/// `GrabStatus::Stop`
pub fn filter_map_events_with_delay<F>(mut func: F) -> io::Result<()>
where
    F: FnMut(InputEvent) -> (Instant, Option<InputEvent>, GrabStatus),
{
    let (epoll_fd, mut devices, output_devices) = setup_devices()?;
    let mut inotify = setup_inotify(epoll_fd, &devices)?;

    //grab devices
    let _grab = devices
        .iter_mut()
        .try_for_each(|device| device.grab(evdev_rs::GrabMode::Grab));
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
            .next()
            .and_then(|sim_inst| {
                sim_inst
                    .checked_duration_since(Instant::now())
                    .unwrap_or_default()
                    .as_millis()
                    .checked_add(1)?
                    .try_into()
                    .ok()
            })
            .unwrap_or(-1i32);
        println!("Waiting for {}ms for new events...", wait_ms);
        let num_events = epoll::wait(epoll_fd, wait_ms, &mut epoll_buffer)?;
        //add new events to queue
        'events: for event in epoll_buffer.get(0..num_events).unwrap() {
            // new device file created
            if event.data == INOTIFY_DATA {
                for event in inotify.read_events(&mut inotify_buffer)? {
                    assert!(
                        event.mask.contains(inotify::EventMask::CREATE),
                        "inotify is listening for events other than file creation"
                    );
                    add_device_to_epoll_from_inotify_event(epoll_fd, event, &mut devices, true)?;
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
                            let device_fd = device.file().as_raw_fd();
                            let empty_event = epoll::Event::new(epoll::Events::empty(), 0);
                            epoll::ctl(epoll_fd, EPOLL_CTL_DEL, device_fd, empty_event)?;
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

// TODO: return the never type, `!` when it is stabalized
/// Like `filter_map_events`, only never returns
#[deprecated(since = "0.1.0", note = "Use EventListener.iter().filter_map instead")]
pub fn filter_map_events_noreturn<F>(func: F) -> io::Result<()>
where
    F: Fn(InputEvent) -> Option<InputEvent>,
{
    filter_map_events(|input_event| {
        let output_event = func(input_event);
        (output_event, GrabStatus::Continue)
    })
}

#[deprecated(
    since = "0.1.0",
    note = "Use EventListener.iter().take_while().filter_map instead"
)]
/// Similar to Iterator's filter_map, only this blocks waiting for input
///
/// Filter and transform events. To stop grabbing, return `GrabStatus::Stop`
pub fn filter_map_events<F>(mut func: F) -> io::Result<()>
where
    F: FnMut(InputEvent) -> (Option<InputEvent>, GrabStatus),
{
    let (epoll_fd, mut devices, output_devices) = setup_devices()?;
    let mut inotify = setup_inotify(epoll_fd, &devices)?;

    //grab devices
    let _grab = devices
        .iter_mut()
        .try_for_each(|device| device.grab(evdev_rs::GrabMode::Grab));
    // create buffer for epoll to fill
    let mut epoll_buffer = [epoll::Event::new(epoll::Events::empty(), 0); 4];
    let mut inotify_buffer = vec![0_u8; 4096];
    'event_loop: loop {
        let num_events = epoll::wait(epoll_fd, -1, &mut epoll_buffer)?;

        //map and simulate events, dealing with
        'events: for event in &epoll_buffer[0..num_events] {
            // new device file created
            if event.data == INOTIFY_DATA {
                for event in inotify.read_events(&mut inotify_buffer)? {
                    assert!(
                        event.mask.contains(inotify::EventMask::CREATE),
                        "inotify is listening for events other than file creation"
                    );
                    add_device_to_epoll_from_inotify_event(epoll_fd, event, &mut devices, true)?;
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
                            let device_fd = device.file().as_raw_fd();
                            let empty_event = epoll::Event::new(epoll::Events::empty(), 0);
                            epoll::ctl(epoll_fd, EPOLL_CTL_DEL, device_fd, empty_event)?;
                            continue 'events;
                        }
                    };
                    let (event, grab_status) = func(event);
                    if let (Some(event), Some(out_device)) = (event, output_devices.get(device_idx))
                    {
                        out_device.write_event(&event)?
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

#[deprecated(since = "0.1.0", note = "Use EventListener.iter().for_each instead")]
/// Similar to Iterator's for_each, only this blocks waiting for input
///
/// Calls a closure on each event. To stop grabbing, return `GrabStatus::Stop`
pub fn for_each_event<F>(mut func: F) -> io::Result<()>
where
    F: FnMut(InputEvent) -> GrabStatus,
{
    let (epoll_fd, mut devices, _output_devices) = setup_devices()?;
    let mut inotify = setup_inotify(epoll_fd, &devices)?;

    // create buffer for epoll to fill
    let mut epoll_buffer = [epoll::Event::new(epoll::Events::empty(), 0); 4];
    let mut inotify_buffer = vec![0_u8; 4096];
    'event_loop: loop {
        let num_events = epoll::wait(epoll_fd, -1, &mut epoll_buffer)?;

        //map and simulate events, dealing with
        'events: for event in &epoll_buffer[0..num_events] {
            // new device file created
            if event.data == INOTIFY_DATA {
                for event in inotify.read_events(&mut inotify_buffer)? {
                    assert!(
                        event.mask.contains(inotify::EventMask::CREATE),
                        "inotify is listening for events other than file creation"
                    );
                    add_device_to_epoll_from_inotify_event(epoll_fd, event, &mut devices, false)?;
                }
            } else {
                // Input device recieved event
                let device_idx = event.data as usize;
                let device = devices.get(device_idx).unwrap();
                while device.has_event_pending() {
                    let (_, event) = match device.next_event(evdev_rs::ReadFlag::NORMAL) {
                        Ok(event) => event,
                        Err(_) => {
                            let device_fd = device.file().as_raw_fd();
                            let empty_event = epoll::Event::new(epoll::Events::empty(), 0);
                            epoll::ctl(epoll_fd, EPOLL_CTL_DEL, device_fd, empty_event)?;
                            continue 'events;
                        }
                    };
                    let grab_status = func(event);
                    if grab_status == GrabStatus::Stop {
                        break 'event_loop;
                    }
                }
            }
        }
    }

    epoll::close(epoll_fd)?;
    Ok(())
}
