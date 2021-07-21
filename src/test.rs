#![cfg(test)]

use std::thread;
use std::time::{Duration, Instant};

use crate::{open_file_nonblock, EventsListener};
use evdev_rs::{
    enums::{BusType, EventCode, EventType, InputProp, EV_KEY, EV_MSC, EV_REL, EV_SYN},
    Device, DeviceWrapper, GrabMode, InputEvent, ReadFlag, TimeVal, UInputDevice, UninitDevice,
};
use serial_test::serial;

const SYN_REPORT_EVENT: InputEvent = InputEvent {
    time: TimeVal {
        tv_sec: 0,
        tv_usec: 0,
    },
    event_code: EventCode::EV_SYN(EV_SYN::SYN_REPORT),
    value: 0,
};

/// These tests generally don't assume the host system has any devices present
/// These tests use device names to disambiguate devices, so generally every
/// test needs a unique device name.

// Silly convience function that should probably be in evdev_rs
fn event_code_to_event_type(event_code: &EventCode) -> EventType {
    InputEvent::new(&TimeVal::new(0, 0), event_code, 0)
        .event_type()
        .unwrap()
}

// Create an UninitDevice with the passed EventCodes enabled
// Does not support EV_ABS or EV_REP
fn setup_device<'a, T: Iterator<Item = &'a EventCode>>(codes: T) -> UninitDevice {
    let dev = UninitDevice::new().unwrap();
    dev.set_bustype(BusType::BUS_USB as u16);
    for event_code in codes {
        let event_type = event_code_to_event_type(event_code);
        dev.enable_event_type(&event_type).unwrap();
        dev.enable_event_code(event_code, None).unwrap();
    }
    dev
}

fn setup_mouse(name: &str) -> UninitDevice {
    // Set up device like it is a mouse
    // REL_X, REL_Y, BTN_LEFT, BTN_RIGHT, BTN_MIDDLE, BTN_SIDE,
    // BTN_EXTRA, REL_WHEEL
    const MOUSE_CODES: [EventCode; 21] = [
        EventCode::EV_SYN(EV_SYN::SYN_REPORT),
        EventCode::EV_SYN(EV_SYN::SYN_CONFIG),
        EventCode::EV_SYN(EV_SYN::SYN_MT_REPORT),
        EventCode::EV_SYN(EV_SYN::SYN_DROPPED),
        EventCode::EV_SYN(EV_SYN::SYN_MAX),
        EventCode::EV_REL(EV_REL::REL_X),
        EventCode::EV_REL(EV_REL::REL_Y),
        EventCode::EV_REL(EV_REL::REL_WHEEL),
        EventCode::EV_REL(EV_REL::REL_HWHEEL),
        EventCode::EV_REL(EV_REL::REL_WHEEL_HI_RES),
        EventCode::EV_REL(EV_REL::REL_HWHEEL_HI_RES),
        EventCode::EV_KEY(EV_KEY::BTN_LEFT),
        EventCode::EV_KEY(EV_KEY::BTN_RIGHT),
        EventCode::EV_KEY(EV_KEY::BTN_MIDDLE),
        EventCode::EV_KEY(EV_KEY::BTN_MIDDLE),
        EventCode::EV_KEY(EV_KEY::BTN_SIDE),
        EventCode::EV_KEY(EV_KEY::BTN_EXTRA),
        EventCode::EV_KEY(EV_KEY::BTN_FORWARD),
        EventCode::EV_KEY(EV_KEY::BTN_BACK),
        EventCode::EV_KEY(EV_KEY::BTN_TASK),
        EventCode::EV_MSC(EV_MSC::MSC_SCAN),
    ];
    let dev = setup_device(MOUSE_CODES.iter());
    dev.set_name(name);
    dev.enable_property(&InputProp::INPUT_PROP_POINTER).unwrap();
    dev
}

// Enables all keys present on a tenkeyless (no numpad) keyboard
fn setup_kbd(name: &str) -> UninitDevice {
    const KEYBOARD_CODES: [EventCode; 89] = [
        EventCode::EV_MSC(EV_MSC::MSC_SCAN),
        EventCode::EV_SYN(EV_SYN::SYN_REPORT),
        EventCode::EV_KEY(EV_KEY::KEY_ESC),
        EventCode::EV_KEY(EV_KEY::KEY_F1),
        EventCode::EV_KEY(EV_KEY::KEY_F2),
        EventCode::EV_KEY(EV_KEY::KEY_F3),
        EventCode::EV_KEY(EV_KEY::KEY_F4),
        EventCode::EV_KEY(EV_KEY::KEY_F5),
        EventCode::EV_KEY(EV_KEY::KEY_F6),
        EventCode::EV_KEY(EV_KEY::KEY_F7),
        EventCode::EV_KEY(EV_KEY::KEY_F8),
        EventCode::EV_KEY(EV_KEY::KEY_F9),
        EventCode::EV_KEY(EV_KEY::KEY_F10),
        EventCode::EV_KEY(EV_KEY::KEY_F11),
        EventCode::EV_KEY(EV_KEY::KEY_F12),
        EventCode::EV_KEY(EV_KEY::KEY_SYSRQ),
        EventCode::EV_KEY(EV_KEY::KEY_SCROLLLOCK),
        EventCode::EV_KEY(EV_KEY::KEY_GRAVE),
        EventCode::EV_KEY(EV_KEY::KEY_1),
        EventCode::EV_KEY(EV_KEY::KEY_2),
        EventCode::EV_KEY(EV_KEY::KEY_3),
        EventCode::EV_KEY(EV_KEY::KEY_4),
        EventCode::EV_KEY(EV_KEY::KEY_5),
        EventCode::EV_KEY(EV_KEY::KEY_6),
        EventCode::EV_KEY(EV_KEY::KEY_7),
        EventCode::EV_KEY(EV_KEY::KEY_8),
        EventCode::EV_KEY(EV_KEY::KEY_9),
        EventCode::EV_KEY(EV_KEY::KEY_0),
        EventCode::EV_KEY(EV_KEY::KEY_MINUS),
        EventCode::EV_KEY(EV_KEY::KEY_EQUAL),
        EventCode::EV_KEY(EV_KEY::KEY_BACKSPACE),
        EventCode::EV_KEY(EV_KEY::KEY_INSERT),
        EventCode::EV_KEY(EV_KEY::KEY_HOME),
        EventCode::EV_KEY(EV_KEY::KEY_PAGEUP),
        EventCode::EV_KEY(EV_KEY::KEY_PAGEDOWN),
        EventCode::EV_KEY(EV_KEY::KEY_TAB),
        EventCode::EV_KEY(EV_KEY::KEY_Q),
        EventCode::EV_KEY(EV_KEY::KEY_W),
        EventCode::EV_KEY(EV_KEY::KEY_E),
        EventCode::EV_KEY(EV_KEY::KEY_R),
        EventCode::EV_KEY(EV_KEY::KEY_T),
        EventCode::EV_KEY(EV_KEY::KEY_Y),
        EventCode::EV_KEY(EV_KEY::KEY_U),
        EventCode::EV_KEY(EV_KEY::KEY_I),
        EventCode::EV_KEY(EV_KEY::KEY_O),
        EventCode::EV_KEY(EV_KEY::KEY_P),
        EventCode::EV_KEY(EV_KEY::KEY_LEFTBRACE),
        EventCode::EV_KEY(EV_KEY::KEY_RIGHTBRACE),
        EventCode::EV_KEY(EV_KEY::KEY_BACKSLASH),
        EventCode::EV_KEY(EV_KEY::KEY_DELETE),
        EventCode::EV_KEY(EV_KEY::KEY_END),
        EventCode::EV_KEY(EV_KEY::KEY_PAGEDOWN),
        EventCode::EV_KEY(EV_KEY::KEY_CAPSLOCK),
        EventCode::EV_KEY(EV_KEY::KEY_A),
        EventCode::EV_KEY(EV_KEY::KEY_S),
        EventCode::EV_KEY(EV_KEY::KEY_D),
        EventCode::EV_KEY(EV_KEY::KEY_F),
        EventCode::EV_KEY(EV_KEY::KEY_G),
        EventCode::EV_KEY(EV_KEY::KEY_H),
        EventCode::EV_KEY(EV_KEY::KEY_J),
        EventCode::EV_KEY(EV_KEY::KEY_K),
        EventCode::EV_KEY(EV_KEY::KEY_L),
        EventCode::EV_KEY(EV_KEY::KEY_SEMICOLON),
        EventCode::EV_KEY(EV_KEY::KEY_APOSTROPHE),
        EventCode::EV_KEY(EV_KEY::KEY_ENTER),
        EventCode::EV_KEY(EV_KEY::KEY_LEFTSHIFT),
        EventCode::EV_KEY(EV_KEY::KEY_Z),
        EventCode::EV_KEY(EV_KEY::KEY_X),
        EventCode::EV_KEY(EV_KEY::KEY_C),
        EventCode::EV_KEY(EV_KEY::KEY_V),
        EventCode::EV_KEY(EV_KEY::KEY_B),
        EventCode::EV_KEY(EV_KEY::KEY_N),
        EventCode::EV_KEY(EV_KEY::KEY_M),
        EventCode::EV_KEY(EV_KEY::KEY_COMMA),
        EventCode::EV_KEY(EV_KEY::KEY_DOT),
        EventCode::EV_KEY(EV_KEY::KEY_SLASH),
        EventCode::EV_KEY(EV_KEY::KEY_RIGHTSHIFT),
        EventCode::EV_KEY(EV_KEY::KEY_UP),
        EventCode::EV_KEY(EV_KEY::KEY_LEFTCTRL),
        EventCode::EV_KEY(EV_KEY::KEY_LEFTMETA),
        EventCode::EV_KEY(EV_KEY::KEY_LEFTALT),
        EventCode::EV_KEY(EV_KEY::KEY_SPACE),
        EventCode::EV_KEY(EV_KEY::KEY_RIGHTALT),
        EventCode::EV_KEY(EV_KEY::KEY_FN),
        EventCode::EV_KEY(EV_KEY::KEY_COMPOSE),
        EventCode::EV_KEY(EV_KEY::KEY_RIGHTCTRL),
        EventCode::EV_KEY(EV_KEY::KEY_LEFT),
        EventCode::EV_KEY(EV_KEY::KEY_DOWN),
        EventCode::EV_KEY(EV_KEY::KEY_RIGHT),
    ];
    let dev = setup_device(KEYBOARD_CODES.iter());
    dev.set_name(name);
    dev
}

// Tests that a fake keyboard can be created and that events can be sent to it
// and read from it
#[test]
#[serial]
fn keyboard() {
    const TEST_KEYBOARD_NAME: &str = "gelo test keyboard";
    let uinput = {
        let uninit = setup_kbd(TEST_KEYBOARD_NAME);
        UInputDevice::create_from_device(&uninit).unwrap()
    };

    //Listen only for uinput, the newly created device
    let mut listener =
        EventsListener::new_filtered(true, |dev| dev.name() == Some(TEST_KEYBOARD_NAME)).unwrap();

    //Simulate shift press, then unpress
    let code = EventCode::EV_KEY(EV_KEY::KEY_LEFTSHIFT);
    let press = InputEvent::new(&TimeVal::new(0, 0), &code, 1);
    uinput.write_event(&press).unwrap();
    uinput.write_event(&SYN_REPORT_EVENT).unwrap();

    let unpress = InputEvent::new(&TimeVal::new(0, 0), &code, 0);
    uinput.write_event(&unpress).unwrap();
    uinput.write_event(&SYN_REPORT_EVENT).unwrap();

    println!("Waiting for sent events to be recieved...");
    let events: Vec<_> = listener.iter().take(3).collect();
    assert_eq!(events[0].0.event_code, press.event_code);
    assert_eq!(events[0].0.value, press.value);
    assert_eq!(events[2].0.event_code, unpress.event_code);
    assert_eq!(events[2].0.value, unpress.value);
}

// Tests that a fake mouse can be created and that events can be sent to it
// and read from it
#[test]
#[serial]
fn mouse() {
    // Sending REL_X events doesn't move the mouse in my X11 stack unless the
    // device's name contains "mouse"
    const MOUSE_NAME: &str = "gelo test mouse";
    let uinput = {
        let uninit = setup_mouse(MOUSE_NAME);
        UInputDevice::create_from_device(&uninit).unwrap()
    };
    thread::sleep(Duration::from_millis(200));

    // Listen only for uinput, the newly created device
    let mut listener =
        EventsListener::new_filtered(true, |dev| dev.name() == Some(MOUSE_NAME)).unwrap();

    //Simulate an event that doesn't do anything just to listen for it
    let input = InputEvent::new(&TimeVal::new(0, 0), &EventCode::EV_REL(EV_REL::REL_X), 10);
    uinput.write_event(&input).unwrap();
    uinput.write_event(&SYN_REPORT_EVENT).unwrap();

    println!("Waiting for sent event to be recieved...");
    for (event, _) in listener.iter().take(1) {
        assert_eq!(event.event_code, input.event_code);
        assert_eq!(event.value, input.value);
    }
}

// Tests that writes to a grabbed udev input can be read from it
#[test]
#[serial]
fn test_write_to_grabbed() {
    const KEYBOARD_NAME: &str = "gelo test write to grab keyboard";
    let uinput = {
        let uninit = setup_kbd(KEYBOARD_NAME);
        UInputDevice::create_from_device(&uninit).unwrap()
    };
    let dev_file = open_file_nonblock(uinput.devnode().unwrap()).unwrap();
    let mut device = Device::new_from_file(dev_file).unwrap();
    device.grab(GrabMode::Grab).unwrap();
    let press = InputEvent::new(
        &TimeVal::new(0, 0),
        &EventCode::EV_KEY(EV_KEY::KEY_RIGHTSHIFT),
        1,
    );
    let unpress = InputEvent::new(
        &TimeVal::new(0, 0),
        &EventCode::EV_KEY(EV_KEY::KEY_RIGHTSHIFT),
        0,
    );

    uinput.write_event(&press).unwrap();
    uinput.write_event(&SYN_REPORT_EVENT).unwrap();
    uinput.write_event(&unpress).unwrap();
    uinput.write_event(&SYN_REPORT_EVENT).unwrap();

    thread::sleep(Duration::from_millis(5));
    let (_status, event) = device.next_event(ReadFlag::NORMAL).unwrap();
    assert_eq!(event.event_code, press.event_code);
}

#[test]
#[serial]
fn test_multiple_grab() {
    // Create two fake inputs in case we're on a server/VM with no input devices
    const MOUSE_NAME: &str = "gelo test multiple grab mouse";
    const KBD_NAME: &str = "gelo test multiple grab keyboard";
    let _uinput1 = {
        let uninit = setup_mouse(MOUSE_NAME);
        UInputDevice::create_from_device(&uninit).unwrap()
    };
    let _uinput2 = {
        let uninit = setup_kbd(KBD_NAME);
        UInputDevice::create_from_device(&uninit).unwrap()
    };

    let listener1 =
        EventsListener::new_filtered(true, |dev| dev.name() == Some(MOUSE_NAME)).unwrap();
    assert_eq!(listener1.devices().len(), 1);
    // Test that you can't grab the same device multiple times
    let listener2 = EventsListener::new_filtered(true, |dev| dev.name() == Some(MOUSE_NAME));
    assert!(listener2.is_err());
    assert!(listener2.unwrap_err().raw_os_error().unwrap() == libc::EBUSY);
    // Test that you can listen to all devices while one is grabbed
    let listener3 = EventsListener::new(false);
    assert!(listener3.is_ok());
    drop(listener3);
    // Test that you can grab two sets of devices so long as their intersection is empty
    let listener4 = EventsListener::new_filtered(true, |dev| dev.name() == Some(KBD_NAME)).unwrap();
    assert_eq!(listener4.devices().len(), 1);
}

macro_rules! assert_eq_with_error {
    ($max_error:expr, $left:expr, $right:expr) => {{
        assert!($left - $right < $max_error)
    }};
}

// This tests that the deprecated filter_map_events_with_delay function
// properly delays events.
#[test]
#[serial]
#[allow(deprecated)]
fn test_filter_map_events_with_delay() {
    use crate::GrabStatus;

    // create fake device (with known name)
    // set up delaying events on another thread
    // simulate event on fake device
    // event is delayed on other thread
    // listen for event on copy of fake device

    const KEYBOARD_NAME: &str = "gelo test delay keyboard";
    let uinput = {
        let uninit = setup_kbd(KEYBOARD_NAME);
        UInputDevice::create_from_device(&uninit).unwrap()
    };

    let ms_delay = 35_u32;

    let handle = {
        let delay_dur = Duration::from_millis((ms_delay).into());
        thread::spawn(move || {
            let mut event_count = 0;
            crate::deprecated::filter_map_events_with_delay(move |event| {
                event_count += 1;

                // delay at most ten events
                let ret = if event_count >= 40 {
                    (Instant::now(), None, GrabStatus::Stop)
                } else {
                    let sim_inst = Instant::now() + delay_dur;
                    (sim_inst, Some(event.clone()), GrabStatus::Continue)
                };
                return ret;
            })
            .unwrap();
        })
    };

    thread::sleep(Duration::from_millis(10));

    // Listen only for mouse (the grabbed original and ungrabbed copy created
    // by filter_map_events_with_delay, since they are difficult to distinguish)
    let mut listener =
        EventsListener::new_filtered(false, |dev| dev.name() == Some(KEYBOARD_NAME)).unwrap();
    assert_eq!(listener.devices().len(), 2);

    let press = InputEvent::new(
        &TimeVal::new(0, 0),
        &EventCode::EV_KEY(EV_KEY::KEY_RIGHTSHIFT),
        2,
    );
    uinput.write_event(&press).unwrap();
    let send_instant = Instant::now();
    uinput.write_event(&SYN_REPORT_EVENT).unwrap();
    let unpress = InputEvent::new(
        &TimeVal::new(0, 0),
        &EventCode::EV_KEY(EV_KEY::KEY_RIGHTSHIFT),
        0,
    );
    uinput.write_event(&unpress).unwrap();
    uinput.write_event(&SYN_REPORT_EVENT).unwrap();

    let (recv_event, _) = listener.iter().take(1).next().unwrap();
    let recv_instant = Instant::now();
    assert_eq!(recv_event.event_code, press.event_code);
    assert_eq!(recv_event.value, press.value);

    let measured_delay = recv_instant - send_instant;
    assert_eq_with_error!(3, u128::from(ms_delay), measured_delay.as_millis());

    // make sure other thread terminates
    for _ in 0..20 {
        uinput.write_event(&press).unwrap();
        uinput.write_event(&SYN_REPORT_EVENT).unwrap();
        uinput.write_event(&unpress).unwrap();
        uinput.write_event(&SYN_REPORT_EVENT).unwrap();
    }

    handle.join().unwrap();
}
