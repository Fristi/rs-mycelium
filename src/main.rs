mod improv;
mod wifi;
mod kv;

use esp_idf_sys as _; // If using the `binstart` feature of `esp-idf-sys`, always keep this module imported
use log::*;
use improv::*;
use bluedroid::gatt_server::{Characteristic, GLOBAL_GATT_SERVER, Profile, Service};
use bluedroid::utilities::{AttributePermissions, BleUuid, CharacteristicProperties};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use embedded_svc::wifi::Wifi;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::{EspDefaultNvs, EspDefaultNvsPartition};
use esp_idf_sys::esp_restart;
use crate::wifi::{EspMyceliumWifi, MyceliumWifiError, MyceliumWifi, WifiSettings};
use heapless::String;
use crate::kv::{KvStore, NvsKvsStore};

fn main() -> ! {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_sys::link_patches();
    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    let sysloop = EspSystemEventLoop::take().unwrap();
    let nvs_partition = EspDefaultNvsPartition::take().unwrap();
    let nvs = EspDefaultNvs::new(nvs_partition, "mycelium", true).unwrap();

    let mut kv = NvsKvsStore::new(nvs);
    let wifi_connector = EspMyceliumWifi::new(sysloop);


    let wifi_settings: Option<WifiSettings> = kv.get("wifi_settings").unwrap();

    match wifi_settings {
        Some(s) => {
            let _ = wifi_connector.connect(&s.ssid, &s.password).unwrap();
        },
        None => {
            wifi_setup(wifi_connector, kv)
        }
    }

    loop {
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn wifi_setup(wifi_connector: EspMyceliumWifi, kv: NvsKvsStore) {
    let state = Arc::new(RwLock::new(ImprovState::Authorized));
    let state_read = state.clone();
    let error = Arc::new(RwLock::new(ImprovError::None));
    let error_read = error.clone();
    let improv_handler = Arc::new(Mutex::new(ImprovHandler::new(wifi_connector, kv, error, state)));

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
            improv_handler.lock().unwrap().handle(&bytes);

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
}



struct ImprovHandler<W, N> {
    wifi: W,
    settings: N,
    error: Arc<RwLock<ImprovError>>,
    state: Arc<RwLock<ImprovState>>
}

enum ImprovCommandResult<R> {
    Connected(R)
}

impl <W, N> ImprovHandler<W, N> {

    fn new<R>(esp_wifi: W, settings: N, error: Arc<RwLock<ImprovError>>, state: Arc<RwLock<ImprovState>>) -> ImprovHandler<W, N> where W : MyceliumWifi<R>, N : KvStore {
        ImprovHandler { wifi: esp_wifi, settings, error, state }
    }

    fn handle<R>(&mut self, bytes: &Vec<u8>) where W : MyceliumWifi<R>, N : KvStore {
        if let Some(cmd) = ImprovCommand::from_bytes(&bytes.as_slice()).ok() {
            match cmd {
                ImprovCommand::WifiSettings { ssid, password } => {
                    *self.error.write().unwrap() = ImprovError::None;
                    *self.state.write().unwrap() = ImprovState::Provisioning;

                    match self.wifi.connect(&ssid, &password) {
                        Ok(_) => {
                            *self.state.write().unwrap() = ImprovState::Provisioned;
                            self.settings.set("wifi_settings", WifiSettings { ssid, password}).unwrap();
                            unsafe { esp_restart() }
                        }
                        Err(wifi_err) => {
                            let err = match wifi_err {
                                MyceliumWifiError::Esp(err) => ImprovError::Unknown,
                                MyceliumWifiError::Timeout => ImprovError::UnableToConnect,
                                MyceliumWifiError::NoIp => ImprovError::NotAuthorized,
                            };
                            *self.error.write().unwrap() = err;
                            *self.state.write().unwrap() = ImprovState::Authorized;
                        }
                    };
                },
                _ => {
                    *self.error.write().unwrap() = ImprovError::InvalidRpc;
                }
            }
        } else {
            *self.error.write().unwrap() = ImprovError::UnknownRpc;
        }
    }
}
