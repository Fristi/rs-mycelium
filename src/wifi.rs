// based on https://github.com/ferrous-systems/espressif-trainings/blob/1ec7fd78660c58739019b4c146634077a08e3d5e/common/lib/esp32-c3-dkc02-bsc/src/wifi.rs
// based on https://github.com/ivmarkov/rust-esp32-std-demo/blob/main/src/main.rs

use std::net::Ipv4Addr;
use std::time::Duration;
use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration, Wifi};
use esp_idf_hal::peripheral;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::netif::{EspNetif, EspNetifWait};
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, EspNvsPartition, NvsDefault};
use esp_idf_svc::wifi::{EspWifi, WifiWait};
use heapless::String;
use log::info;
use esp_idf_sys::EspError;
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub enum MyceliumWifiError {
    Esp(EspError),
    Timeout,
    NoIp
}

#[derive(Serialize, Deserialize)]
pub struct WifiSettings {
    pub ssid: String<32>,
    pub password: String<64>
}

pub trait MyceliumWifi<R> {
    fn connect(&self, ssid: &String<32>, psk: &String<64>) -> Result<R, MyceliumWifiError>;
}

pub struct EspMyceliumWifi {
    sysloop: EspSystemEventLoop
}

impl EspMyceliumWifi {
    pub fn new(sysloop: EspSystemEventLoop) -> EspMyceliumWifi {
        EspMyceliumWifi { sysloop }
    }
}

impl MyceliumWifi<EspWifi<'static>> for EspMyceliumWifi {
    fn connect(&self, ssid: &String<32>, psk: &String<64>) -> Result<EspWifi<'static>, MyceliumWifiError> {

        let mut auth_method = AuthMethod::WPA2Personal;
        if psk.is_empty() {
            auth_method = AuthMethod::None;
        }

        let modem = unsafe { esp_idf_hal::modem::WifiModem::new() };
        let mut wifi = EspWifi::new(modem, self.sysloop.clone(), None)?;
        info!("Searching for WiFi network {}", ssid);

        let ap_infos = wifi.scan()?;
        let ours = ap_infos.into_iter().find(|a| a.ssid.eq(ssid));
        let channel = if let Some(ours) = ours {
            info!(
            "Found configured access point {} on channel {}",
            ssid, ours.channel
        );
            Some(ours.channel)
        } else {
            info!(
            "Configured access point {} not found during scanning, will go with unknown channel",
            ssid
        );
            None
        };

        info!("Setting WiFi configuration");
        let conf = Configuration::Client(ClientConfiguration {
            ssid: ssid.clone(),
            password: psk.clone(),
            channel,
            auth_method,
            ..Default::default()
        });

        wifi.set_configuration(&conf)?;

        info!("Getting WiFi status");
        wifi.start()?;

        let wait = WifiWait::new(&self.sysloop)?;

        if !wait.wait_with_timeout(Duration::from_secs(20), || wifi.is_started().unwrap()) {
            return Err(MyceliumWifiError::Timeout)
        }

        info!("Started");

        wifi.connect()?;

        if !EspNetifWait::new::<EspNetif>(wifi.sta_netif(), &self.sysloop)?.wait_with_timeout(
                Duration::from_secs(10),
                || {
                    let connected= wifi.is_connected().unwrap();
                    let ip_info =  wifi.sta_netif().get_ip_info().unwrap();

                    info!("Status: {:?} {:?}", connected, ip_info);

                    connected && ip_info.ip != Ipv4Addr::new(0, 0, 0, 0)
                },
            ) {
            return Err(MyceliumWifiError::NoIp)
        }

        Ok(wifi)
    }
}

impl From<EspError> for MyceliumWifiError {
    fn from(value: EspError) -> Self {
        MyceliumWifiError::Esp(value)
    }
}