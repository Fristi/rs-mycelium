mod improv;
mod wifi;

use esp_idf_sys as _; // If using the `binstart` feature of `esp-idf-sys`, always keep this module imported
use log::*;
use improv::*;
use bluedroid::gatt_server::{Characteristic, GLOBAL_GATT_SERVER, Profile, Service};
use bluedroid::utilities::{AttributePermissions, BleUuid, CharacteristicProperties};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use crate::wifi::wifi;

fn main() -> ! {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_sys::link_patches();
    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    let state = Arc::new(RwLock::new(State::Authorized));
    let state_read = state.clone();
    let error = Arc::new(RwLock::new(Error::None));
    let error_read = error.clone();
    let sysloop = EspSystemEventLoop::take().unwrap();
    let peripherals = Peripherals::take().unwrap();

    let current_state = Characteristic::new(BleUuid::from_uuid128_string(IMPROV_STATUS_UUID))
        .name("Current state")
        .permissions(AttributePermissions::new().read())
        .properties(CharacteristicProperties::new().read().notify())
        .show_name()
        .on_read(move |_| {
            let s = state_read.read().unwrap();
            let ss = *s;
            vec![ss.into()]
        })
        .build();



    let error_state = Characteristic::new(BleUuid::from_uuid128_string(IMPROV_ERROR_UUID))
        .name("Error state")
        .permissions(AttributePermissions::new().read())
        .properties(CharacteristicProperties::new().read().notify())
        .show_name()
        .on_read(move |_| {
            let s = error_read.read().unwrap();
            let ss = *s;
            vec![ss.into()]
        })
        .build();

    let rpc_command = Characteristic::new(BleUuid::from_uuid128_string(IMPROV_RPC_COMMAND_UUID))
        .name("RPC command handler")
        .permissions(AttributePermissions::new().write())
        .properties(CharacteristicProperties::new().write())
        .on_write(move |bytes, _| {
            match ImprovCommand::from_bytes(bytes.as_slice()) {
                Ok(ImprovCommand::WifiSettings { ssid, password }) => {
                    info!("Got ssid and password: {} {}", ssid, password);
                    *state.write().unwrap() = State::Provisioning;
                    *error.write().unwrap() = Error::None;
                    match wifi(&ssid, &password, sysloop.clone()) {
                        Ok(_) => {
                            *state.write().unwrap() = State::Provisioned;
                            info!("Connected")
                        },
                        Err(err) => {
                            info!("Error {:?}", err);
                            *error.write().unwrap() = Error::UnableToConnect;
                            *state.write().unwrap() = State::Authorized;
                            return ()
                        }
                    }
                    return ()
                }
                Ok(cmd) => {
                    info!("Command not processed {:?}", cmd);
                    return ()
                }
                Err(err) => {
                    info!("Error {:?}", err);
                    return ()
                }
            }
        })
        .show_name()
        .build();


    let rpc_result = Characteristic::new(BleUuid::from_uuid128_string(IMPROV_RPC_RESULT_UUID))
        .name("RPC result")
        .permissions(AttributePermissions::new().read())
        .properties(CharacteristicProperties::new().read().notify())
        .show_name()
        .build();

    let capabilities = Characteristic::new(BleUuid::from_uuid128_string(IMPROV_CAPABILITIES_UUID))
        .name("Capabilities")
        .permissions(AttributePermissions::new().read())
        .properties(CharacteristicProperties::new().read())
        .show_name()
        .set_value([0x00])
        .build();


    let service = Service::new(BleUuid::from_uuid128_string(IMPROV_SERVICE_UUID))
        .name("Improv Service")
        .primary()
        .characteristic(&rpc_command)
        .characteristic(&rpc_result)
        .characteristic(&current_state)
        .characteristic(&error_state)
        .characteristic(&capabilities)
        .build();

    let profile = Profile::new(0x0001)
        .name("Default Profile")
        .service(&service)
        .build();

    GLOBAL_GATT_SERVER
        .lock()
        .unwrap()
        .profile(profile)
        .device_name("Improve onboarding")
        .appearance(bluedroid::utilities::Appearance::GenericComputer)
        .advertise_service(&service)
        .start();

    loop {
        std::thread::sleep(Duration::from_millis(100));
    }
}

