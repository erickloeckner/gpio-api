use std::{env, process};
use std::fs;
use std::sync::{Arc, Mutex};

use gpio_cdev::{Chip, LineHandle, LineRequestFlags};
use gpio_cdev::errors::Error;
use serde::Deserialize;
use warp::Filter;

#[derive(Deserialize)]
struct Config {
    main: Main,
    gpio: Gpio,
}

#[derive(Deserialize)]
struct Main {
    debug: bool,
    port: u16,
}

#[derive(Clone, Deserialize)]
struct Gpio {
    chip: String,
    pins: Vec<u32>,
    names: Vec<String>,
}

#[derive(Deserialize)]
struct FormData {
    pin: u32,
    state: u8,
}

#[derive(Deserialize)]
struct FormDataName {
    name: String,
    state: u8,
}

struct HandlePair {
    handle: LineHandle,
    name: String,
}

fn get_handle_out(chip: &mut Chip, pin: u32) -> Result<LineHandle, Error> {
    let handle = chip
        .get_line(pin)?
        .request(LineRequestFlags::OUTPUT, 0, "gpio-api")?;
    Ok(handle)
}

fn get_handle_value(handle: &LineHandle) -> &'static str {
    match handle.get_value() {
        Ok(v) => {
            match v {
                0 => "0",
                1 => "1",
                _ => "err",
            }
        }
        Err(_) => "err",
    }
}

fn set_handle_value(handle: &LineHandle, value: u8, debug: bool) {
    match handle.set_value(value) {
        Ok(()) => {
            if debug { println!("pin {} set to {}", handle.line().offset(), value) }
        }
        Err(err) => {
            if debug { println!("error: {}", err) }
        }
    }
}

#[tokio::main]
async fn main() {
    let config_path = env::args().nth(1).unwrap_or_else(|| {
        println!("no config file specified");
        process::exit(1);
    });
    let config_raw = fs::read_to_string(&config_path).unwrap_or_else(|err| {
        println!("error reading config: {}", err);
        process::exit(1);
    });
    let config: Config = toml::from_str(&config_raw).unwrap_or_else(|err| {
        println!("error parsing config: {}", err);
        process::exit(1);
    });

    let mut chip = Chip::new(&config.gpio.chip).unwrap_or_else(|err| {
        println!("error opening GPIO chip: {}", err);
        process::exit(1);
    });

    let handles: Arc<Mutex<Vec<HandlePair>>> = Arc::new(Mutex::new(Vec::new()));
    for (pin, name) in config.gpio.pins.iter().zip(config.gpio.names) {
        let pin = get_handle_out(&mut chip, *pin).unwrap_or_else(|err| {
            println!("error opening GPIO pin {}: {}", pin, err);
            process::exit(1);
        });
        handles.lock().unwrap().push(HandlePair {handle: pin, name: name.clone()});
    }
    let handles_filter = warp::any().map(move || handles.clone());

    let debug = warp::any().map(move || config.main.debug.clone());

    let get = warp::path("get")
        .and(warp::path::param::<usize>())
        .and(handles_filter.clone())
        .map(|id: usize, pins: Arc<Mutex<Vec<HandlePair>>>| {
            if let Some(pin) = pins.lock().unwrap().get(id) {
                get_handle_value(&pin.handle)
            } else {
                "invalid GPIO"
            }
        });

    let set = warp::post()
        .and(warp::path("set"))
        .and(warp::body::content_length_limit(1024 * 16))
        .and(warp::body::form())
        .and(handles_filter.clone())
        .and(debug.clone())
        .map(|form: FormData, pins: Arc<Mutex<Vec<HandlePair>>>, debug: bool| {
            if let Some(pin) = pins.lock().unwrap().get(form.pin as usize) {
                set_handle_value(&pin.handle, form.state, debug);
                "OK"
            } else {
                "invalid GPIO pin"
            }
        });

    let name_get = warp::path("name")
        .and(warp::path("get"))
        .and(warp::path::param::<String>())
        .and(handles_filter.clone())
        .map(|name: String, pins: Arc<Mutex<Vec<HandlePair>>>| {
            let mut value = None;
            for pin in pins.lock().unwrap().iter() {
                if pin.name == name {
                    value = Some(get_handle_value(&pin.handle));
                }
            }
            if value.is_some() {
                value.unwrap()
            } else {
                "invalid GPIO name"
            }
        });

    let name_set = warp::post()
        .and(warp::path("name"))
        .and(warp::path("set"))
        .and(warp::body::content_length_limit(1024 * 16))
        .and(warp::body::form())
        .and(handles_filter.clone())
        .and(debug.clone())
        .map(|form: FormDataName, pins: Arc<Mutex<Vec<HandlePair>>>, debug: bool| {
            let mut name_match = false;
            for pin in pins.lock().unwrap().iter() {
                if pin.name == form.name {
                    name_match = true;
                    set_handle_value(&pin.handle, form.state, debug);
                }
            }
            if name_match {
                "OK"
            } else {
                "invalid GPIO name"
            }
        });

    let gpio = warp::path("gpio")
        .and(handles_filter.clone())
        .map(|pins: Arc<Mutex<Vec<HandlePair>>>| {
            let mut out = String::new();
            for pin in pins.lock().unwrap().iter() {
                out.push_str(&format!(
                    "pin: {} | name: {} | state: {}\n",
                    pin.handle.line().offset(),
                    pin.name,
                    get_handle_value(&pin.handle),
                ));
            }
            out
        });

    let routes = get
        .or(set)
        .or(name_get)
        .or(name_set)
        .or(gpio);

    warp::serve(routes)
        .run(([0, 0, 0, 0], config.main.port))
        .await;
}
